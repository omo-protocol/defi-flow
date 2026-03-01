#!/usr/bin/env node
/**
 * Ships hedgefund agent portfolio stats to MongoDB.
 *
 * For each agent (identified by PRIVATE_KEY):
 *   1. Agent wallet USDT0 balance on HyperEVM
 *   2. Vault share positions → convertToAssets for USD value
 *   3. ETH on Base (gas tracking)
 *   4. Per-strategy metrics from shared state files (supplementary)
 *
 * Vault definitions come from vaults.json — fully dynamic, no hardcoded
 * strategy-specific queries.
 *
 * Runs once per invocation — call from cron or entrypoint loop.
 *
 * Env: MONGODB_URI, MONGODB_DB, PRIVATE_KEY,
 *      VAULTS_JSON (default: /app/workspace/vaults.json),
 *      STRATEGY_STATES_DIR (default: /app/strategy-states)
 */
import { MongoClient } from "mongodb";
import { readFile } from "fs/promises";
import { execSync } from "child_process";

const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "hedgefund-agent";
const PRIVATE_KEY = process.env.PRIVATE_KEY;
const VAULTS_JSON = process.env.VAULTS_JSON || "/app/workspace/vaults.json";
const STRATEGY_STATES_DIR =
  process.env.STRATEGY_STATES_DIR || "/app/strategy-states";

const HYPEREVM_RPC = "https://rpc.hyperliquid.xyz/evm";
const BASE_RPC = "https://mainnet.base.org";
const USDT0 = "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb";
const USDT0_DECIMALS = 6;
const INITIAL_CAPITAL = 90; // Each agent started with $90

if (!MONGODB_URI) {
  console.log("[stats] MONGODB_URI not set — skipping.");
  process.exit(0);
}

// ── EVM helpers (raw eth_call, no cast dependency) ──────

const SEL = {
  balanceOf: "0x70a08231",
  totalAssets: "0x01e1d114",
  _totalAssets: "0xce04bebb", // Morpho vaults on HyperEVM
  totalSupply: "0x18160ddd",
  convertToAssets: "0x07a2d13a",
};

function encodeAddress(addr) {
  return addr.toLowerCase().replace("0x", "").padStart(64, "0");
}

function encodeUint256(hex) {
  return hex.replace("0x", "").padStart(64, "0");
}

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
  if (json.error) return "0x";
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

function toUsdt(raw) {
  return Number(raw) / 10 ** USDT0_DECIMALS;
}

function toEth(raw) {
  return Number(raw) / 1e18;
}

// ── Wallet address from private key (secp256k1) ────────
// Minimal: use cast if available, otherwise skip
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

async function readJson(path) {
  try {
    return JSON.parse(await readFile(path, "utf-8"));
  } catch {
    return null;
  }
}

// ── Query vault positions ───────────────────────────────

async function queryVaultPosition(vaultAddr, wallet, rpc, decimals) {
  const walletEnc = encodeAddress(wallet);

  const [sharesHex, _totalAssetsHex, totalAssetsHex, totalSupplyHex] =
    await Promise.all([
      ethCall(rpc, vaultAddr, SEL.balanceOf + walletEnc),
      ethCall(rpc, vaultAddr, SEL._totalAssets).catch(() => "0x"),
      ethCall(rpc, vaultAddr, SEL.totalAssets).catch(() => "0x"),
      ethCall(rpc, vaultAddr, SEL.totalSupply).catch(() => "0x"),
    ]);

  const shares = decode(sharesHex);
  const _totalAssets = decode(_totalAssetsHex);
  const stdTotalAssets = decode(totalAssetsHex);
  const totalAssets = _totalAssets > 0n ? _totalAssets : stdTotalAssets;
  const totalSupply = decode(totalSupplyHex);

  // Convert our shares to underlying assets
  let assetsValue = 0n;
  if (shares > 0n) {
    try {
      const assetsHex = await ethCall(
        rpc,
        vaultAddr,
        SEL.convertToAssets + encodeUint256("0x" + shares.toString(16))
      );
      assetsValue = decode(assetsHex);
    } catch {
      // Fallback: manual ratio
      if (totalSupply > 0n) {
        assetsValue = (shares * totalAssets) / totalSupply;
      }
    }
  }

  // Idle vault balance (base token in vault contract)
  const idleHex = await ethCall(
    rpc,
    USDT0,
    SEL.balanceOf + encodeAddress(vaultAddr)
  );
  const idle = decode(idleHex);
  const totalAssetsNum = toUsdt(totalAssets);
  const idleNum = toUsdt(idle);

  return {
    shares: toEth(shares), // shares are 18 decimals
    our_position_value: toUsdt(assetsValue),
    total_assets: totalAssetsNum,
    idle_balance: idleNum,
    reserve_ratio: totalAssetsNum > 0 ? idleNum / totalAssetsNum : 0,
  };
}

// ── Main ────────────────────────────────────────────────

// Derive wallet address
let wallet = null;
if (PRIVATE_KEY) {
  try {
    wallet = execSync(`cast wallet address --private-key ${PRIVATE_KEY}`, {
      encoding: "utf-8",
      timeout: 5_000,
    }).trim();
  } catch {
    console.error("[stats] Failed to derive wallet address from PRIVATE_KEY");
  }
}

if (!wallet) {
  console.error("[stats] No wallet — set PRIVATE_KEY");
  process.exit(1);
}

const vaultsConfig = await readJson(VAULTS_JSON);
const rpc = vaultsConfig?.chain?.rpc_url || HYPEREVM_RPC;
const decimals = vaultsConfig?.base_token?.decimals || 6;
const walletEnc = encodeAddress(wallet);

// 1. Agent wallet USDT0 on HyperEVM
const walletUsdt0Hex = await ethCall(rpc, USDT0, SEL.balanceOf + walletEnc);
const walletBalance = toUsdt(decode(walletUsdt0Hex));

// 2. ETH on Base (gas money)
const baseEthHex = await ethBalance(BASE_RPC, wallet);
const baseEthBalance = toEth(decode(baseEthHex));

// 3. Vault positions — dynamic from vaults.json
const vaults = [];
const strategies = [];
let totalVaultPositions = 0;

if (vaultsConfig?.vaults) {
  for (const v of vaultsConfig.vaults) {
    if (v.address.includes("REPLACE")) {
      vaults.push({ name: v.name, strategy: v.strategy, status: "not_deployed" });
      continue;
    }

    const pos = await queryVaultPosition(v.address, wallet, rpc, decimals);
    totalVaultPositions += pos.our_position_value;

    const reserveHealth =
      pos.reserve_ratio < (v.reserve?.trigger_threshold || 0.05)
        ? "critical"
        : pos.reserve_ratio < (v.reserve?.target_ratio || 0.2) * 0.5
          ? "warning"
          : "healthy";

    vaults.push({
      name: v.name,
      strategy: v.strategy,
      address: v.address,
      status: "active",
      total_assets: pos.total_assets,
      idle_balance: pos.idle_balance,
      reserve_ratio: pos.reserve_ratio,
      reserve_health: reserveHealth,
      our_position_value: pos.our_position_value,
      our_shares: pos.shares,
    });

    // 4. Per-strategy metrics from shared state files (supplementary)
    const strategyName = v.strategy.replace(/_basic$/, ""); // normalize
    const state = await readJson(
      `${STRATEGY_STATES_DIR}/${strategyName}/state.json`
    );
    // Also try the raw strategy name
    const stateAlt = state || await readJson(
      `${STRATEGY_STATES_DIR}/${v.strategy}/state.json`
    );
    const st = state || stateAlt;

    strategies.push({
      name: v.strategy,
      wallet: v.strategy_wallet || "unknown",
      tvl: st?.last_tvl || 0,
      wallet_balance: 0, // strategy wallet balance — daemon tracks this
      venue_tvl: st?.last_tvl || 0,
      source: st ? "state-file" : "unavailable",
      // Extra metrics from state
      initial_capital: st?.initial_capital || 0,
      peak_tvl: st?.peak_tvl || 0,
      cumulative_funding: st?.cumulative_funding || 0,
      cumulative_interest: st?.cumulative_interest || 0,
      cumulative_rewards: st?.cumulative_rewards || 0,
      cumulative_costs: st?.cumulative_costs || 0,
    });
  }
}

const totalStrategyTvl = strategies.reduce((s, st) => s + st.tvl, 0);
const portfolioTvl = walletBalance + totalVaultPositions;

const doc = {
  timestamp: new Date(),
  wallet,
  chain: vaultsConfig?.chain?.name || "hyperevm",
  model: process.env.MODEL_NAME || "unknown",
  initial_capital: INITIAL_CAPITAL,
  portfolio_tvl: portfolioTvl,
  wallet_balance: walletBalance,
  vault_positions: totalVaultPositions,
  base_eth_balance: baseEthBalance,
  strategy_tvl: totalStrategyTvl,
  strategies,
  vaults,
};

// Derived metrics
doc.pnl = portfolioTvl - INITIAL_CAPITAL;
doc.pnl_percent =
  INITIAL_CAPITAL > 0 ? (doc.pnl / INITIAL_CAPITAL) * 100 : 0;

const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  await client.connect();
  const db = client.db(DB_NAME);
  const col = db.collection("portfolio_stats");
  await col.createIndex({ timestamp: 1 }).catch(() => {});
  await col.createIndex({ wallet: 1, timestamp: 1 }).catch(() => {});
  await col.insertOne(doc);

  console.log(
    `[stats] ${wallet.slice(0, 8)}… TVL=$${portfolioTvl.toFixed(2)} ` +
      `(wallet=$${walletBalance.toFixed(2)} vaults=$${totalVaultPositions.toFixed(2)} base=${baseEthBalance.toFixed(4)}ETH)`
  );
  for (const v of vaults.filter((v) => v.status === "active")) {
    console.log(
      `[stats]   ${v.name}: $${v.total_assets.toFixed(0)} reserve=${(v.reserve_ratio * 100).toFixed(1)}% [${v.reserve_health}] our=$${v.our_position_value.toFixed(2)}`
    );
  }
} catch (err) {
  console.error(`[stats] Failed: ${err.message}`);
  process.exit(1);
} finally {
  await client.close();
}
