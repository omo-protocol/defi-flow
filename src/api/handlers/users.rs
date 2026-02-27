use alloy::hex;
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::auth;
use crate::api::error::ApiError;
use crate::api::state::AppState;

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub ok: bool,
    pub username: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, ApiError> {
    if req.username.len() < 3 || req.username.len() > 32 {
        return Err(ApiError::BadRequest("Username must be 3-32 characters".into()));
    }
    if req.password.len() < 8 {
        return Err(ApiError::BadRequest("Password must be at least 8 characters".into()));
    }

    let password_hash = auth::hash_password(&req.password)
        .map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    let key_salt = auth::generate_salt();
    let user_id = Uuid::new_v4().to_string();

    let db = state.inner.read().await;
    let db = db.db.lock().await;

    // Check uniqueness
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM users WHERE username = ?1",
            [&req.username],
            |row| row.get(0),
        )
        .ok();

    if existing.is_some() {
        return Err(ApiError::Conflict("Username already taken".into()));
    }

    db.execute(
        "INSERT INTO users (id, username, password_hash, key_salt) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![user_id, req.username, password_hash, key_salt],
    )
    .map_err(|e| ApiError::Internal(format!("db insert: {e}")))?;

    Ok(Json(RegisterResponse {
        ok: true,
        username: req.username,
    }))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginUser {
    pub id: String,
    pub username: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: LoginUser,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let row: Option<(String, String, String, String)> = db
        .query_row(
            "SELECT id, username, password_hash, key_salt FROM users WHERE username = ?1",
            [&req.username],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();

    let (user_id, username, password_hash, key_salt) =
        row.ok_or_else(|| ApiError::Unauthorized("Invalid username or password".into()))?;

    if !auth::verify_password(&req.password, &password_hash) {
        return Err(ApiError::Unauthorized("Invalid username or password".into()));
    }

    // Derive and cache the encryption key for wallet operations
    let derived_key = auth::derive_key(&req.password, &key_salt)
        .map_err(|e| ApiError::Internal(format!("key derivation: {e:#}")))?;
    let derived_key_hex = hex::encode(derived_key);

    db.execute(
        "UPDATE users SET derived_key = ?1 WHERE id = ?2",
        rusqlite::params![derived_key_hex, user_id],
    )
    .map_err(|e| ApiError::Internal(format!("caching derived key: {e}")))?;

    let token = auth::create_jwt(&user_id, &inner.auth_secret)
        .map_err(|e| ApiError::Internal(format!("jwt: {e:#}")))?;

    Ok(Json(LoginResponse {
        token,
        user: LoginUser {
            id: user_id,
            username,
        },
    }))
}
