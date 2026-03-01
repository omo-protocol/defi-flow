#!/usr/bin/env node
/**
 * Ships quant agent strategy stats to MongoDB.
 *
 * For each agent (identified by DEFI_FLOW_PRIVATE_KEY):
 *   1. Agent wallet USDT0 balance on HyperEVM (idle funds)
 *   2. ETH on Base (gas tracking)
 *   3. Deployed strategies via `defi-flow query` (per-venue on-chain TVL)
 *      - Falls back to state file last_tvl if binary unavailable
 *   4. Computes PnL, APR, APY, max drawdown per strategy
 *
 * Strategy discovery: reads registry.json OR scans /app/strategies/*.json
 *
 * Runs as a long-lived daemon: reactive via fs.watch + 5-min poll fallback.
 *
 * Env: MONGODB_URI, MONGODB_DB, DEFI_FLOW_PRIVATE_KEY,
 *      DEFI_FLOW_REGISTRY_DIR (default: /app/.defi-flow)
 */
import { MongoClient } from "mongodb";
import { readFile, watch, readdir } from "fs/promises";
import { join } from "path";
import { execSync } from "child_process";

// ── Config ──────────────────────────────────────────────
const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "agent-logs";
const REGISTRY_DIR = process.env.DEFI_FLOW_REGISTRY_DIR || "/app/.defi-flow";
const REGISTRY_PATH = join(REGISTRY_DIR, "registry.json");
const STRATEGIES_DIR = process.env.STRATEGIES_DIR || "/app/strategies";
const POLL_INTERVAL_MS = 5 * 60 * 1000;
const MIN_SHIP_INTERVAL_MS = 30_000;

const INITIAL_CAPITAL = parseFloat(process.env.INITIAL_CAPITAL || "90");
const HYPEREVM_RPC = "https://rpc.hyperliquid.xyz/evm";
const BASE_RPC = "https://mainnet.base.org";
const USDT0 = "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb";
const HL_INFO_URL = "https://api.hyperliquid.xyz/info";

if (!MONGODB_URI) {
  console.log("[stats] MONGODB_URI not set — skipping.");
  process.exit(0);
}

// ── Throttle map ────────────────────────────────────────
const lastShipped = new Map();

// ── EVM helpers (raw eth_call) ──────────────────────────

async function ethCall(rpc, to, data) {
  const resp = await fetch(rpc, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "eth_call",
      params: [{ to, data }, "latest"],
    }),
  });
  const json = await resp.json();
  return json.result || "0x";
}

async function ethBalance(rpc, addr) {
  const resp = await fetch(rpc, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "eth_getBalance",
      params: [addr, "latest"],
    }),
  });
  const json = await resp.json();
  return json.result || "0x0";
}

function decode(hex) {
  if (!hex || hex === "0x") return 0n;
  return BigInt(hex);
}

function encodeAddress(addr) {
  return addr.toLowerCase().replace("0x", "").padStart(64, "0");
}

// ── Price helpers ───────────────────────────────────────

async function getPrices() {
  try {
    const resp = await fetch(HL_INFO_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ type: "allMids" }),
    });
    const mids = await resp.json();
    return {
      eth: parseFloat(mids["ETH"] || "0"),
      hype: parseFloat(mids["HYPE"] || "0"),
    };
  } catch {
    return { eth: 0, hype: 0 };
  }
}

// ── JSON helpers ────────────────────────────────────────

async function readJson(path) {
  try {
    return JSON.parse(await readFile(path, "utf-8"));
  } catch {
    return null;
  }
}

// ── Derive wallet from PK ───────────────────────────────

function deriveWallet() {
  const pk = process.env.DEFI_FLOW_PRIVATE_KEY;
  if (!pk) return null;
  try {
    return execSync(`cast wallet address --private-key ${pk}`, {
      encoding: "utf-8",
      timeout: 5_000,
    }).trim();
  } catch {
    // cast not available — try defi-flow
    // The wallet address is printed in defi-flow's output but we can't easily get it.
    // For now, return null and the script will still work for strategy stats.
    return null;
  }
}

// ── Query strategy via defi-flow query ──────────────────

function queryStrategy(strategyFile, stateFile) {
  const args = [strategyFile];
  if (stateFile) args.push("--state-file", stateFile);
  try {
    const output = execSync(`defi-flow query ${args.join(" ")}`, {
      encoding: "utf-8",
      timeout: 30_000,
      env: { ...process.env },
    }).trim();
    return JSON.parse(output);
  } catch {
    return null;
  }
}

// ── Discover strategies ─────────────────────────────────

async function discoverStrategies() {
  const strategies = [];

  // 1. Try registry
  const registry = await readJson(REGISTRY_PATH);
  if (registry?.daemons) {
    for (const [name, entry] of Object.entries(registry.daemons)) {
      strategies.push({
        name,
        strategy_file: entry.strategy_file,
        state_file: entry.state_file,
        mode: entry.mode || "unknown",
        network: entry.network || "unknown",
        started_at: entry.started_at || null,
      });
    }
  }

  // 2. Also scan strategies dir for any not in registry
  try {
    const files = await readdir(STRATEGIES_DIR);
    for (const f of files.filter((f) => f.endsWith(".json"))) {
      const fullPath = join(STRATEGIES_DIR, f);
      const name = f.replace(".json", "");
      if (strategies.some((s) => s.name === name)) continue;

      // Check if there's a state file
      const stateFile = join(REGISTRY_DIR, "state", `${name}.state.json`);

      strategies.push({
        name,
        strategy_file: fullPath,
        state_file: stateFile,
        mode: "discovered",
        network: "unknown",
        started_at: null,
      });
    }
  } catch {
    // strategies dir doesn't exist
  }

  return strategies;
}

// ── Compute derived metrics ─────────────────────────────

function computeMetrics(name, entry, queryResult, state) {
  // Prefer defi-flow query TVL (fully on-chain) → state.last_tvl → 0
  const tvl = queryResult?.total_tvl || state?.last_tvl || 0;
  const initialCapital = state?.initial_capital || 0;
  const peakTvl = state?.peak_tvl || tvl;
  const startedAt = entry.started_at ? new Date(entry.started_at) : null;

  const pnl = initialCapital > 0 ? tvl - initialCapital : 0;

  const uptimeMs = startedAt ? Date.now() - startedAt.getTime() : 0;
  const uptimeHours = Math.max(uptimeMs / (1000 * 3600), 1);
  const hoursPerYear = 8766;
  const apr =
    initialCapital > 0
      ? (pnl / initialCapital) * (hoursPerYear / uptimeHours)
      : 0;
  const apy = Math.exp(apr) - 1;
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
    // Per-venue breakdown from defi-flow query (if available)
    venues: queryResult?.venues || {},
    wallet_tokens: queryResult?.wallet_tokens || {},
    // Backwards compat: balances map (node_id → USD value)
    balances: state?.balances || {},
    // Cumulative metrics from state
    funding_pnl: state?.cumulative_funding || 0,
    interest_pnl: state?.cumulative_interest || 0,
    rewards_pnl: state?.cumulative_rewards || 0,
    swap_costs: state?.cumulative_costs || 0,
    last_tick: state?.last_tick || 0,
    mode: entry.mode || "unknown",
    network: entry.network || "unknown",
    status: "running",
    model: process.env.MODEL_NAME || "unknown",
  };
}

// ── Ship stats for all strategies ───────────────────────

async function shipStats(db, wallet) {
  const strategies = await discoverStrategies();
  if (strategies.length === 0) {
    // No strategies yet — still ship agent-level stats
  }

  const strategyCol = db.collection("strategy_stats");
  const portfolioCol = db.collection("portfolio_stats");
  let shipped = 0;

  // Per-strategy stats
  const strategyDocs = [];
  for (const entry of strategies) {
    const lastTime = lastShipped.get(entry.name) || 0;
    if (Date.now() - lastTime < MIN_SHIP_INTERVAL_MS) continue;

    // Try defi-flow query for on-chain TVL
    const queryResult = queryStrategy(entry.strategy_file, entry.state_file);

    // Read state file for cumulative metrics
    const state = await readJson(entry.state_file);

    if (!queryResult && !state) {
      console.log(
        `[stats] SKIP ${entry.name} — no query result or state file`
      );
      continue;
    }

    const metrics = computeMetrics(entry.name, entry, queryResult, state);
    strategyDocs.push(metrics);

    try {
      await strategyCol.insertOne(metrics);
      lastShipped.set(entry.name, Date.now());
      shipped++;

      const aprPct = (metrics.apr * 100).toFixed(2);
      const ddPct = (metrics.max_drawdown * 100).toFixed(2);
      console.log(
        `[stats] ${entry.name}: TVL=$${metrics.tvl.toFixed(2)} PnL=$${metrics.pnl.toFixed(2)} APR=${aprPct}% DD=${ddPct}%`
      );
    } catch (err) {
      console.error(`[stats] FAIL ${entry.name}: ${err.message}`);
    }
  }

  // Agent-level portfolio snapshot
  if (wallet) {
    const walletEnc = encodeAddress(wallet);
    const [usdt0Hex, baseEthHex, evmEthHex, prices] = await Promise.all([
      ethCall(HYPEREVM_RPC, USDT0, "0x70a08231" + walletEnc),
      ethBalance(BASE_RPC, wallet),
      ethBalance(HYPEREVM_RPC, wallet),
      getPrices(),
    ]);
    const walletBalance = Number(decode(usdt0Hex)) / 1e6;
    const baseEthBalance = Number(decode(baseEthHex)) / 1e18;
    const evmHypeBalance = Number(decode(evmEthHex)) / 1e18;
    const baseEthValue = baseEthBalance * prices.eth;
    const evmHypeValue = evmHypeBalance * prices.hype; // HyperEVM native = HYPE, not ETH
    const totalGasValue = baseEthValue + evmHypeValue;

    const strategyTvl = strategyDocs.reduce((s, d) => s + d.tvl, 0);
    // Portfolio = idle USDT0 + deployed strategy TVL + gas tokens at market price
    const portfolioTvl = walletBalance + strategyTvl + totalGasValue;

    // Fetch previous peak from MongoDB for drawdown tracking
    const prev = await portfolioCol
      .find({ wallet })
      .sort({ timestamp: -1 })
      .limit(1)
      .toArray();
    const prevDoc = prev[0];
    const initialCapital = INITIAL_CAPITAL;
    // Reset peak if previous was from inflated data (HYPE-as-ETH bug)
    const prevPeak = prevDoc?.peak_tvl || 0;
    const peakTvl = prevPeak > portfolioTvl * 3 ? portfolioTvl : Math.max(prevPeak, portfolioTvl);
    const maxDrawdown = peakTvl > 0 ? Math.max(0, 1 - portfolioTvl / peakTvl) : 0;
    const pnl = portfolioTvl - initialCapital;

    const portfolioDoc = {
      timestamp: new Date(),
      wallet,
      chain: "hyperevm",
      model: process.env.MODEL_NAME || "unknown",
      initial_capital: initialCapital,
      portfolio_tvl: portfolioTvl,
      wallet_balance: walletBalance,
      strategy_tvl: strategyTvl,
      base_eth_balance: baseEthBalance,
      base_eth_value: baseEthValue,
      evm_hype_balance: evmHypeBalance,
      evm_hype_value: evmHypeValue,
      total_gas_value: totalGasValue,
      eth_price: prices.eth,
      hype_price: prices.hype,
      peak_tvl: peakTvl,
      max_drawdown: maxDrawdown,
      strategies_count: strategyDocs.length,
      pnl,
      pnl_percent: initialCapital > 0 ? (pnl / initialCapital) * 100 : 0,
    };

    await portfolioCol.insertOne(portfolioDoc);
    console.log(
      `[stats] Portfolio: TVL=$${portfolioTvl.toFixed(2)} (wallet=$${walletBalance.toFixed(2)} strats=$${strategyTvl.toFixed(2)} gas=$${totalGasValue.toFixed(2)}) PnL=$${pnl.toFixed(2)} DD=${(maxDrawdown * 100).toFixed(1)}%`
    );
  }

  if (shipped > 0) {
    console.log(`[stats] Shipped ${shipped} strategy stats.`);
  }
}

// ── Main ────────────────────────────────────────────────

const wallet = deriveWallet();
console.log(`[stats] Wallet: ${wallet || "unknown (cast unavailable)"}`);

const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  await client.connect();
  await client.db("admin").command({ ping: 1 });
  console.log(`[stats] Connected to MongoDB (db: ${DB_NAME})`);

  const db = client.db(DB_NAME);

  // Create indexes
  const stratCol = db.collection("strategy_stats");
  await stratCol
    .createIndex({ strategy: 1, timestamp: 1 })
    .catch(() => {});
  await stratCol.createIndex({ strategy: 1 }).catch(() => {});

  const portCol = db.collection("portfolio_stats");
  await portCol
    .createIndex({ wallet: 1, timestamp: 1 })
    .catch(() => {});
  await portCol.createIndex({ timestamp: 1 }).catch(() => {});

  // Initial ship
  await shipStats(db, wallet);

  // Watch for state file changes (reactive)
  try {
    const watcher = watch(REGISTRY_DIR, { recursive: true });
    console.log(`[stats] Watching ${REGISTRY_DIR} for changes...`);

    const pollLoop = async () => {
      while (true) {
        await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
        await shipStats(db, wallet).catch((e) =>
          console.error(`[stats] Poll error: ${e.message}`)
        );
      }
    };

    const watchLoop = async () => {
      for await (const event of watcher) {
        if (event.filename?.endsWith(".json")) {
          await new Promise((r) => setTimeout(r, 2000));
          await shipStats(db, wallet).catch((e) =>
            console.error(`[stats] Watch error: ${e.message}`)
          );
        }
      }
    };

    await Promise.race([pollLoop(), watchLoop()]);
  } catch {
    console.log("[stats] fs.watch unavailable, polling only.");
    while (true) {
      await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
      await shipStats(db, wallet).catch((e) =>
        console.error(`[stats] Poll error: ${e.message}`)
      );
    }
  }
} catch (err) {
  console.error("[stats] Stats shipper failed:", err.message);
  process.exit(1);
}
