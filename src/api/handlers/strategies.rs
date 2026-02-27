use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::error::ApiError;
use crate::api::middleware::AuthUser;
use crate::api::state::AppState;

#[derive(Serialize)]
pub struct StrategyListItem {
    pub id: String,
    pub name: String,
    pub wallet_id: Option<String>,
    pub wallet_label: Option<String>,
    pub wallet_address: Option<String>,
    pub updated_at: i64,
    pub created_at: i64,
}

pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<StrategyListItem>>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let mut stmt = db
        .prepare(
            "SELECT s.id, s.name, s.wallet_id, w.label, w.address, s.updated_at, s.created_at
             FROM strategies s
             LEFT JOIN wallets w ON s.wallet_id = w.id
             WHERE s.user_id = ?1
             ORDER BY s.updated_at DESC",
        )
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let items = stmt
        .query_map([&auth.user_id], |row| {
            Ok(StrategyListItem {
                id: row.get(0)?,
                name: row.get(1)?,
                wallet_id: row.get(2)?,
                wallet_label: row.get(3)?,
                wallet_address: row.get(4)?,
                updated_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| ApiError::Internal(format!("{e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(items))
}

#[derive(Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    pub workflow_json: serde_json::Value,
    pub wallet_id: Option<String>,
}

#[derive(Serialize)]
pub struct CreateStrategyResponse {
    pub id: String,
    pub name: String,
    pub wallet_id: Option<String>,
}

pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateStrategyRequest>,
) -> Result<Json<CreateStrategyResponse>, ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("Name is required".into()));
    }

    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    // Verify wallet belongs to user
    if let Some(ref wid) = req.wallet_id {
        let exists: Option<String> = db
            .query_row(
                "SELECT id FROM wallets WHERE id = ?1 AND user_id = ?2",
                rusqlite::params![wid, auth.user_id],
                |row| row.get(0),
            )
            .ok();
        if exists.is_none() {
            return Err(ApiError::BadRequest("Wallet not found".into()));
        }
    }

    let id = Uuid::new_v4().to_string();
    let workflow_str = serde_json::to_string(&req.workflow_json)
        .map_err(|e| ApiError::BadRequest(format!("invalid workflow_json: {e}")))?;

    db.execute(
        "INSERT INTO strategies (id, user_id, wallet_id, name, workflow_json) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, auth.user_id, req.wallet_id, req.name, workflow_str],
    )
    .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(CreateStrategyResponse {
        id,
        name: req.name,
        wallet_id: req.wallet_id,
    }))
}

#[derive(Serialize)]
pub struct StrategyDetail {
    pub id: String,
    pub name: String,
    pub wallet_id: Option<String>,
    pub wallet_label: Option<String>,
    pub wallet_address: Option<String>,
    pub workflow_json: serde_json::Value,
    pub updated_at: i64,
    pub created_at: i64,
}

pub async fn get_one(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StrategyDetail>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let row = db
        .query_row(
            "SELECT s.id, s.name, s.wallet_id, w.label, w.address, s.workflow_json, s.updated_at, s.created_at
             FROM strategies s
             LEFT JOIN wallets w ON s.wallet_id = w.id
             WHERE s.id = ?1 AND s.user_id = ?2",
            rusqlite::params![id, auth.user_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .map_err(|_| ApiError::NotFound("Strategy not found".into()))?;

    let workflow_json: serde_json::Value =
        serde_json::from_str(&row.5).unwrap_or(serde_json::Value::Null);

    Ok(Json(StrategyDetail {
        id: row.0,
        name: row.1,
        wallet_id: row.2,
        wallet_label: row.3,
        wallet_address: row.4,
        workflow_json,
        updated_at: row.6,
        created_at: row.7,
    }))
}

#[derive(Deserialize)]
pub struct UpdateStrategyRequest {
    pub name: Option<String>,
    pub workflow_json: Option<serde_json::Value>,
    pub wallet_id: Option<Option<String>>, // None = don't change, Some(None) = clear, Some(Some(id)) = set
}

pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStrategyRequest>,
) -> Result<Json<super::wallets::OkResponse>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    // Verify ownership
    let exists: Option<String> = db
        .query_row(
            "SELECT id FROM strategies WHERE id = ?1 AND user_id = ?2",
            rusqlite::params![id, auth.user_id],
            |row| row.get(0),
        )
        .ok();
    if exists.is_none() {
        return Err(ApiError::NotFound("Strategy not found".into()));
    }

    // Verify wallet if being changed
    if let Some(Some(ref wid)) = req.wallet_id {
        let w_exists: Option<String> = db
            .query_row(
                "SELECT id FROM wallets WHERE id = ?1 AND user_id = ?2",
                rusqlite::params![wid, auth.user_id],
                |row| row.get(0),
            )
            .ok();
        if w_exists.is_none() {
            return Err(ApiError::BadRequest("Wallet not found".into()));
        }
    }

    // Build dynamic update
    let mut sets = vec!["updated_at = unixepoch()".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref name) = req.name {
        sets.push(format!("name = ?{}", params.len() + 1));
        params.push(Box::new(name.clone()));
    }
    if let Some(ref wj) = req.workflow_json {
        let s = serde_json::to_string(wj).unwrap_or_default();
        sets.push(format!("workflow_json = ?{}", params.len() + 1));
        params.push(Box::new(s));
    }
    if let Some(ref wid) = req.wallet_id {
        sets.push(format!("wallet_id = ?{}", params.len() + 1));
        params.push(Box::new(wid.clone()));
    }

    params.push(Box::new(id.clone()));
    params.push(Box::new(auth.user_id.clone()));

    let sql = format!(
        "UPDATE strategies SET {} WHERE id = ?{} AND user_id = ?{}",
        sets.join(", "),
        params.len() - 1,
        params.len(),
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    db.execute(&sql, param_refs.as_slice())
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(super::wallets::OkResponse { ok: true }))
}

pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<super::wallets::OkResponse>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let affected = db
        .execute(
            "DELETE FROM strategies WHERE id = ?1 AND user_id = ?2",
            rusqlite::params![id, auth.user_id],
        )
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    if affected == 0 {
        return Err(ApiError::NotFound("Strategy not found".into()));
    }

    Ok(Json(super::wallets::OkResponse { ok: true }))
}
