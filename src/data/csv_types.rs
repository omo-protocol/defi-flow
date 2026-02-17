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

/// LP pool data row — supports Aerodrome Slipstream concentrated liquidity.
///
/// For concentrated liquidity pools (Aerodrome Slipstream), `current_tick` tracks
/// the active pool tick. Fee APY is the pool-wide rate; the simulator applies a
/// concentration multiplier based on the position's tick range.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LpCsvRow {
    pub timestamp: u64,
    /// Current pool tick (Uniswap V3 / Slipstream). 0 for full-range / legacy pools.
    #[serde(default)]
    pub current_tick: i32,
    /// Price of token0 in USD.
    pub price_a: f64,
    /// Price of token1 in USD.
    pub price_b: f64,
    /// Pool-wide fee APY (from subgraph: avg_daily_fees / tvl * 365).
    pub fee_apy: f64,
    /// AERO reward emission rate (annualized, e.g. 0.10 = 10%).
    pub reward_rate: f64,
    /// AERO token price in USD.
    pub reward_token_price: f64,
}

/// Lending market data row.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LendingCsvRow {
    pub timestamp: u64,
    pub supply_apy: f64,
    pub borrow_apy: f64,
    pub utilization: f64,
    pub reward_apy: f64,
}

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

/// Generic price row for spot trading.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PriceCsvRow {
    pub timestamp: u64,
    pub price: f64,
    pub bid: f64,
    pub ask: f64,
}
