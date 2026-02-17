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

pub fn default_perp_row() -> PerpCsvRow {
    PerpCsvRow {
        symbol: "BTC".to_string(),
        mark_price: 100.0,
        index_price: 100.0,
        funding_rate: 0.0,
        open_interest: 0.0,
        volume_24h: 0.0,
        bid: 99.99,
        ask: 100.01,
        mid_price: 100.0,
        last_price: 100.0,
        premium: 0.0,
        basis: 0.0,
        timestamp: 0,
        funding_apy: 0.0,
        rewards_apy: 0.0,
    }
}

pub fn default_price_row() -> PriceCsvRow {
    PriceCsvRow {
        timestamp: 0,
        price: 1.0,
        bid: 1.0,
        ask: 1.0,
    }
}
