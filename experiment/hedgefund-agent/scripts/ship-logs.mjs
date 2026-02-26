#!/usr/bin/env node
/**
 * Ships OpenClaw session logs (JSONL) to MongoDB.
 *
 * - Scans multiple possible log directories (OpenClaw stores sessions
 *   at ~/.openclaw/agents/<agentId>/sessions/ or ~/.openclaw/sessions/)
 * - Redacts secrets before shipping (API keys, private keys, tokens)
 * - Only moves files to shipped/ AFTER successful MongoDB write
 * - Env: MONGODB_URI, MONGODB_DB (default: from agent name)
 */
import { MongoClient } from "mongodb";
import { readdir, readFile, rename, mkdir, stat } from "fs/promises";
import { join, basename } from "path";

// ── Config ──────────────────────────────────────────────
const MONGODB_URI = process.env.MONGODB_URI;
const DB_NAME = process.env.MONGODB_DB || "agent-logs";

// OpenClaw stores sessions in various paths depending on version/config
const LOG_DIRS = [
  process.env.OPENCLAW_LOGS_DIR,
  "/root/.openclaw/agents/main/sessions",
  "/root/.openclaw/sessions",
  "/root/.openclaw/cron/runs",
].filter(Boolean);

if (!MONGODB_URI) {
  console.log("MONGODB_URI not set — skipping log shipment.");
  process.exit(0);
}

// ── Secret redaction ────────────────────────────────────
const SECRET_PATTERNS = [
  // Anthropic API keys
  { pattern: /sk-ant-[A-Za-z0-9_-]{20,}/g, label: "ANTHROPIC_KEY" },
  // OpenAI API keys
  { pattern: /sk-[A-Za-z0-9]{20,}/g, label: "API_KEY" },
  // Ethereum private keys (0x + 64 hex chars)
  { pattern: /0x[0-9a-fA-F]{64}/g, label: "PRIVATE_KEY" },
  // Generic bearer tokens
  { pattern: /Bearer\s+[A-Za-z0-9_.-]{20,}/gi, label: "BEARER_TOKEN" },
  // GitHub tokens (ghp_, gho_, ghs_, ghr_)
  { pattern: /gh[psr]_[A-Za-z0-9_]{20,}/g, label: "GH_TOKEN" },
  // MongoDB connection strings (redact password)
  {
    pattern: /mongodb(\+srv)?:\/\/[^:]+:[^@]+@/g,
    replace: (match) => match.replace(/:([^@]+)@/, ":[REDACTED]@"),
  },
  // Generic password/secret/key in JSON values
  {
    pattern:
      /"(password|secret|private_key|api_key|auth_token|PRIVATE_KEY|DEFI_FLOW_PRIVATE_KEY|ANTHROPIC_API_KEY|GATEWAY_AUTH_TOKEN|MONGODB_URI)"\s*:\s*"[^"]+"/gi,
    replace: (match, key) => `"${key}": "[REDACTED]"`,
  },
];

function redact(text) {
  let result = text;
  for (const rule of SECRET_PATTERNS) {
    if (rule.replace) {
      result = result.replace(rule.pattern, rule.replace);
    } else {
      result = result.replace(rule.pattern, `[REDACTED:${rule.label}]`);
    }
  }
  return result;
}

// ── Find log files ──────────────────────────────────────
async function findJsonlFiles() {
  const allFiles = [];

  for (const dir of LOG_DIRS) {
    try {
      const s = await stat(dir);
      if (!s.isDirectory()) continue;
      const files = await readdir(dir);
      for (const f of files) {
        if (f.endsWith(".jsonl")) {
          allFiles.push({ dir, file: f, path: join(dir, f) });
        }
      }
    } catch {
      // directory doesn't exist — skip
    }
  }

  return allFiles;
}

// ── Main ────────────────────────────────────────────────
const client = new MongoClient(MONGODB_URI, {
  serverSelectionTimeoutMS: 10_000,
  connectTimeoutMS: 10_000,
});

try {
  // Verify connection
  await client.connect();
  await client.db("admin").command({ ping: 1 });
  console.log(`Connected to MongoDB (db: ${DB_NAME})`);

  const db = client.db(DB_NAME);
  const sessions = db.collection("sessions");
  const reasoning = db.collection("reasoning");

  // Ensure indexes (idempotent)
  await sessions.createIndex({ file: 1 }, { unique: true }).catch(() => {});
  await reasoning.createIndex({ _source_file: 1, _shipped_at: 1 });
  await reasoning.createIndex({ "message.role": 1 });
  await reasoning.createIndex({ type: 1, ts: 1 });

  const logFiles = await findJsonlFiles();

  if (logFiles.length === 0) {
    console.log("No session logs to ship.");
    process.exit(0);
  }

  let shipped = 0;
  let totalDocs = 0;

  for (const { dir, file, path } of logFiles) {
    // Skip if already shipped (check MongoDB)
    const existing = await sessions.findOne({ file });
    if (existing) {
      console.log(`  SKIP ${file} (already shipped)`);
      // Move to shipped if still in source dir
      const shippedDir = join(dir, "shipped");
      await mkdir(shippedDir, { recursive: true }).catch(() => {});
      await rename(path, join(shippedDir, file)).catch(() => {});
      continue;
    }

    const rawContent = await readFile(path, "utf-8");

    // Redact secrets BEFORE parsing
    const content = redact(rawContent);

    const lines = content.trim().split("\n").filter(Boolean);
    const docs = [];

    for (const line of lines) {
      try {
        const parsed = JSON.parse(line);
        parsed._source_file = file;
        parsed._shipped_at = new Date();
        parsed._agent = DB_NAME;
        docs.push(parsed);
      } catch {
        // skip malformed lines
      }
    }

    if (docs.length === 0) continue;

    // Write to MongoDB — session metadata + reasoning entries
    // Use a transaction-like approach: write docs first, then metadata, then move file
    try {
      await reasoning.insertMany(docs, { ordered: false });
      await sessions.insertOne({
        file,
        source_dir: dir,
        shipped_at: new Date(),
        message_count: docs.length,
        agent: DB_NAME,
      });

      // Only move file AFTER successful write
      const shippedDir = join(dir, "shipped");
      await mkdir(shippedDir, { recursive: true }).catch(() => {});
      await rename(path, join(shippedDir, file));

      shipped++;
      totalDocs += docs.length;
      console.log(`  OK ${file} (${docs.length} entries)`);
    } catch (writeErr) {
      // Don't move file if write failed — retry next time
      console.error(`  FAIL ${file}: ${writeErr.message}`);
    }
  }

  console.log(
    `Shipped ${shipped} files (${totalDocs} entries) to MongoDB [${DB_NAME}].`
  );
} catch (err) {
  console.error("Log shipment failed:", err.message);
  process.exit(1);
} finally {
  await client.close();
}
