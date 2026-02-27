#!/usr/bin/env node
/**
 * Query MongoDB timeseries data for defi-flow agents.
 *
 * Usage:
 *   node query-mongo.mjs                     # all data
 *   node query-mongo.mjs --hours 24          # last 24h
 *   node query-mongo.mjs --strategy lending  # filter by strategy name
 *
 * Env:
 *   MONGODB_URI  — connection string (defaults to localhost)
 */

import { MongoClient } from "mongodb";

const MONGODB_URI =
  process.env.MONGODB_URI || "mongodb://localhost:27017";

const hours = (() => {
  const i = process.argv.indexOf("--hours");
  return i !== -1 ? Number(process.argv[i + 1]) : null;
})();

const strategyFilter = (() => {
  const i = process.argv.indexOf("--strategy");
  return i !== -1 ? process.argv[i + 1] : null;
})();

const since = hours
  ? new Date(Date.now() - hours * 3600_000)
  : new Date(0);

const client = new MongoClient(MONGODB_URI);

try {
  await client.connect();

  // ───────────────────────────────────────────────────────
  // 1. Quant Agent — strategy_stats (per-strategy timeseries)
  // ───────────────────────────────────────────────────────
  //
  // Schema:
  //   {
  //     strategy:        String,      // e.g. "lending", "delta_neutral"
  //     timestamp:       Date,
  //     tvl:             Number,
  //     pnl:             Number,
  //     apr:             Number,
  //     apy:             Number,
  //     max_drawdown:    Number,
  //     initial_capital: Number,
  //     peak_tvl:        Number,
  //     uptime_hours:    Number,
  //     balances:        Object,      // { "node_id": { token, amount, usd_value } }
  //     funding_pnl:     Number,
  //     interest_pnl:    Number,
  //     rewards_pnl:     Number,
  //     swap_costs:      Number,
  //     last_tick:       String,      // ISO timestamp of last engine tick
  //     mode:            String,      // "live" | "paper"
  //     network:         String,      // chain name
  //     status:          String,      // "running" | "stopped"
  //   }
  //
  const quantDb = client.db("quant-agent");
  const statsFilter = { timestamp: { $gte: since } };
  if (strategyFilter) statsFilter.strategy = strategyFilter;

  const strategyStats = await quantDb
    .collection("strategy_stats")
    .find(statsFilter)
    .sort({ timestamp: 1 })
    .toArray();

  console.log(`\n═══ Quant Agent — strategy_stats (${strategyStats.length} docs) ═══`);
  for (const s of strategyStats) {
    console.log(
      `  [${s.timestamp?.toISOString?.() ?? s.timestamp}] ${s.strategy}` +
        `  TVL=$${s.tvl?.toFixed(2)}  PnL=$${s.pnl?.toFixed(2)}` +
        `  APR=${(s.apr * 100)?.toFixed(1)}%  DD=${(s.max_drawdown * 100)?.toFixed(1)}%`
    );
  }

  // ───────────────────────────────────────────────────────
  // 2. Hedgefund Agent — portfolio_stats (portfolio-level timeseries)
  // ───────────────────────────────────────────────────────
  //
  // Schema:
  //   {
  //     timestamp:      Date,
  //     wallet:         String,      // hex address
  //     chain:          String,      // e.g. "hyperevm"
  //     portfolio_tvl:  Number,
  //     strategies: [{
  //       name:            String,
  //       tvl:             Number,
  //       initial_capital: Number,
  //       peak_tvl:        Number,
  //       pnl:             Number,
  //       last_tick:       String,
  //       funding_pnl:     Number,
  //       interest_pnl:    Number,
  //       rewards_pnl:     Number,
  //       costs:           Number,
  //     }],
  //     vaults: [{
  //       name:                String,
  //       strategy:            String,
  //       address:             String,
  //       status:              String,
  //       total_assets:        Number,
  //       idle_balance:        Number,
  //       reserve_ratio:       Number,
  //       reserve_health:      String,
  //       our_position_value:  Number,
  //     }],
  //   }
  //
  const hfDb = client.db("hedgefund-agent");

  const portfolioStats = await hfDb
    .collection("portfolio_stats")
    .find({ timestamp: { $gte: since } })
    .sort({ timestamp: 1 })
    .toArray();

  console.log(`\n═══ Hedgefund Agent — portfolio_stats (${portfolioStats.length} docs) ═══`);
  for (const p of portfolioStats) {
    console.log(
      `  [${p.timestamp?.toISOString?.() ?? p.timestamp}]` +
        `  TVL=$${p.portfolio_tvl?.toFixed(2)}  wallet=${p.wallet?.slice(0, 10)}…`
    );
    for (const s of p.strategies ?? []) {
      console.log(`    ├─ ${s.name}: TVL=$${s.tvl?.toFixed(2)} PnL=$${s.pnl?.toFixed(2)}`);
    }
    for (const v of p.vaults ?? []) {
      console.log(
        `    └─ vault ${v.name}: assets=$${v.total_assets?.toFixed(2)}` +
          ` idle=$${v.idle_balance?.toFixed(2)} reserve=${(v.reserve_ratio * 100)?.toFixed(1)}%`
      );
    }
  }

  // ───────────────────────────────────────────────────────
  // 3. Agent Reasoning Logs (both agents)
  // ───────────────────────────────────────────────────────
  //
  // Schema (reasoning collection):
  //   {
  //     _source_file: String,     // original JSONL filename
  //     _shipped_at:  Date,       // when shipped to mongo
  //     _agent:       String,     // "quant-agent" | "hedgefund-agent"
  //     type:         String,     // "user", "assistant", "tool_use", "tool_result", etc.
  //     content:      Mixed,      // message content (secrets redacted)
  //     ...other JSONL fields
  //   }
  //
  // Schema (sessions collection):
  //   {
  //     file:          String,
  //     source_dir:    String,
  //     shipped_at:    Date,
  //     message_count: Number,
  //     agent:         String,
  //   }
  //

  for (const [dbName, db] of [["quant-agent", quantDb], ["hedgefund-agent", hfDb]]) {
    const sessions = await db
      .collection("sessions")
      .find({ shipped_at: { $gte: since } })
      .sort({ shipped_at: -1 })
      .limit(5)
      .toArray();

    console.log(`\n═══ ${dbName} — recent sessions (${sessions.length}) ═══`);
    for (const s of sessions) {
      console.log(
        `  [${s.shipped_at?.toISOString?.() ?? s.shipped_at}]` +
          ` ${s.file} (${s.message_count} msgs)`
      );
    }

    // Get last 10 assistant reasoning messages
    const reasoning = await db
      .collection("reasoning")
      .find({ type: "assistant", _shipped_at: { $gte: since } })
      .sort({ _shipped_at: -1 })
      .limit(10)
      .toArray();

    console.log(`\n═══ ${dbName} — recent reasoning (${reasoning.length}) ═══`);
    for (const r of reasoning) {
      const text =
        typeof r.content === "string"
          ? r.content.slice(0, 120)
          : JSON.stringify(r.content)?.slice(0, 120);
      console.log(`  [${r._shipped_at?.toISOString?.() ?? "?"}] ${text}…`);
    }
  }

  // ───────────────────────────────────────────────────────
  // 4. Aggregation: daily TVL + PnL timeseries
  // ───────────────────────────────────────────────────────

  const dailyTvl = await quantDb
    .collection("strategy_stats")
    .aggregate([
      { $match: { timestamp: { $gte: since } } },
      {
        $group: {
          _id: {
            strategy: "$strategy",
            day: { $dateToString: { format: "%Y-%m-%d", date: "$timestamp" } },
          },
          avg_tvl: { $avg: "$tvl" },
          max_tvl: { $max: "$tvl" },
          last_pnl: { $last: "$pnl" },
          last_apr: { $last: "$apr" },
          samples: { $sum: 1 },
        },
      },
      { $sort: { "_id.day": 1 } },
    ])
    .toArray();

  console.log(`\n═══ Daily TVL Aggregation (${dailyTvl.length} days) ═══`);
  for (const d of dailyTvl) {
    console.log(
      `  ${d._id.day} | ${d._id.strategy}` +
        `  avg_tvL=$${d.avg_tvl?.toFixed(2)} max=$${d.max_tvl?.toFixed(2)}` +
        `  PnL=$${d.last_pnl?.toFixed(2)} APR=${(d.last_apr * 100)?.toFixed(1)}%` +
        `  (${d.samples} samples)`
    );
  }
} finally {
  await client.close();
}
