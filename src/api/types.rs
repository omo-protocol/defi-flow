use serde::{Deserialize, Serialize};

use crate::backtest::result::BacktestResult;
use crate::model::workflow::Workflow;

// ── Request types ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ValidateRequest {
    pub workflow: Workflow,
    #[serde(default)]
    pub check_onchain: bool,
}

#[derive(Deserialize)]
pub struct BacktestRequest {
    pub workflow: Workflow,
    #[serde(default = "default_capital")]
    pub capital: f64,
    #[serde(default = "default_slippage")]
    pub slippage_bps: f64,
    #[serde(default = "default_seed")]
    pub seed: u64,
    pub data_dir: Option<String>,
    pub monte_carlo: Option<u32>,
}

fn default_capital() -> f64 {
    10_000.0
}
fn default_slippage() -> f64 {
    10.0
}
fn default_seed() -> u64 {
    42
}

#[derive(Deserialize)]
pub struct RunStartRequest {
    pub workflow: Workflow,
    #[serde(default = "default_network")]
    pub network: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_slippage")]
    pub slippage_bps: f64,
}

fn default_network() -> String {
    "testnet".to_string()
}

// ── Response types ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ValidateResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct BacktestResponse {
    pub id: String,
    pub result: BacktestResult,
}

#[derive(Serialize)]
pub struct RunStartResponse {
    pub session_id: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct RunStatusResponse {
    pub session_id: String,
    pub status: String,
    pub tvl: f64,
    pub started_at: u64,
    pub network: String,
    pub dry_run: bool,
    pub workflow_name: String,
}

#[derive(Serialize)]
pub struct RunListEntry {
    pub session_id: String,
    pub workflow_name: String,
    pub status: String,
    pub network: String,
    pub started_at: u64,
}

#[derive(Serialize)]
pub struct BacktestSummary {
    pub id: String,
    pub label: String,
    pub twrr_pct: f64,
    pub sharpe: f64,
    pub max_drawdown_pct: f64,
    pub created_at: u64,
}

#[derive(Serialize)]
pub struct BacktestRecord {
    pub id: String,
    pub workflow: Workflow,
    pub result: BacktestResult,
    pub created_at: u64,
}

#[derive(Serialize)]
pub struct DataManifestResponse {
    pub files: Vec<DataFileEntry>,
}

#[derive(Serialize)]
pub struct DataFileEntry {
    pub name: String,
    pub size: u64,
}
