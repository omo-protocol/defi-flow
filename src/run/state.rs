use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Persistent state for the run command, saved as JSON between restarts.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RunState {
    /// Whether the deploy phase has completed (skip on restart).
    pub deploy_completed: bool,
    /// Unix timestamp of the last tick.
    pub last_tick: u64,
    /// Per-node USD balance tracking.
    pub balances: HashMap<String, f64>,
}

impl RunState {
    /// Load state from file, or create a fresh state if the file doesn't exist.
    pub fn load_or_new(path: &Path) -> Result<Self> {
        if path.exists() {
            let contents =
                std::fs::read_to_string(path).context("reading state file")?;
            let state: RunState =
                serde_json::from_str(&contents).context("parsing state file")?;
            println!("Loaded state from {} (deploy_completed={})", path.display(), state.deploy_completed);
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

    pub fn get_balance(&self, node_id: &str) -> f64 {
        self.balances.get(node_id).copied().unwrap_or(0.0)
    }

    pub fn add_balance(&mut self, node_id: &str, amount: f64) {
        let entry = self.balances.entry(node_id.to_string()).or_insert(0.0);
        *entry += amount;
    }

    pub fn deduct_balance(&mut self, node_id: &str, amount: f64) {
        let entry = self.balances.entry(node_id.to_string()).or_insert(0.0);
        *entry = (*entry - amount).max(0.0);
    }
}
