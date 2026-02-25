use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::Json;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{BacktestRequest, BacktestResponse, BacktestSummary};
use crate::backtest;

pub async fn run_backtest(
    State(state): State<AppState>,
    Json(req): Json<BacktestRequest>,
) -> Result<Json<BacktestResponse>, ApiError> {
    let id = uuid::Uuid::new_v4().to_string();

    // Validate first
    if let Err(errs) = crate::validate::validate(&req.workflow) {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        return Err(ApiError::Validation(msgs));
    }

    let state_inner = state.inner.read().await;
    let data_dir = req
        .data_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state_inner.data_dir.join("data"));
    drop(state_inner);

    let workflow = req.workflow.clone();
    let capital = req.capital;
    let slippage_bps = req.slippage_bps;
    let seed = req.seed;

    // Run backtest in blocking task (it creates its own tokio runtime internally,
    // so we use spawn_blocking to avoid nesting runtimes)
    let result = tokio::task::spawn_blocking(move || {
        // Write workflow to a temp file so run_single_backtest can load it
        let tmp_dir = std::env::temp_dir().join("defi-flow-api");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| ApiError::Internal(format!("creating temp dir: {e}")))?;
        let workflow_path = tmp_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &workflow_path,
            serde_json::to_string_pretty(&workflow).unwrap(),
        )
        .map_err(|e| ApiError::Internal(format!("writing workflow: {e}")))?;

        let config = backtest::BacktestConfig {
            workflow_path: workflow_path.clone(),
            data_dir,
            capital,
            slippage_bps,
            seed,
            verbose: false,
            output: None,
            tick_csv: None,
            monte_carlo: None,
        };

        let result = backtest::run_single_backtest(&config)
            .map_err(|e| ApiError::Internal(format!("backtest failed: {:#}", e)));

        // Clean up temp file
        let _ = std::fs::remove_file(&workflow_path);

        result
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    // Persist to history
    {
        let state_inner = state.inner.read().await;
        if let Err(e) = state_inner
            .history
            .save_backtest(&id, &result, &req.workflow)
        {
            eprintln!("[api] warning: failed to save backtest history: {:#}", e);
        }
    }

    Ok(Json(BacktestResponse { id, result }))
}

pub async fn list_backtests(
    State(state): State<AppState>,
) -> Result<Json<Vec<BacktestSummary>>, ApiError> {
    let state_inner = state.inner.read().await;
    let list = state_inner
        .history
        .list_backtests()
        .map_err(|e| ApiError::Internal(format!("{:#}", e)))?;
    Ok(Json(list))
}

pub async fn get_backtest(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::api::types::BacktestRecord>, ApiError> {
    let state_inner = state.inner.read().await;
    let record = state_inner
        .history
        .get_backtest(&id)
        .map_err(|_| ApiError::NotFound(format!("backtest '{id}' not found")))?;
    Ok(Json(record))
}
