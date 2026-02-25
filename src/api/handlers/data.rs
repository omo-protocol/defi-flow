use axum::extract::{Multipart, State};
use axum::Json;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{DataFileEntry, DataManifestResponse};

pub async fn upload_data(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let state_inner = state.inner.read().await;
    let data_dir = state_inner.data_dir.join("data");
    drop(state_inner);

    std::fs::create_dir_all(&data_dir)
        .map_err(|e| ApiError::Internal(format!("creating data dir: {e}")))?;

    let mut uploaded = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    {
        let name = field
            .file_name()
            .unwrap_or("unnamed.csv")
            .to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(format!("reading field: {e}")))?;

        let path = data_dir.join(&name);
        std::fs::write(&path, &bytes)
            .map_err(|e| ApiError::Internal(format!("writing {name}: {e}")))?;

        uploaded.push(name);
    }

    Ok(Json(serde_json::json!({
        "uploaded": uploaded,
        "count": uploaded.len(),
    })))
}

pub async fn get_manifest(
    State(state): State<AppState>,
) -> Result<Json<DataManifestResponse>, ApiError> {
    let state_inner = state.inner.read().await;
    let data_dir = state_inner.data_dir.join("data");
    drop(state_inner);

    let mut files = Vec::new();

    if data_dir.exists() {
        for entry in std::fs::read_dir(&data_dir)
            .map_err(|e| ApiError::Internal(format!("reading data dir: {e}")))?
        {
            let entry = entry.map_err(|e| ApiError::Internal(e.to_string()))?;
            let meta = entry.metadata().map_err(|e| ApiError::Internal(e.to_string()))?;
            if meta.is_file() {
                files.push(DataFileEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    size: meta.len(),
                });
            }
        }
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(DataManifestResponse { files }))
}
