use std::collections::HashMap;

use axum::Json;
use axum::extract::State;

use crate::api::error::ApiError;
use crate::api::middleware::AuthUser;
use crate::api::state::AppState;

pub async fn get_config(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<HashMap<String, String>>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let mut stmt = db
        .prepare("SELECT key, value FROM user_config WHERE user_id = ?1")
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let map: HashMap<String, String> = stmt
        .query_map([&auth.user_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| ApiError::Internal(format!("{e}")))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(map))
}

pub async fn update_config(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(updates): Json<HashMap<String, Option<String>>>,
) -> Result<Json<super::wallets::OkResponse>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    for (key, value) in &updates {
        match value {
            Some(v) if !v.is_empty() => {
                db.execute(
                    "INSERT INTO user_config (user_id, key, value) VALUES (?1, ?2, ?3)
                     ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![auth.user_id, key, v],
                )
                .map_err(|e| ApiError::Internal(format!("{e}")))?;
            }
            _ => {
                db.execute(
                    "DELETE FROM user_config WHERE user_id = ?1 AND key = ?2",
                    rusqlite::params![auth.user_id, key],
                )
                .map_err(|e| ApiError::Internal(format!("{e}")))?;
            }
        }
    }

    Ok(Json(super::wallets::OkResponse { ok: true }))
}
