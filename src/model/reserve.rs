use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::chain::Chain;

/// Configuration for vault reserve management.
///
/// When present in a workflow, the daemon monitors the vault's reserve ratio
/// on each tick. If the reserve drops below `trigger_threshold`, it unwinds
/// venue positions pro-rata and transfers freed capital to the vault to
/// restore the reserve to `target_ratio`.
///
/// This is optional — strategies without vaults omit this field entirely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReserveConfig {
    /// Contracts manifest key for the target vault (e.g. "morpho_usdc_vault").
    /// Must have a matching entry in the workflow `contracts` manifest for `vault_chain`.
    pub vault_address: String,

    /// Chain where the vault lives.
    pub vault_chain: Chain,

    /// Token symbol to withdraw and send to vault (e.g. "USDC").
    pub vault_token: String,

    /// Target reserve ratio (0.0–1.0). Default: 0.20 (20% of vault TVL kept idle).
    #[serde(default = "default_target_ratio")]
    pub target_ratio: f64,

    /// Trigger threshold (0.0–1.0). Default: 0.05 (5%).
    /// Unwinding only happens when reserve drops below this ratio.
    #[serde(default = "default_trigger_threshold")]
    pub trigger_threshold: f64,

    /// Minimum deficit (USD) to trigger an unwind. Default: 100.0.
    /// Prevents dust-level unwinds.
    #[serde(default = "default_min_unwind")]
    pub min_unwind: f64,
}

fn default_target_ratio() -> f64 {
    0.20
}
fn default_trigger_threshold() -> f64 {
    0.05
}
fn default_min_unwind() -> f64 {
    100.0
}
