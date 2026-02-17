use serde::{Deserialize, Serialize};

/// Options data row — same format as markowitz's OptionsCsvRow.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OptionsCsvRow {
    pub snapshot: u64,
    pub timestamp: u64,
    pub spot_price: f64,
    pub asset: String,
    pub address: String,
    pub option_type: String,
    pub strike: f64,
    pub expiry: u64,
    pub days_to_expiry: f64,
    pub premium: f64,
    pub apy: f64,
    pub delta: Option<f64>,
}
