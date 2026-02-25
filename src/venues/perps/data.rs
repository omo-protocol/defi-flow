use serde::{Deserialize, Serialize};

/// Perp market data row — same format as markowitz's PerpCsvRow.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PerpCsvRow {
    pub symbol: String,
    pub mark_price: f64,
    pub index_price: f64,
    pub funding_rate: f64,
    pub open_interest: f64,
    pub volume_24h: f64,
    pub bid: f64,
    pub ask: f64,
    pub mid_price: f64,
    pub last_price: f64,
    pub premium: f64,
    pub basis: f64,
    pub timestamp: u64,
    pub funding_apy: f64,
    pub rewards_apy: f64,
}

/// Generic price row for spot trading.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PriceCsvRow {
    pub timestamp: u64,
    pub price: f64,
    pub bid: f64,
    pub ask: f64,
}
