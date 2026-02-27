use alloy::hex;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::auth;
use crate::api::error::ApiError;
use crate::api::middleware::AuthUser;
use crate::api::state::AppState;

#[derive(Serialize)]
pub struct WalletInfo {
    pub id: String,
    pub label: String,
    pub address: String,
    pub created_at: i64,
}

pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<WalletInfo>>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let mut stmt = db
        .prepare("SELECT id, label, address, created_at FROM wallets WHERE user_id = ?1 ORDER BY created_at DESC")
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let wallets = stmt
        .query_map([&auth.user_id], |row| {
            Ok(WalletInfo {
                id: row.get(0)?,
                label: row.get(1)?,
                address: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| ApiError::Internal(format!("{e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(wallets))
}

#[derive(Deserialize)]
pub struct CreateWalletRequest {
    pub label: String,
    pub mode: String, // "generate" | "import"
    #[serde(rename = "privateKey")]
    pub private_key: Option<String>,
}

#[derive(Serialize)]
pub struct CreateWalletResponse {
    pub id: String,
    pub label: String,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<String>,
}

pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateWalletRequest>,
) -> Result<Json<CreateWalletResponse>, ApiError> {
    if req.label.is_empty() {
        return Err(ApiError::BadRequest("Label is required".into()));
    }

    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    // Get cached derived key
    let derived_key_hex: String = db
        .query_row(
            "SELECT derived_key FROM users WHERE id = ?1",
            [&auth.user_id],
            |row| row.get(0),
        )
        .map_err(|_| ApiError::Unauthorized("Session expired, please login again".into()))?;

    let mut derived_key = [0u8; 32];
    hex::decode_to_slice(&derived_key_hex, &mut derived_key)
        .map_err(|e| ApiError::Internal(format!("derived key decode: {e}")))?;

    let (address, private_key, mnemonic) = match req.mode.as_str() {
        "generate" => {
            // Generate a random wallet using alloy
            let signer = alloy::signers::local::PrivateKeySigner::random();
            let addr = format!("{:?}", signer.address());
            let pk = format!("0x{}", hex::encode(signer.to_bytes()));
            (addr, pk, None::<String>) // mnemonic not available from random signer
        }
        "import" => {
            let pk = req
                .private_key
                .as_deref()
                .ok_or_else(|| ApiError::BadRequest("privateKey required for import".into()))?;
            let pk_clean = pk.strip_prefix("0x").unwrap_or(pk);
            let pk_bytes: [u8; 32] = hex::decode(pk_clean)
                .map_err(|_| ApiError::BadRequest("Invalid private key format".into()))?
                .try_into()
                .map_err(|_| ApiError::BadRequest("Private key must be 32 bytes".into()))?;
            let signer = alloy::signers::local::PrivateKeySigner::from_bytes(
                &alloy::primitives::B256::from(pk_bytes),
            )
            .map_err(|e| ApiError::BadRequest(format!("Invalid private key: {e}")))?;
            let addr = format!("{:?}", signer.address());
            (addr, pk.to_string(), None)
        }
        _ => return Err(ApiError::BadRequest("mode must be 'generate' or 'import'".into())),
    };

    // Check duplicate
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM wallets WHERE user_id = ?1 AND address = ?2",
            rusqlite::params![auth.user_id, address],
            |row| row.get(0),
        )
        .ok();

    if existing.is_some() {
        return Err(ApiError::Conflict("Wallet already exists".into()));
    }

    let encrypted_pk = auth::encrypt_pk(&private_key, &derived_key)
        .map_err(|e| ApiError::Internal(format!("encryption: {e:#}")))?;

    let wallet_id = Uuid::new_v4().to_string();

    db.execute(
        "INSERT INTO wallets (id, user_id, label, address, encrypted_pk) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![wallet_id, auth.user_id, req.label, address, encrypted_pk],
    )
    .map_err(|e| ApiError::Internal(format!("db insert: {e}")))?;

    Ok(Json(CreateWalletResponse {
        id: wallet_id,
        label: req.label,
        address,
        mnemonic,
    }))
}

#[derive(Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    let inner = state.inner.read().await;
    let db = inner.db.lock().await;

    let affected = db
        .execute(
            "DELETE FROM wallets WHERE id = ?1 AND user_id = ?2",
            rusqlite::params![id, auth.user_id],
        )
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    if affected == 0 {
        return Err(ApiError::NotFound("Wallet not found".into()));
    }

    Ok(Json(OkResponse { ok: true }))
}
