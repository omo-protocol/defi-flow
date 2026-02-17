use serde::{Deserialize, Serialize};

/// Pendle market data row.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PendleCsvRow {
    pub timestamp: u64,
    pub pt_price: f64,
    pub yt_price: f64,
    pub implied_apy: f64,
    pub underlying_price: f64,
    pub maturity: u64,
}

pub fn default_pendle_row() -> PendleCsvRow {
    PendleCsvRow {
        timestamp: 0,
        pt_price: 0.95,
        yt_price: 0.05,
        implied_apy: 0.10,
        underlying_price: 1.0,
        maturity: u64::MAX,
    }
}
