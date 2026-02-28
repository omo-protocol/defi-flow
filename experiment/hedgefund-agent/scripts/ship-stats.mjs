#!/usr/bin/env node
/**
 * Ships portfolio stats to MongoDB.
 *
 * Reads defi-flow state files (mounted from strategy containers) for each
 * strategy's perf metrics, adds on-chain vault queries (reserve ratio,
 * idle balance, share price) via `cast`, and ships to MongoDB.
 *
 * Runs once per invocation — call from cron or entrypoint loop.
 *
 * Env: MONGODB_URI, MONGODB_DB, PRIVATE_KEY,
 *      VAULTS_JSON (default: /app/workspace/vaults.json),
 *      STRATEGY_STATES_DIR (default: /app/strategy-states)
 */
import { MongoClient } from "mongodb";
import { readFile, readdir } from "fs/promises";
import { join } from "path";
import { execSync } from "child_process";

const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "hedgefund-agent";
const PRIVATE_KEY = process.env.PRIVATE_KEY;
const VAULTS_JSON = process.env.VAULTS_JSON || "/app/workspace/vaults.json";
const STATES_DIR = process.env.STRATEGY_STATES_DIR || "/app/strategy-states";

if (!MONGODB_URI) {
  console.log("[stats] MONGODB_URI not set — skipping.");
  process.exit(0);
}

// ── Helpers ─────────────────────────────────────────────

async function readJson(path) {
  try {
    return JSON.parse(await readFile(path, "utf-8"));
  } catch {
    return null;
  }
}

function cast(target, sig, args, rpcUrl) {
  const cmd = `cast call ${target} "${sig}" ${args.join(" ")} --rpc-url ${rpcUrl}`;
  try {
    return execSync(cmd, { encoding: "utf-8", timeout: 15_000 }).trim();
  } catch {
    return null;
  }
}

function parseUnits(raw, decimals = 6) {
  if (!raw) return 0;
  const clean = raw.split(/\s/)[0].replace(/[^0-9]/g, "");
  if (!clean) return 0;
  try {
    const big = BigInt(clean);
    const div = BigInt(10 ** decimals);
    return Number(big / div) + Number(big % div) / Number(div);
  } catch {
    return 0;
  }
}

function walletAddress() {
  if (!PRIVATE_KEY) return null;
  try {
    return execSync(`cast wallet address --private-key ${PRIVATE_KEY}`, {
      encoding: "utf-8",
      timeout: 5_000,
    }).trim();
  } catch {
    return null;
  }
}

// ── Collect strategy states ─────────────────────────────

async function collectStrategyStats() {
  const strategies = [];
  try {
    // Each strategy is mounted as a subdirectory: /app/strategy-states/<name>/
    // State file lives at /app/strategy-states/<name>/state.json
    const dirs = await readdir(STATES_DIR);
    for (const dir of dirs) {
      const statePath = join(STATES_DIR, dir, "state.json");
      const state = await readJson(statePath);
      if (!state) continue;

      const balances = state.balances || {};
      const tvl = Object.values(balances).reduce((a, b) => a + b, 0);
      const initial = state.initial_capital || 0;
      const pnl = initial > 0 ? tvl - initial : 0;

      strategies.push({
        name: dir,
        tvl,
        initial_capital: initial,
        peak_tvl: state.peak_tvl || 0,
        pnl,
        last_tick: state.last_tick || 0,
        funding_pnl: state.cumulative_funding || 0,
        interest_pnl: state.cumulative_interest || 0,
        rewards_pnl: state.cumulative_rewards || 0,
        costs: state.cumulative_costs || 0,
      });
    }
  } catch {
    // states dir doesn't exist or can't read — fine
  }
  return strategies;
}

// ── Collect vault on-chain data ─────────────────────────

async function collectVaultStats(vaultsConfig, wallet) {
  if (!vaultsConfig?.vaults) return [];
  const rpc = vaultsConfig.chain?.rpc_url || "https://rpc.hyperliquid.xyz/evm";
  const baseToken = vaultsConfig.base_token?.address;
  const decimals = vaultsConfig.base_token?.decimals || 6;
  const vaults = [];

  for (const v of vaultsConfig.vaults) {
    if (v.address.includes("REPLACE")) {
      vaults.push({ name: v.name, strategy: v.strategy, status: "not_deployed" });
      continue;
    }

    const totalAssets = parseUnits(cast(v.address, "totalAssets()(uint256)", [], rpc), decimals);
    const idle = baseToken
      ? parseUnits(cast(baseToken, "balanceOf(address)(uint256)", [v.address], rpc), decimals)
      : 0;

    let ourValue = 0;
    let ourShares = 0;
    if (wallet) {
      const sharesRaw = cast(v.address, "balanceOf(address)(uint256)", [wallet], rpc);
      const shares = sharesRaw?.split(/\s/)[0]?.replace(/[^0-9]/g, "") || "0";
      if (shares !== "0") {
        // Try convertToAssets first (standard ERC4626)
        ourValue = parseUnits(cast(v.address, "convertToAssets(uint256)(uint256)", [shares], rpc), decimals);
        // Fallback: if convertToAssets reverts, estimate from share ratio
        if (ourValue === 0) {
          const totalSupplyRaw = cast(v.address, "totalSupply()(uint256)", [], rpc);
          const totalSupply = totalSupplyRaw?.split(/\s/)[0]?.replace(/[^0-9]/g, "") || "0";
          if (totalSupply !== "0") {
            const ratio = Number(BigInt(shares) * 10000n / BigInt(totalSupply)) / 10000;
            // Query strategy wallet balance on-chain for allocated capital (no state files)
            let allocated = 0;
            if (v.strategy_wallet && baseToken) {
              allocated = parseUnits(
                cast(baseToken, "balanceOf(address)(uint256)", [v.strategy_wallet], rpc),
                decimals
              );
            }
            const vaultValue = totalAssets > 0 ? totalAssets : (idle + allocated);
            ourValue = ratio * vaultValue;
          }
        }
        ourShares = parseUnits(sharesRaw, 18); // vault shares are 18 decimals
      }
    }

    const reserveRatio = totalAssets > 0 ? idle / totalAssets : 0;

    vaults.push({
      name: v.name,
      strategy: v.strategy,
      address: v.address,
      status: "active",
      total_assets: totalAssets,
      idle_balance: idle,
      reserve_ratio: reserveRatio,
      reserve_health: reserveRatio < (v.reserve?.trigger_threshold || 0.05) ? "critical"
        : reserveRatio < (v.reserve?.target_ratio || 0.2) * 0.5 ? "warning" : "healthy",
      our_position_value: ourValue,
      our_shares: ourShares,
    });
  }

  return vaults;
}

// ── Main ────────────────────────────────────────────────

const wallet = walletAddress();
const vaultsConfig = await readJson(VAULTS_JSON);
const strategies = await collectStrategyStats();
const vaults = await collectVaultStats(vaultsConfig, wallet);

const totalStrategyTvl = strategies.reduce((s, st) => s + st.tvl, 0);
const totalVaultPositions = vaults.reduce((s, v) => s + (v.our_position_value || 0), 0);

// Agent's own USDT0 wallet balance
const rpc = vaultsConfig?.chain?.rpc_url || "https://rpc.hyperliquid.xyz/evm";
const baseToken = vaultsConfig?.base_token?.address;
const decimals = vaultsConfig?.base_token?.decimals || 6;
const walletBalance = (wallet && baseToken)
  ? parseUnits(cast(baseToken, "balanceOf(address)(uint256)", [wallet], rpc), decimals)
  : 0;

// Portfolio = agent's own holdings (wallet balance + vault share value)
// Strategy TVLs are shared across agents — informational only, not portfolio
const agentPortfolio = walletBalance + totalVaultPositions;

const doc = {
  timestamp: new Date(),
  wallet: wallet || "unknown",
  chain: vaultsConfig?.chain?.name || "hyperevm",
  model: process.env.MODEL_NAME || "unknown",
  portfolio_tvl: agentPortfolio,
  wallet_balance: walletBalance,
  vault_positions: totalVaultPositions,
  strategy_tvl: totalStrategyTvl,
  strategies,
  vaults,
};

const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  await client.connect();
  const db = client.db(DB_NAME);
  const col = db.collection("portfolio_stats");
  await col.createIndex({ timestamp: 1 }).catch(() => {});
  await col.insertOne(doc);

  console.log(
    `[stats] TVL=$${doc.portfolio_tvl.toFixed(2)} ` +
    `(${strategies.length} strategies, ${vaults.filter(v => v.status === "active").length} vaults)`
  );
  for (const v of vaults.filter(v => v.status === "active")) {
    console.log(`[stats]   ${v.name}: $${v.total_assets.toFixed(0)} reserve=${(v.reserve_ratio*100).toFixed(1)}% [${v.reserve_health}]`);
  }
} catch (err) {
  console.error(`[stats] Failed: ${err.message}`);
  process.exit(1);
} finally {
  await client.close();
}
