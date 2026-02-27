import Database from "better-sqlite3";
import path from "path";
import crypto from "crypto";

const DB_PATH = path.join(process.cwd(), "data", "defi-flow.db");

let db: Database.Database | null = null;

export function getDb(): Database.Database {
  if (db) return db;

  db = new Database(DB_PATH);
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");

  migrate(db);
  ensureAuthSecret(db);

  return db;
}

function migrate(db: Database.Database) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS users (
      id             TEXT PRIMARY KEY,
      username       TEXT UNIQUE NOT NULL,
      password_hash  TEXT NOT NULL,
      key_salt       TEXT NOT NULL,
      derived_key    TEXT,
      created_at     INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE IF NOT EXISTS wallets (
      id            TEXT PRIMARY KEY,
      user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      label         TEXT NOT NULL,
      address       TEXT NOT NULL,
      encrypted_pk  TEXT NOT NULL,
      created_at    INTEGER DEFAULT (unixepoch()),
      UNIQUE(user_id, address)
    );

    CREATE TABLE IF NOT EXISTS strategies (
      id            TEXT PRIMARY KEY,
      user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      wallet_id     TEXT REFERENCES wallets(id) ON DELETE SET NULL,
      name          TEXT NOT NULL,
      workflow_json TEXT NOT NULL,
      updated_at    INTEGER DEFAULT (unixepoch()),
      created_at    INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE IF NOT EXISTS user_config (
      user_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      key      TEXT NOT NULL,
      value    TEXT NOT NULL,
      PRIMARY KEY (user_id, key)
    );

    CREATE TABLE IF NOT EXISTS config (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
  `);
}

function ensureAuthSecret(db: Database.Database) {
  const row = db.prepare("SELECT value FROM config WHERE key = 'auth_secret'").get() as
    | { value: string }
    | undefined;

  if (!row) {
    const secret = crypto.randomBytes(64).toString("base64url");
    db.prepare("INSERT INTO config (key, value) VALUES ('auth_secret', ?)").run(secret);
  }
}

export function getAuthSecret(): string {
  const envSecret = process.env.AUTH_SECRET;
  if (envSecret) return envSecret;

  const db = getDb();
  const row = db.prepare("SELECT value FROM config WHERE key = 'auth_secret'").get() as {
    value: string;
  };
  return row.value;
}
