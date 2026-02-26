use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, State};

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{BacktestRequest, BacktestResponse, BacktestSummary};
use crate::backtest;

/// Slugify a workflow name for use as a directory name.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

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
    let base_data_dir = state_inner.data_dir.clone();
    drop(state_inner);

    // Resolve data dir: explicit > auto-resolved from workflow name
    let data_dir = if let Some(ref dir) = req.data_dir {
        PathBuf::from(dir)
    } else {
        // Auto-resolve: ~/.defi-flow/data/<slugified-name>
        let slug = slugify(&req.workflow.name);
        base_data_dir.join("data").join(&slug)
    };

    let workflow = req.workflow.clone();
    let auto_fetch = req.auto_fetch;
    let capital = req.capital;
    let slippage_bps = req.slippage_bps;
    let seed = req.seed;
    let monte_carlo = req.monte_carlo;
    let data_dir_clone = data_dir.clone();

    // Run backtest in blocking task
    let (historical, mc_output) = tokio::task::spawn_blocking(move || {
        let tmp_dir = std::env::temp_dir().join("defi-flow-api");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| ApiError::Internal(format!("creating temp dir: {e}")))?;
        let workflow_path = tmp_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &workflow_path,
            serde_json::to_string_pretty(&workflow).unwrap(),
        )
        .map_err(|e| ApiError::Internal(format!("writing workflow: {e}")))?;

        // Auto-fetch data if manifest doesn't exist
        let manifest_path = data_dir_clone.join("manifest.json");
        if !manifest_path.exists() {
            if auto_fetch {
                eprintln!(
                    "[api] No data at {}, auto-fetching...",
                    data_dir_clone.display()
                );
                crate::fetch_data::run(&workflow_path, &data_dir_clone, 365, "8h")
                    .map_err(|e| ApiError::Internal(format!("auto-fetch failed: {:#}", e)))?;
            } else {
                return Err(ApiError::BadRequest(format!(
                    "No data at {}. Set auto_fetch=true or run fetch-data first.",
                    data_dir_clone.display()
                )));
            }
        }

        let mc_config =
            monte_carlo.map(|n| backtest::monte_carlo::MonteCarloConfig { n_simulations: n });

        let config = backtest::BacktestConfig {
            workflow_path: workflow_path.clone(),
            data_dir: data_dir_clone,
            capital,
            slippage_bps,
            seed,
            verbose: false,
            output: None,
            tick_csv: None,
            monte_carlo: mc_config,
        };

        // Run historical backtest
        let historical = backtest::run_single_backtest(&config)
            .map_err(|e| ApiError::Internal(format!("backtest failed: {:#}", e)))?;

        // Run Monte Carlo if requested
        let mc_output = if let Some(ref mc_cfg) = config.monte_carlo {
            let mc_result = backtest::monte_carlo::run(&config, mc_cfg, historical.clone())
                .map_err(|e| ApiError::Internal(format!("monte carlo failed: {:#}", e)))?;
            Some(backtest::MonteCarloOutput {
                n_simulations: mc_result.simulations.len(),
                simulations: mc_result.simulations,
            })
        } else {
            None
        };

        let _ = std::fs::remove_file(&workflow_path);
        Ok((historical, mc_output))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    // Persist to history
    {
        let state_inner = state.inner.read().await;
        if let Err(e) = state_inner
            .history
            .save_backtest(&id, &historical, &req.workflow)
        {
            eprintln!("[api] warning: failed to save backtest history: {:#}", e);
        }
    }

    Ok(Json(BacktestResponse {
        id,
        result: historical,
        monte_carlo: mc_output,
    }))
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
