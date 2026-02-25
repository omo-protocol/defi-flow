use serde::{Deserialize, Serialize};

/// Lending market data row.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LendingCsvRow {
    pub timestamp: u64,
    pub supply_apy: f64,
    pub borrow_apy: f64,
    pub utilization: f64,
    pub reward_apy: f64,
}
