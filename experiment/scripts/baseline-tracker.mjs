#!/usr/bin/env node
/**
 * Baseline PnL Tracker
 *
 * Tracks the PnL of a control wallet across the 3 Morpho vaults.
 * Pushes snapshots to MongoDB as a baseline for the experiment.
 *
 * Usage:
 *   MONGODB_URI=... node baseline-tracker.mjs
 *   # or run once: node baseline-tracker.mjs --once
 */
import { MongoClient } from "mongodb";

// ── Config ──────────────────────────────────────────────
const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = "baseline";
const WALLET = "0x03947958Df64597b534933e51914acA0420551c3";
const INITIAL_DEPOSIT = 90;
const POLL_INTERVAL = 300_000; // 5 minutes
const RPC_URL = "https://rpc.hyperliquid.xyz/evm";
const ONCE = process.argv.includes("--once");

if (!MONGODB_URI) {
  console.error("[baseline] MONGODB_URI not set");
  process.exit(1);
}

// ── Vault definitions ───────────────────────────────────
const USDT0 = "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb";
const USDT0_DECIMALS = 6;

const VAULTS = [
  {
    name: "Lending Vault",
    strategy: "lending",
    address: "0x58D0F36A87177a4F1Aa8C2eB6e91d424D7248f1C",
  },
  {
    name: "Delta-Neutral Vault",
    strategy: "delta_neutral",
    address: "0x41B5FBB5c6E3938A8536B1d8828a45f7fd839ab6",
  },
  {
    name: "PT Yield Vault",
    strategy: "pt_yield",
    address: "0xe600EB6913376B4Ac7eD645B2bFF8A20B4F8cfB0",
  },
];

// ── ABI encodings (ERC4626) ─────────────────────────────
// We use raw eth_call to avoid external deps

// keccak256 function selectors
const SEL = {
  balanceOf: "0x70a08231", // balanceOf(address)
  totalAssets: "0x01e1d114", // totalAssets() — standard ERC4626
  _totalAssets: "0xce04bebb", // _totalAssets() — Morpho vaults on HyperEVM
  totalSupply: "0x18160ddd", // totalSupply()
  convertToAssets: "0x07a2d13a", // convertToAssets(uint256)
};

function encodeAddress(addr) {
  return addr.toLowerCase().replace("0x", "").padStart(64, "0");
}

function encodeUint256(hex) {
  return hex.replace("0x", "").padStart(64, "0");
}

async function ethCall(to, data) {
  const resp = await fetch(RPC_URL, {
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
  if (json.error) throw new Error(`RPC error: ${json.error.message}`);
  return json.result;
}

function decodeUint256(hex) {
  if (!hex || hex === "0x") return 0n;
  return BigInt(hex);
}

function toUsdt(raw) {
  return Number(raw) / 10 ** USDT0_DECIMALS;
}

// ── Query a single vault ────────────────────────────────
async function queryVault(vault, wallet) {
  const walletEnc = encodeAddress(wallet);

  // Parallel RPC calls — use _totalAssets (Morpho HyperEVM) with fallback
  const [sharesHex, _totalAssetsHex, totalAssetsHex, totalSupplyHex] =
    await Promise.all([
      ethCall(vault.address, SEL.balanceOf + walletEnc),
      ethCall(vault.address, SEL._totalAssets).catch(() => "0x"),
      ethCall(vault.address, SEL.totalAssets).catch(() => "0x"),
      ethCall(vault.address, SEL.totalSupply).catch(() => "0x"),
    ]);

  const shares = decodeUint256(sharesHex);
  const _totalAssets = decodeUint256(_totalAssetsHex);
  const stdTotalAssets = decodeUint256(totalAssetsHex);
  const totalAssets = _totalAssets > 0n ? _totalAssets : stdTotalAssets;
  const totalSupply = decodeUint256(totalSupplyHex);

  // Convert our shares to underlying assets
  let assetsValue = 0n;
  if (shares > 0n) {
    try {
      const assetsHex = await ethCall(
        vault.address,
        SEL.convertToAssets + encodeUint256("0x" + shares.toString(16))
      );
      assetsValue = decodeUint256(assetsHex);
    } catch {
      // convertToAssets reverted — compute manually: shares * totalAssets / totalSupply
      if (totalSupply > 0n) {
        assetsValue = (shares * totalAssets) / totalSupply;
      }
    }
  }

  // Share price: USDT per share (shares=18 decimals, assets=6 decimals)
  const sharePrice =
    totalSupply > 0n
      ? Number(totalAssets * BigInt(10 ** 12)) / Number(totalSupply)
      : 1.0;

  return {
    vault_name: vault.name,
    vault_address: vault.address,
    strategy: vault.strategy,
    shares: shares.toString(),
    shares_value_usdt: toUsdt(assetsValue),
    share_price: sharePrice,
    vault_tvl_usdt: toUsdt(totalAssets),
    vault_total_supply: totalSupply.toString(),
  };
}

// ── Snapshot all vaults ─────────────────────────────────
async function takeSnapshot(db) {
  const collection = db.collection("portfolio_snapshots");

  const vaultResults = await Promise.all(
    VAULTS.map((v) => queryVault(v, WALLET))
  );

  // Also check wallet's raw USDT0 balance
  const walletBalHex = await ethCall(
    USDT0,
    SEL.balanceOf + encodeAddress(WALLET)
  );
  const walletBalance = toUsdt(decodeUint256(walletBalHex));

  // Total portfolio value across all vaults
  const totalVaultValue = vaultResults.reduce(
    (sum, v) => sum + v.shares_value_usdt,
    0
  );

  const doc = {
    timestamp: new Date(),
    wallet: WALLET,
    chain: "hyperevm",
    wallet_usdt_balance: walletBalance,
    total_vault_value: totalVaultValue,
    total_portfolio_value: totalVaultValue + walletBalance,
    vaults: vaultResults,
  };

  // PnL against the known initial deposit
  doc.initial_value = INITIAL_DEPOSIT;
  doc.pnl_absolute = doc.total_portfolio_value - INITIAL_DEPOSIT;
  doc.pnl_percent =
    INITIAL_DEPOSIT > 0
      ? ((doc.total_portfolio_value - INITIAL_DEPOSIT) / INITIAL_DEPOSIT) * 100
      : 0;

  // Time-weighted APR: use first snapshot timestamp as start time
  const firstSnapshot = await collection.findOne(
    { wallet: WALLET },
    { sort: { timestamp: 1 } }
  );

  const startTime = firstSnapshot
    ? firstSnapshot.timestamp.getTime()
    : Date.now();
  const hoursElapsed = Math.max(
    (Date.now() - startTime) / (1000 * 3600),
    1
  );
  const hoursPerYear = 8766;
  doc.apr =
    INITIAL_DEPOSIT > 0
      ? (doc.pnl_absolute / INITIAL_DEPOSIT) * (hoursPerYear / hoursElapsed)
      : 0;
  doc.hours_elapsed = Math.round(hoursElapsed * 10) / 10;

  await collection.insertOne(doc);

  // Log summary
  const vaultSummary = vaultResults
    .map(
      (v) =>
        `  ${v.vault_name}: $${v.shares_value_usdt.toFixed(2)} (price: ${v.share_price.toFixed(6)}, tvl: $${v.vault_tvl_usdt.toFixed(2)})`
    )
    .join("\n");

  console.log(
    `[baseline] ${new Date().toISOString()}\n` +
      `  Wallet: ${WALLET}\n` +
      `  USDT0 balance: $${walletBalance.toFixed(2)}\n` +
      vaultSummary +
      `\n  Total: $${doc.total_portfolio_value.toFixed(2)} | PnL: $${doc.pnl_absolute.toFixed(2)} (${doc.pnl_percent.toFixed(2)}%) | APR: ${(doc.apr * 100).toFixed(2)}%`
  );

  return doc;
}

// ── Main ────────────────────────────────────────────────
const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  await client.connect();
  await client.db("admin").command({ ping: 1 });
  console.log(`[baseline] Connected to MongoDB, tracking ${WALLET} ($${INITIAL_DEPOSIT} deposited)`);

  const db = client.db(DB_NAME);
  const collection = db.collection("portfolio_snapshots");

  // Create indexes
  await collection
    .createIndex({ wallet: 1, timestamp: 1 })
    .catch(() => {});
  await collection.createIndex({ timestamp: 1 }).catch(() => {});

  // Take first snapshot
  await takeSnapshot(db);

  if (ONCE) {
    console.log("[baseline] --once flag set, exiting.");
    await client.close();
    process.exit(0);
  }

  // Poll loop
  console.log(
    `[baseline] Polling every ${POLL_INTERVAL / 1000}s. Ctrl+C to stop.`
  );
  while (true) {
    await new Promise((r) => setTimeout(r, POLL_INTERVAL));
    await takeSnapshot(db).catch((e) =>
      console.error(`[baseline] Snapshot error: ${e.message}`)
    );
  }
} catch (err) {
  console.error("[baseline] Fatal:", err.message);
  process.exit(1);
}
