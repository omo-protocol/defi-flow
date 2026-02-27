use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::engine::reserve::{AllocationRecord, ReserveActionRecord};

/// Persistent state for the run command, saved as JSON between restarts.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RunState {
    /// Whether the deploy phase has completed (skip on restart).
    pub deploy_completed: bool,
    /// Unix timestamp of the last tick.
    pub last_tick: u64,
    /// Per-node USD balance tracking.
    pub balances: HashMap<String, f64>,
    /// Audit trail of reserve management actions (deallocate/unwind).
    #[serde(default)]
    pub reserve_actions: Vec<ReserveActionRecord>,
    /// Audit trail of allocation actions (pulling excess from vault).
    #[serde(default)]
    pub allocation_actions: Vec<AllocationRecord>,

    // ── Performance tracking ──
    /// Capital deployed at the end of the deploy phase (sum of all node balances).
    #[serde(default)]
    pub initial_capital: f64,
    /// Highest TVL observed since deploy.
    #[serde(default)]
    pub peak_tvl: f64,
    /// Cumulative funding income from perp venues (USD).
    #[serde(default)]
    pub cumulative_funding: f64,
    /// Cumulative lending interest income (USD).
    #[serde(default)]
    pub cumulative_interest: f64,
    /// Cumulative rewards income — gauge, airdrop, etc. (USD).
    #[serde(default)]
    pub cumulative_rewards: f64,
    /// Cumulative swap/transaction costs (USD).
    #[serde(default)]
    pub cumulative_costs: f64,
}

impl RunState {
    /// Load state from file, or create a fresh state if the file doesn't exist.
    pub fn load_or_new(path: &Path) -> Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path).context("reading state file")?;
            let state: RunState = serde_json::from_str(&contents).context("parsing state file")?;
            println!(
                "Loaded state from {} (deploy_completed={})",
                path.display(),
                state.deploy_completed
            );
            Ok(state)
        } else {
            Ok(RunState::default())
        }
    }

    /// Save state to file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).context("writing state file")?;
        Ok(())
    }
}
