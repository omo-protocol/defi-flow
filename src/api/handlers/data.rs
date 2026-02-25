use axum::extract::{Multipart, State};
use axum::Json;
use serde::Deserialize;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{DataFileEntry, DataManifestResponse};
use crate::model::workflow::Workflow;

#[derive(Deserialize)]
pub struct FetchDataRequest {
    pub workflow: Workflow,
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default = "default_interval")]
    pub interval: String,
    /// Override output directory (default: data/<slugified-workflow-name>)
    pub output_dir: Option<String>,
}

fn default_days() -> u32 {
    365
}
fn default_interval() -> String {
    "8h".to_string()
}

/// POST /api/data/fetch — fetch historical data for a workflow from venue APIs
pub async fn fetch_data(
    State(state): State<AppState>,
    Json(req): Json<FetchDataRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Validate first
    if let Err(errs) = crate::validate::validate(&req.workflow) {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        return Err(ApiError::Validation(msgs));
    }

    let state_inner = state.inner.read().await;
    let base_data_dir = state_inner.data_dir.clone();
    drop(state_inner);

    let output_dir = req
        .output_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let slug = slugify(&req.workflow.name);
            base_data_dir.join("data").join(slug)
        });

    let workflow = req.workflow;
    let days = req.days;
    let interval = req.interval;

    let result = tokio::task::spawn_blocking(move || {
        // Write workflow to temp file (fetch_data::run needs a path)
        let tmp_dir = std::env::temp_dir().join("defi-flow-api");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| ApiError::Internal(format!("creating temp dir: {e}")))?;
        let workflow_path = tmp_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &workflow_path,
            serde_json::to_string_pretty(&workflow).unwrap(),
        )
        .map_err(|e| ApiError::Internal(format!("writing workflow: {e}")))?;

        let result = crate::fetch_data::run(&workflow_path, &output_dir, days, &interval)
            .map_err(|e| ApiError::Internal(format!("fetch-data failed: {:#}", e)));

        let _ = std::fs::remove_file(&workflow_path);

        result.map(|_| output_dir)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))?;

    let output_dir = result?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "data_dir": output_dir.to_string_lossy(),
    })))
}

pub async fn upload_data(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let state_inner = state.inner.read().await;
    let data_dir = state_inner.data_dir.join("data").join("uploads");
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
        "data_dir": data_dir.to_string_lossy(),
    })))
}

pub async fn get_manifest(
    State(state): State<AppState>,
) -> Result<Json<DataManifestResponse>, ApiError> {
    let state_inner = state.inner.read().await;
    let base = state_inner.data_dir.join("data");
    drop(state_inner);

    let mut files = Vec::new();
    collect_files_recursive(&base, &base, &mut files);
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(DataManifestResponse { files }))
}

fn collect_files_recursive(
    root: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<DataFileEntry>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files);
        } else if let Ok(meta) = entry.metadata() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            files.push(DataFileEntry {
                name: rel,
                size: meta.len(),
            });
        }
    }
}

/// Slugify a workflow name for use as a directory name.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}
