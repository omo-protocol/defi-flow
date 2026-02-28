#!/usr/bin/env node
/**
 * Query MongoDB timeseries data for defi-flow agents.
 *
 * Database naming: quant-{model} | hedgefund-{model}
 * Models: minimax, qwen, kimi, glm, opus, gemini, grok
 *
 * Usage:
 *   node query-mongo.mjs                              # all models, all data
 *   node query-mongo.mjs --model opus                 # single model
 *   node query-mongo.mjs --hours 24                   # last 24h
 *   node query-mongo.mjs --strategy lending           # filter by strategy
 *   node query-mongo.mjs --agent quant                # quant only
 *   node query-mongo.mjs --agent hedgefund            # hedgefund only
 *   node query-mongo.mjs --trace <source_file>        # full session trace
 *   node query-mongo.mjs --costs                      # token usage summary
 *
 * Env:
 *   MONGODB_URI  — connection string (defaults to localhost)
 */

import { MongoClient } from "mongodb";

const MONGODB_URI = process.env.MONGODB_URI || "mongodb://localhost:27017";
const MODELS = ["minimax", "qwen", "kimi", "glm", "opus", "gemini", "grok"];

// ── CLI args ────────────────────────────────────────────

function arg(flag) {
  const i = process.argv.indexOf(flag);
  return i !== -1 ? process.argv[i + 1] : null;
}
function flag(name) {
  return process.argv.includes(name);
}

const hours = arg("--hours") ? Number(arg("--hours")) : null;
const modelFilter = arg("--model"); // e.g. "opus"
const strategyFilter = arg("--strategy");
const agentFilter = arg("--agent"); // "quant" | "hedgefund"
const traceFile = arg("--trace"); // source_file for full trace
const showCosts = flag("--costs");

const since = hours ? new Date(Date.now() - hours * 3600_000) : new Date(0);
const models = modelFilter ? [modelFilter] : MODELS;

const client = new MongoClient(MONGODB_URI);

try {
  await client.connect();

  // If --trace, dump a full session trace and exit
  if (traceFile) {
    await renderTrace(client, traceFile);
    process.exit(0);
  }

  // ───────────────────────────────────────────────────────
  // 1. Strategy Stats (quant-{model} databases)
  // ───────────────────────────────────────────────────────

  if (!agentFilter || agentFilter === "quant") {
    for (const model of models) {
      const dbName = `quant-${model}`;
      const db = client.db(dbName);

      // Check if collection exists (skip empty databases)
      const collections = await db.listCollections({ name: "strategy_stats" }).toArray();
      if (collections.length === 0) continue;

      const filter = { timestamp: { $gte: since } };
      if (strategyFilter) filter.strategy = strategyFilter;

      const stats = await db
        .collection("strategy_stats")
        .find(filter)
        .sort({ timestamp: -1 })
        .limit(20)
        .toArray();

      if (stats.length === 0) continue;

      console.log(`\n═══ ${dbName} — strategy_stats (${stats.length} latest) ═══`);
      for (const s of stats) {
        console.log(
          `  [${ts(s.timestamp)}] ${s.strategy}` +
            `  TVL=$${n(s.tvl)}  PnL=$${n(s.pnl)}` +
            `  APR=${pct(s.apr)}  DD=${pct(s.max_drawdown)}` +
            `  status=${s.status ?? "?"}`
        );
        if (s.balances && Object.keys(s.balances).length > 0) {
          for (const [node, val] of Object.entries(s.balances)) {
            console.log(`    ├─ ${node}: $${n(val)}`);
          }
        }
      }

      // Daily aggregation per model
      const daily = await db
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

      if (daily.length > 0) {
        console.log(`\n  ── daily aggregation (${daily.length} days) ──`);
        for (const d of daily) {
          console.log(
            `    ${d._id.day} | ${d._id.strategy}` +
              `  avg=$${n(d.avg_tvl)} max=$${n(d.max_tvl)}` +
              `  PnL=$${n(d.last_pnl)} APR=${pct(d.last_apr)}` +
              `  (${d.samples} samples)`
          );
        }
      }
    }
  }

  // ───────────────────────────────────────────────────────
  // 2. Portfolio Stats (hedgefund-{model} databases)
  // ───────────────────────────────────────────────────────

  if (!agentFilter || agentFilter === "hedgefund") {
    for (const model of models) {
      const dbName = `hedgefund-${model}`;
      const db = client.db(dbName);

      const collections = await db.listCollections({ name: "portfolio_stats" }).toArray();
      if (collections.length === 0) continue;

      const stats = await db
        .collection("portfolio_stats")
        .find({ timestamp: { $gte: since } })
        .sort({ timestamp: -1 })
        .limit(10)
        .toArray();

      if (stats.length === 0) continue;

      console.log(`\n═══ ${dbName} — portfolio_stats (${stats.length} latest) ═══`);
      for (const p of stats) {
        console.log(
          `  [${ts(p.timestamp)}]` +
            `  TVL=$${n(p.portfolio_tvl)}  wallet=${p.wallet?.slice(0, 10)}…`
        );
        for (const s of p.strategies ?? []) {
          console.log(
            `    ├─ ${s.name}: TVL=$${n(s.tvl)} PnL=$${n(s.pnl)} APR=${pct(s.apr)} [${s.status}]`
          );
        }
        for (const v of p.vaults ?? []) {
          const health = v.status === "healthy" ? "✓" : v.status === "warning" ? "⚠" : "✗";
          console.log(
            `    └─ ${health} vault ${v.name}: assets=$${n(v.total_assets)}` +
              ` idle=$${n(v.idle)}` +
              ` reserve=${pct(v.reserve_ratio)}` +
              ` ours=$${n(v.our_value)}`
          );
        }
      }
    }
  }

  // ───────────────────────────────────────────────────────
  // 3. Reasoning Sessions (all agent databases)
  // ───────────────────────────────────────────────────────

  const agentPrefixes = [];
  if (!agentFilter || agentFilter === "quant") agentPrefixes.push("quant");
  if (!agentFilter || agentFilter === "hedgefund") agentPrefixes.push("hedgefund");

  for (const prefix of agentPrefixes) {
    for (const model of models) {
      const dbName = `${prefix}-${model}`;
      const db = client.db(dbName);

      const collections = await db.listCollections({ name: "sessions" }).toArray();
      if (collections.length === 0) continue;

      // Recent sessions
      const sessions = await db
        .collection("sessions")
        .find({ shipped_at: { $gte: since } })
        .sort({ shipped_at: -1 })
        .limit(5)
        .toArray();

      if (sessions.length === 0) continue;

      console.log(`\n═══ ${dbName} — recent sessions (${sessions.length}) ═══`);
      for (const s of sessions) {
        console.log(
          `  [${ts(s.shipped_at)}] ${s.file} (${s.message_count} msgs)`
        );
      }

      // Recent assistant reasoning
      const reasoning = await db
        .collection("reasoning")
        .find({
          type: "message",
          "message.role": "assistant",
          _shipped_at: { $gte: since },
        })
        .sort({ _shipped_at: -1 })
        .limit(5)
        .toArray();

      if (reasoning.length > 0) {
        console.log(`\n  ── recent reasoning (${reasoning.length}) ──`);
        for (const r of reasoning) {
          const msg = r.message;
          // Extract text content from content blocks
          const text = (msg?.content ?? [])
            .filter((b) => b.type === "text")
            .map((b) => b.text)
            .join(" ")
            .slice(0, 140);
          const tools = (msg?.content ?? [])
            .filter((b) => b.type === "toolCall")
            .map((b) => b.name);
          const toolStr = tools.length > 0 ? ` [${tools.join(", ")}]` : "";
          console.log(`  [${ts(r._shipped_at)}] ${text}…${toolStr}`);
        }
      }
    }
  }

  // ───────────────────────────────────────────────────────
  // 4. Token Usage / Costs (--costs flag)
  // ───────────────────────────────────────────────────────

  if (showCosts) {
    console.log(`\n═══ Token Usage & Costs ═══`);

    for (const prefix of agentPrefixes) {
      for (const model of models) {
        const dbName = `${prefix}-${model}`;
        const db = client.db(dbName);

        const collections = await db.listCollections({ name: "reasoning" }).toArray();
        if (collections.length === 0) continue;

        const costAgg = await db
          .collection("reasoning")
          .aggregate([
            {
              $match: {
                type: "message",
                "message.role": "assistant",
                _shipped_at: { $gte: since },
              },
            },
            {
              $group: {
                _id: null,
                total_input: { $sum: "$message.usage.input" },
                total_output: { $sum: "$message.usage.output" },
                total_cache_read: { $sum: "$message.usage.cacheRead" },
                total_cost: { $sum: "$message.usage.cost.total" },
                messages: { $sum: 1 },
              },
            },
          ])
          .toArray();

        if (costAgg.length === 0) continue;

        const c = costAgg[0];
        console.log(
          `  ${dbName}: ${c.messages} msgs` +
            `  in=${tok(c.total_input)} out=${tok(c.total_output)}` +
            `  cache=${tok(c.total_cache_read)}` +
            `  cost=$${c.total_cost?.toFixed(4) ?? "?"}`
        );
      }
    }
  }
} finally {
  await client.close();
}

// ── Full session trace renderer ─────────────────────────

async function renderTrace(client, sourceFile) {
  // Search across all databases for this source file
  for (const prefix of ["quant", "hedgefund"]) {
    for (const model of MODELS) {
      const dbName = `${prefix}-${model}`;
      const db = client.db(dbName);

      const docs = await db
        .collection("reasoning")
        .find({ _source_file: sourceFile })
        .sort({ timestamp: 1 })
        .toArray();

      if (docs.length === 0) continue;

      console.log(`\n═══ Trace: ${sourceFile} (${dbName}, ${docs.length} docs) ═══\n`);

      for (const doc of docs) {
        if (doc.type === "session") {
          console.log(`── SESSION START ── ${doc.id} at ${doc.timestamp}`);
          console.log(`   cwd: ${doc.cwd}\n`);
        } else if (doc.type === "model_change") {
          console.log(`── MODEL: ${doc.modelId} (${doc.provider}) ──\n`);
        } else if (doc.type === "message") {
          const msg = doc.message;
          if (msg.role === "user") {
            const text = (msg.content ?? [])
              .filter((b) => b.type === "text")
              .map((b) => b.text)
              .join("\n");
            console.log(`USER:\n${indent(text)}\n`);
          } else if (msg.role === "assistant") {
            const texts = (msg.content ?? []).filter((b) => b.type === "text");
            const tools = (msg.content ?? []).filter((b) => b.type === "toolCall");

            if (texts.length > 0) {
              const text = texts.map((b) => b.text).join("\n");
              console.log(`ASSISTANT (${msg.model ?? "?"}):`);
              console.log(indent(text));
            }
            for (const t of tools) {
              console.log(`  TOOL_CALL: ${t.name}(${JSON.stringify(t.arguments).slice(0, 200)})`);
            }
            if (msg.usage) {
              console.log(
                `  tokens: in=${msg.usage.input} out=${msg.usage.output}` +
                  ` cost=$${msg.usage.cost?.total?.toFixed(4) ?? "?"}`
              );
            }
            console.log();
          } else if (msg.role === "toolResult") {
            const text = (msg.content ?? [])
              .filter((b) => b.type === "text")
              .map((b) => b.text)
              .join("\n");
            const status = msg.isError ? "ERROR" : "OK";
            const dur = msg.details?.durationMs ? ` ${msg.details.durationMs}ms` : "";
            console.log(`  TOOL_RESULT [${msg.toolName}] ${status}${dur}:`);
            console.log(indent(text.slice(0, 500), "    "));
            console.log();
          }
        }
      }
      return; // found and rendered
    }
  }
  console.log(`No trace found for source_file: ${sourceFile}`);
}

// ── Helpers ─────────────────────────────────────────────

function ts(d) {
  return d?.toISOString?.() ?? d ?? "?";
}
function n(v) {
  return v?.toFixed?.(2) ?? v ?? "?";
}
function pct(v) {
  return v != null ? `${(v * 100).toFixed(1)}%` : "?";
}
function tok(v) {
  if (v == null) return "?";
  if (v > 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (v > 1_000) return `${(v / 1_000).toFixed(1)}K`;
  return String(v);
}
function indent(text, prefix = "  ") {
  return text
    .split("\n")
    .map((l) => prefix + l)
    .join("\n");
}
