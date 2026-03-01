#!/usr/bin/env node
/**
 * Ships strategy performance stats to MongoDB.
 *
 * - Reads /app/.defi-flow/registry.json for running strategies
 * - For each live strategy, reads its state file for perf metrics
 * - Computes TVL, PnL, APR, APY, max drawdown
 * - Ships to strategy_stats collection
 * - Runs as a long-lived daemon: reactive via fs.watch + 5-min poll fallback
 *
 * Env: MONGODB_URI, MONGODB_DB (default: "agent-logs")
 */
import { MongoClient } from "mongodb";
import { readFile, watch } from "fs/promises";
import { join } from "path";

// ── Config ──────────────────────────────────────────────
const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "agent-logs";
const REGISTRY_DIR = process.env.DEFI_FLOW_REGISTRY_DIR || "/app/.defi-flow";
const REGISTRY_PATH = join(REGISTRY_DIR, "registry.json");
const POLL_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes fallback
const MIN_SHIP_INTERVAL_MS = 30_000; // Don't ship more than once per 30s per strategy

if (!MONGODB_URI) {
  console.log("[stats] MONGODB_URI not set — skipping stats shipment.");
  process.exit(0);
}

// ── Throttle map ────────────────────────────────────────
const lastShipped = new Map(); // strategy name → timestamp

// ── Read JSON file safely ───────────────────────────────
async function readJson(path) {
  try {
    const content = await readFile(path, "utf-8");
    return JSON.parse(content);
  } catch {
    return null;
  }
}

// ── Compute derived metrics ─────────────────────────────
function computeMetrics(name, entry, state) {
  // Prefer last_tvl (on-chain queried) over balances sum (engine routing state).
  const balances = state.balances || {};
  const balanceSum = Object.values(balances).reduce((a, b) => a + b, 0);
  const tvl = state.last_tvl > 0 ? state.last_tvl : balanceSum;
  const initialCapital = state.initial_capital || 0;
  const peakTvl = state.peak_tvl || tvl;
  const startedAt = entry.started_at ? new Date(entry.started_at) : null;

  // PnL
  const pnl = initialCapital > 0 ? tvl - initialCapital : 0;

  // Uptime in hours
  const uptimeMs = startedAt ? Date.now() - startedAt.getTime() : 0;
  const uptimeHours = Math.max(uptimeMs / (1000 * 3600), 1); // min 1h to avoid division by zero

  // APR = annualized return (simple)
  const hoursPerYear = 8766; // 365.25 * 24
  const apr = initialCapital > 0
    ? (pnl / initialCapital) * (hoursPerYear / uptimeHours)
    : 0;

  // APY = compound (e^apr - 1)
  const apy = Math.exp(apr) - 1;

  // Max drawdown from peak
  const maxDrawdown = peakTvl > 0 ? Math.max(0, 1 - tvl / peakTvl) : 0;

  return {
    strategy: name,
    timestamp: new Date(),
    tvl,
    pnl,
    apr,
    apy,
    max_drawdown: maxDrawdown,
    initial_capital: initialCapital,
    peak_tvl: peakTvl,
    uptime_hours: Math.round(uptimeHours * 10) / 10,
    balances,
    funding_pnl: state.cumulative_funding || 0,
    interest_pnl: state.cumulative_interest || 0,
    rewards_pnl: state.cumulative_rewards || 0,
    swap_costs: state.cumulative_costs || 0,
    last_tick: state.last_tick || 0,
    mode: entry.mode || "unknown",
    network: entry.network || "unknown",
    status: "running",
    model: process.env.MODEL_NAME || "unknown",
  };
}

// ── Ship stats for all strategies ───────────────────────
async function shipStats(db) {
  const registry = await readJson(REGISTRY_PATH);
  if (!registry?.daemons) return;

  const collection = db.collection("strategy_stats");
  let shipped = 0;

  for (const [name, entry] of Object.entries(registry.daemons)) {
    // Throttle per strategy
    const lastTime = lastShipped.get(name) || 0;
    if (Date.now() - lastTime < MIN_SHIP_INTERVAL_MS) continue;

    // Read state file
    const state = await readJson(entry.state_file);
    if (!state) {
      console.log(`[stats] SKIP ${name} — no state file at ${entry.state_file}`);
      continue;
    }

    const metrics = computeMetrics(name, entry, state);

    try {
      await collection.insertOne(metrics);
      lastShipped.set(name, Date.now());
      shipped++;

      const aprPct = (metrics.apr * 100).toFixed(2);
      const ddPct = (metrics.max_drawdown * 100).toFixed(2);
      console.log(
        `[stats] ${name}: TVL=$${metrics.tvl.toFixed(2)} PnL=$${metrics.pnl.toFixed(2)} APR=${aprPct}% DD=${ddPct}%`
      );
    } catch (err) {
      console.error(`[stats] FAIL ${name}: ${err.message}`);
    }
  }

  if (shipped > 0) {
    console.log(`[stats] Shipped ${shipped} strategy stats to MongoDB.`);
  }
}

// ── Main ────────────────────────────────────────────────
const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  await client.connect();
  await client.db("admin").command({ ping: 1 });
  console.log(`[stats] Connected to MongoDB (db: ${DB_NAME})`);

  const db = client.db(DB_NAME);
  const collection = db.collection("strategy_stats");

  // Create indexes (idempotent)
  await collection.createIndex({ strategy: 1, timestamp: 1 }).catch(() => {});
  await collection.createIndex({ strategy: 1 }).catch(() => {});

  // Initial ship
  await shipStats(db);

  // Watch for state file changes (reactive)
  try {
    const watcher = watch(REGISTRY_DIR, { recursive: true });
    console.log(`[stats] Watching ${REGISTRY_DIR} for changes...`);

    // Run watcher and poll in parallel
    const pollLoop = async () => {
      while (true) {
        await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
        await shipStats(db).catch((e) =>
          console.error(`[stats] Poll error: ${e.message}`)
        );
      }
    };

    const watchLoop = async () => {
      for await (const event of watcher) {
        // Only react to .json file changes (state files, registry)
        if (event.filename?.endsWith(".json")) {
          // Small debounce — state files are written atomically but we still
          // don't want to fire on every single tick
          await new Promise((r) => setTimeout(r, 2000));
          await shipStats(db).catch((e) =>
            console.error(`[stats] Watch error: ${e.message}`)
          );
        }
      }
    };

    // Both run forever
    await Promise.race([pollLoop(), watchLoop()]);
  } catch {
    // fs.watch not supported or registry dir missing — fall back to polling only
    console.log("[stats] fs.watch unavailable, polling only.");
    while (true) {
      await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
      await shipStats(db).catch((e) =>
        console.error(`[stats] Poll error: ${e.message}`)
      );
    }
  }
} catch (err) {
  console.error("[stats] Stats shipper failed:", err.message);
  process.exit(1);
}
