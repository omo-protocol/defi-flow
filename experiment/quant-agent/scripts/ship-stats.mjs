#!/usr/bin/env node
/**
 * Ships quant agent strategy stats to MongoDB.
 *
 * Uses `defi-flow query` per strategy for on-chain TVL (no double counting).
 * Only raw RPC calls are for gas token balances (ETH on Base, HYPE on HyperEVM).
 *
 * Strategy discovery: reads registry.json OR scans /app/strategies/*.json
 * Runs as a long-lived daemon: reactive via fs.watch + 5-min poll fallback.
 *
 * Env: MONGODB_URI, MONGODB_DB, DEFI_FLOW_PRIVATE_KEY,
 *      DEFI_FLOW_REGISTRY_DIR (default: /app/.defi-flow)
 */
import { MongoClient } from "mongodb";
import { readFile, watch, readdir } from "fs/promises";
import { join } from "path";
import { execSync } from "child_process";

// ── Singleton: ensure only one ship-stats process runs ──
try {
  const myPid = process.pid;
  const pids = execSync('pgrep -f "node.*ship-stats"', { encoding: "utf-8" })
    .trim().split("\n").map(Number).filter((p) => p !== myPid && p > 0);
  for (const pid of pids) {
    try { process.kill(pid, "SIGKILL"); } catch {}
  }
  if (pids.length > 0) console.log(`[stats] Singleton: killed ${pids.length} other ship-stats processes`);
} catch {}

// ── Config ──────────────────────────────────────────────
const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "agent-logs";
const REGISTRY_DIR = process.env.DEFI_FLOW_REGISTRY_DIR || "/app/.defi-flow";
const REGISTRY_PATH = join(REGISTRY_DIR, "registry.json");
const STRATEGIES_DIR = process.env.STRATEGIES_DIR || "/app/strategies";
const POLL_INTERVAL_MS = 5 * 60 * 1000;
const MIN_SHIP_INTERVAL_MS = 30_000;

let INITIAL_CAPITAL = null;
const HYPEREVM_RPC = "https://rpc.hyperliquid.xyz/evm";
const BASE_RPC = "https://mainnet.base.org";
const HL_INFO_URL = "https://api.hyperliquid.xyz/info";
const USDT0 = "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb";

if (!MONGODB_URI) {
  console.log("[stats] MONGODB_URI not set — skipping.");
  process.exit(0);
}

// ── Throttle ────────────────────────────────────────────
const lastShipped = new Map();
const lastKnownTvl = new Map();
let lastKnownGas = null; // carry-forward for gas RPC glitches only

// ── Gas token helpers (only raw RPC we still need) ──────

async function rpcCall(rpc, method, params) {
  const resp = await fetch(rpc, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const json = await resp.json();
  return json.result || "0x0";
}

async function ethBalance(rpc, addr) {
  const hex = await rpcCall(rpc, "eth_getBalance", [addr, "latest"]);
  return Number(BigInt(hex)) / 1e18;
}

async function erc20Balance(rpc, token, wallet) {
  const walletEnc = wallet.toLowerCase().replace("0x", "").padStart(64, "0");
  const hex = await rpcCall(rpc, "eth_call", [{ to: token, data: "0x70a08231" + walletEnc }, "latest"]);
  if (!hex || hex === "0x") return 0;
  return Number(BigInt(hex)) / 1e6; // USDT0/USDC = 6 decimals
}

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
      encoding: "utf-8", timeout: 5_000,
    }).trim();
  } catch {
    return null;
  }
}

// ── Query strategy via defi-flow query ──────────────────

function queryStrategy(strategyFile, stateFile) {
  const args = [strategyFile];
  if (stateFile) args.push("--state-file", stateFile);
  try {
    const raw = execSync(`defi-flow query ${args.join(" ")}`, {
      encoding: "utf-8", timeout: 30_000, env: { ...process.env },
    });
    // defi-flow query prints JSON on first line, may print "Loaded state..." on stderr
    const jsonLine = raw.split("\n").find((l) => l.startsWith("{"));
    return jsonLine ? JSON.parse(jsonLine) : null;
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
      const stateFile = join(REGISTRY_DIR, "state", `${name}.state.json`);
      strategies.push({
        name, strategy_file: fullPath, state_file: stateFile,
        mode: "discovered", network: "unknown", started_at: null,
      });
    }
  } catch {}

  return strategies;
}

// ── Compute per-strategy derived metrics ────────────────

function computeMetrics(name, entry, queryResult, state) {
  let tvl = queryResult?.total_tvl || state?.last_tvl || 0;
  if (tvl === 0 && lastKnownTvl.has(name)) {
    tvl = lastKnownTvl.get(name);
  } else if (tvl > 0) {
    lastKnownTvl.set(name, tvl);
  }
  const initialCapital = state?.initial_capital || 0;
  const peakTvl = state?.peak_tvl || tvl;
  const startedAt = entry.started_at ? new Date(entry.started_at) : null;
  const pnl = initialCapital > 0 ? tvl - initialCapital : 0;
  const uptimeMs = startedAt ? Date.now() - startedAt.getTime() : 0;
  const uptimeHours = Math.max(uptimeMs / (1000 * 3600), 1);
  const apr = initialCapital > 0 ? (pnl / initialCapital) * (8766 / uptimeHours) : 0;
  const maxDrawdown = peakTvl > 0 ? Math.max(0, 1 - tvl / peakTvl) : 0;

  return {
    strategy: name, timestamp: new Date(), tvl, pnl, apr,
    apy: Math.exp(apr) - 1, max_drawdown: maxDrawdown,
    initial_capital: initialCapital, peak_tvl: peakTvl,
    uptime_hours: Math.round(uptimeHours * 10) / 10,
    venues: queryResult?.venues || {},
    wallet_tokens: queryResult?.wallet_tokens || {},
    balances: state?.balances || {},
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

// ── Ship stats ──────────────────────────────────────────

async function shipStats(db, wallet) {
  const strategies = await discoverStrategies();
  const strategyCol = db.collection("strategy_stats");
  const portfolioCol = db.collection("portfolio_stats");
  let shipped = 0;

  // Auto-detect IC from first MongoDB record
  if (INITIAL_CAPITAL === null && wallet) {
    const first = await portfolioCol
      .find({ wallet }).sort({ timestamp: 1 }).limit(1).toArray();
    if (first.length > 0) {
      INITIAL_CAPITAL = first[0].initial_capital || first[0].portfolio_tvl;
      console.log(`[stats] Auto-detected IC: $${INITIAL_CAPITAL.toFixed(2)}`);
    }
  }

  // ── Per-strategy stats (via defi-flow query) ──────────
  const queryResults = new Map(); // name → queryResult
  // Dedup: track MAX venue_tvl per wallet (all HL perps report same account value)
  const maxVenueTvlPerWallet = new Map();

  for (const entry of strategies) {
    const lastTime = lastShipped.get(entry.name) || 0;
    if (Date.now() - lastTime < MIN_SHIP_INTERVAL_MS) continue;

    const queryResult = queryStrategy(entry.strategy_file, entry.state_file);
    const state = await readJson(entry.state_file);

    if (!queryResult && !state) {
      console.log(`[stats] SKIP ${entry.name} — no query result or state file`);
      continue;
    }

    queryResults.set(entry.name, queryResult);

    // Track highest venue_tvl per wallet (dedup HL double-counting)
    if (queryResult?.wallet) {
      const prev = maxVenueTvlPerWallet.get(queryResult.wallet) || 0;
      const cur = queryResult.venue_tvl || 0;
      if (cur > prev) maxVenueTvlPerWallet.set(queryResult.wallet, cur);
    }

    const metrics = computeMetrics(entry.name, entry, queryResult, state);
    try {
      await strategyCol.insertOne(metrics);
      lastShipped.set(entry.name, Date.now());
      shipped++;
      console.log(
        `[stats] ${entry.name}: TVL=$${metrics.tvl.toFixed(2)} PnL=$${metrics.pnl.toFixed(2)} APR=${(metrics.apr * 100).toFixed(2)}%`
      );
    } catch (err) {
      console.error(`[stats] FAIL ${entry.name}: ${err.message}`);
    }
  }

  // Aggregated venue TVL = sum of max per wallet (deduped)
  let venueTvl = 0;
  for (const v of maxVenueTvlPerWallet.values()) venueTvl += v;

  // ── Portfolio snapshot ────────────────────────────────
  if (wallet) {
    // Wallet USDT0 balance + gas tokens (3 RPC calls, all parallel)
    const [walletBalance, baseEthBal, evmHypeBal, prices] = await Promise.all([
      erc20Balance(HYPEREVM_RPC, USDT0, wallet),
      ethBalance(BASE_RPC, wallet),
      ethBalance(HYPEREVM_RPC, wallet),
      getPrices(),
    ]);

    let baseEthBalance = baseEthBal;
    let evmHypeBalance = evmHypeBal;
    let baseEthValue = baseEthBalance * prices.eth;
    let evmHypeValue = evmHypeBalance * prices.hype;

    // Carry forward gas only (the only RPC that glitches)
    const ref = lastKnownGas;
    if (ref) {
      if (baseEthBalance === 0 && ref.baseEthBalance > 0) {
        console.log(`[stats] Base RPC glitch — carry forward ETH=${ref.baseEthBalance.toFixed(4)}`);
        baseEthBalance = ref.baseEthBalance;
        baseEthValue = ref.baseEthValue;
      }
      if (evmHypeBalance === 0 && ref.evmHypeBalance > 0) {
        console.log(`[stats] HyperEVM RPC glitch — carry forward HYPE=${ref.evmHypeBalance.toFixed(4)}`);
        evmHypeBalance = ref.evmHypeBalance;
        evmHypeValue = ref.evmHypeValue;
      }
    }
    lastKnownGas = { baseEthBalance, baseEthValue, evmHypeBalance, evmHypeValue };

    const totalGasValue = baseEthValue + evmHypeValue;

    // Portfolio = USDT0 wallet + venue TVL (from query, deduped) + gas
    const strategyTvl = venueTvl;
    const portfolioTvl = walletBalance + strategyTvl + totalGasValue;
    console.log(`[stats] Components: USDT0=$${walletBalance.toFixed(2)} venues=$${strategyTvl.toFixed(2)} gas=$${totalGasValue.toFixed(2)}`);

    if (INITIAL_CAPITAL === null) {
      INITIAL_CAPITAL = portfolioTvl;
      console.log(`[stats] First ship — IC: $${INITIAL_CAPITAL.toFixed(2)}`);
    }

    // Peak tracking from previous MongoDB record
    const prev = await portfolioCol
      .find({ wallet }).sort({ timestamp: -1 }).limit(1).toArray();
    const prevDoc = prev[0];
    const prevPeak = prevDoc?.peak_tvl || 0;
    const peakTvl = prevPeak > portfolioTvl * 3 ? portfolioTvl : Math.max(prevPeak, portfolioTvl);
    const maxDrawdown = peakTvl > 0 ? Math.max(0, 1 - portfolioTvl / peakTvl) : 0;
    const pnl = portfolioTvl - INITIAL_CAPITAL;

    await portfolioCol.insertOne({
      timestamp: new Date(),
      wallet,
      chain: "hyperevm",
      model: process.env.MODEL_NAME || "unknown",
      initial_capital: INITIAL_CAPITAL,
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
      strategies_count: queryResults.size,
      pnl,
      pnl_percent: INITIAL_CAPITAL > 0 ? (pnl / INITIAL_CAPITAL) * 100 : 0,
    });

    console.log(
      `[stats] Portfolio: TVL=$${portfolioTvl.toFixed(2)} (wallet=$${walletBalance.toFixed(2)} strats=$${strategyTvl.toFixed(2)} gas=$${totalGasValue.toFixed(2)}) PnL=$${pnl.toFixed(2)}`
    );
  }

  if (shipped > 0) console.log(`[stats] Shipped ${shipped} strategy stats.`);
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

  const stratCol = db.collection("strategy_stats");
  await stratCol.createIndex({ strategy: 1, timestamp: 1 }).catch(() => {});
  await stratCol.createIndex({ strategy: 1 }).catch(() => {});

  const portCol = db.collection("portfolio_stats");
  await portCol.createIndex({ wallet: 1, timestamp: 1 }).catch(() => {});
  await portCol.createIndex({ timestamp: 1 }).catch(() => {});

  // Initial ship
  await shipStats(db, wallet);

  // Watch for state file changes (reactive) + poll fallback
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
