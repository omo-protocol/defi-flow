use serde::{Deserialize, Serialize};

/// Vault yield data row.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VaultCsvRow {
    pub timestamp: u64,
    pub apy: f64,
    pub reward_apy: f64,
}

pub fn default_vault_row() -> VaultCsvRow {
    VaultCsvRow {
        timestamp: 0,
        apy: 0.0,
        reward_apy: 0.0,
    }
}
