use serde::{Deserialize, Serialize};

/// LP pool data row — supports Aerodrome Slipstream concentrated liquidity.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LpCsvRow {
    pub timestamp: u64,
    #[serde(default)]
    pub current_tick: i32,
    pub price_a: f64,
    pub price_b: f64,
    pub fee_apy: f64,
    pub reward_rate: f64,
    pub reward_token_price: f64,
}

pub fn default_lp_row() -> LpCsvRow {
    LpCsvRow {
        timestamp: 0,
        current_tick: 0,
        price_a: 1.0,
        price_b: 1.0,
        fee_apy: 0.0,
        reward_rate: 0.0,
        reward_token_price: 0.0,
    }
}
