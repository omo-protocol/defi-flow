use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

pub type Db = Arc<Mutex<Connection>>;

pub fn open(path: &std::path::Path) -> Result<(Db, String)> {
    std::fs::create_dir_all(path.parent().unwrap_or(path))
        .context("creating db directory")?;

    let conn = Connection::open(path)
        .with_context(|| format!("opening sqlite at {}", path.display()))?;

    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    migrate(&conn)?;
    let secret = ensure_auth_secret(&conn)?;

    Ok((Arc::new(Mutex::new(conn)), secret))
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
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
        ",
    )?;
    Ok(())
}

fn ensure_auth_secret(conn: &Connection) -> Result<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'auth_secret'",
            [],
            |row| row.get(0),
        )
        .ok();

    if let Some(secret) = existing {
        return Ok(secret);
    }

    use rand::Rng;
    let bytes: [u8; 64] = rand::rng().random();
    let secret = base64_url_encode(&bytes);

    conn.execute(
        "INSERT INTO config (key, value) VALUES ('auth_secret', ?1)",
        [&secret],
    )?;

    Ok(secret)
}

fn base64_url_encode(data: &[u8]) -> String {
    use base64_engine::*;
    ENGINE.encode(data)
}

mod base64_engine {
    use std::fmt::Write;
    pub struct Base64Url;
    pub const ENGINE: Base64Url = Base64Url;
    impl Base64Url {
        pub fn encode(&self, data: &[u8]) -> String {
            let mut s = String::new();
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            for chunk in data.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
                let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
                let n = (b0 << 16) | (b1 << 8) | b2;
                let _ = write!(s, "{}", alphabet[((n >> 18) & 63) as usize] as char);
                let _ = write!(s, "{}", alphabet[((n >> 12) & 63) as usize] as char);
                if chunk.len() > 1 {
                    let _ = write!(s, "{}", alphabet[((n >> 6) & 63) as usize] as char);
                }
                if chunk.len() > 2 {
                    let _ = write!(s, "{}", alphabet[(n & 63) as usize] as char);
                }
            }
            s
        }
    }
}
