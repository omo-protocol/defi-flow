use std::collections::HashMap;

use anyhow::{Result, bail};
use async_trait::async_trait;

use super::data::OptionsCsvRow;
use crate::model::node::{Node, OptionsAction};
use crate::venues::{ExecutionResult, SimMetrics, Venue};

#[derive(Debug, Clone, PartialEq)]
enum OptionType {
    CoveredCall,
    CashSecuredPut,
}

#[derive(Debug, Clone)]
struct StrikeData {
    option_type: OptionType,
    strike: f64,
    expiry: u64,
    days_to_expiry: f64,
    premium: f64,
    apy: f64,
    delta: Option<f64>,
}

struct OptionsSnapshot {
    timestamp: u64,
    spot_prices: HashMap<String, f64>,
    strikes: HashMap<String, Vec<StrikeData>>,
}

#[allow(dead_code)]
struct SoldOption {
    asset: String,
    strike: f64,
    expiry: u64,
    option_type: OptionType,
    size: f64,
    premium_received: f64,
}

/// Options simulator — ported from markowitz's MockOptionsClient.
pub struct OptionsSimulator {
    snapshots: Vec<OptionsSnapshot>,
    cursor: usize,
    current_ts: u64,
    sold_options: Vec<SoldOption>,
    pub total_premium: f64,
    pub settlement_losses: f64,
    balance: f64,
}

impl OptionsSimulator {
    pub fn new(rows: Vec<OptionsCsvRow>) -> Self {
        let snapshots = build_snapshots(rows);
        Self {
            snapshots,
            cursor: 0,
            current_ts: 0,
            sold_options: Vec::new(),
            total_premium: 0.0,
            settlement_losses: 0.0,
            balance: 0.0,
        }
    }

    fn current_snapshot(&self) -> Option<&OptionsSnapshot> {
        self.snapshots
            .get(self.cursor.min(self.snapshots.len().saturating_sub(1)))
    }

    fn advance_cursor(&mut self) {
        while self.cursor + 1 < self.snapshots.len()
            && self.snapshots[self.cursor + 1].timestamp <= self.current_ts
        {
            self.cursor += 1;
        }
    }

    fn settle_expired(&mut self, timestamp: u64, spot_prices: &HashMap<String, f64>) {
        let mut losses = 0.0;

        self.sold_options.retain(|sold| {
            if sold.expiry > timestamp {
                return true;
            }

            let spot = match spot_prices.get(&sold.asset) {
                Some(&p) => p,
                None => return false,
            };

            let loss = match sold.option_type {
                OptionType::CashSecuredPut => {
                    if spot < sold.strike {
                        (sold.strike - spot) * sold.size
                    } else {
                        0.0
                    }
                }
                OptionType::CoveredCall => {
                    if spot > sold.strike {
                        (spot - sold.strike) * sold.size
                    } else {
                        0.0
                    }
                }
            };

            losses += loss;
            false
        });

        self.settlement_losses += losses;
        self.balance -= losses;
    }

    fn select_strike(
        &self,
        asset: &str,
        option_type: &OptionType,
        delta_target: Option<f64>,
        days_to_expiry: Option<u32>,
        min_apy: Option<f64>,
    ) -> Option<StrikeData> {
        let snapshot = self.current_snapshot()?;
        let strikes = snapshot.strikes.get(asset)?;

        let filtered: Vec<&StrikeData> = strikes
            .iter()
            .filter(|s| {
                if s.option_type != *option_type {
                    return false;
                }
                if let Some(min) = min_apy {
                    if s.apy < min {
                        return false;
                    }
                }
                if let Some(dte) = days_to_expiry {
                    let target = dte as f64;
                    if s.days_to_expiry < target * 0.5 || s.days_to_expiry > target * 1.5 {
                        return false;
                    }
                }
                true
            })
            .collect();

        if filtered.is_empty() {
            return None;
        }

        if let Some(target_delta) = delta_target {
            filtered
                .into_iter()
                .min_by(|a, b| {
                    let da = (a.delta.unwrap_or(0.5) - target_delta).abs();
                    let db = (b.delta.unwrap_or(0.5) - target_delta).abs();
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
        } else {
            filtered
                .into_iter()
                .max_by(|a, b| {
                    a.premium
                        .partial_cmp(&b.premium)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
        }
    }
}

#[async_trait]
impl Venue for OptionsSimulator {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        let (action, asset, delta_target, days_to_expiry, min_apy, batch_size) = match node {
            Node::Options {
                action,
                asset,
                delta_target,
                days_to_expiry,
                min_apy,
                batch_size,
                ..
            } => (
                action,
                format!("{asset:?}"),
                *delta_target,
                *days_to_expiry,
                *min_apy,
                *batch_size,
            ),
            _ => bail!("OptionsSimulator called on non-options node"),
        };

        self.advance_cursor();

        match action {
            OptionsAction::SellCoveredCall | OptionsAction::SellCashSecuredPut => {
                let opt_type = match action {
                    OptionsAction::SellCoveredCall => OptionType::CoveredCall,
                    _ => OptionType::CashSecuredPut,
                };

                let strike_data =
                    self.select_strike(&asset, &opt_type, delta_target, days_to_expiry, min_apy);

                if let Some(strike) = strike_data {
                    let size = if let Some(bs) = batch_size {
                        ((input_amount / strike.premium) / bs as f64).floor() * bs as f64
                    } else {
                        input_amount / strike.premium
                    };

                    if size > 0.0 {
                        let premium = strike.premium * size;
                        self.total_premium += premium;
                        self.balance += input_amount + premium;

                        self.sold_options.push(SoldOption {
                            asset: asset.clone(),
                            strike: strike.strike,
                            expiry: strike.expiry,
                            option_type: opt_type,
                            size,
                            premium_received: premium,
                        });

                        return Ok(ExecutionResult::PositionUpdate {
                            consumed: input_amount,
                            output: Some(("USDC".to_string(), premium)),
                        });
                    }
                }

                self.balance += input_amount;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            OptionsAction::BuyCall | OptionsAction::BuyPut => {
                self.balance += input_amount;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            OptionsAction::CollectPremium => {
                let available = self.balance.max(0.0);
                if available > 0.0 {
                    self.balance -= available;
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: available,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
            OptionsAction::Roll => Ok(ExecutionResult::Noop),
            OptionsAction::Close => {
                let available = self.balance.max(0.0);
                self.sold_options.clear();
                self.balance = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: available,
                })
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.balance.max(0.0))
    }

    async fn tick(&mut self, now: u64, _dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();
        if let Some(snapshot) = self.current_snapshot() {
            let ts = snapshot.timestamp;
            let spots = snapshot.spot_prices.clone();
            self.settle_expired(ts, &spots);
        }
        Ok(())
    }

    async fn unwind(&mut self, _fraction: f64) -> Result<f64> {
        // European options cannot be closed before expiry.
        // Collateral is locked until settlement — nothing to free.
        Ok(0.0)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            premium_pnl: self.total_premium - self.settlement_losses,
            ..Default::default()
        }
    }
}

fn parse_option_type(s: &str) -> OptionType {
    match s.to_lowercase().as_str() {
        "csp" | "cashsecuredputs" | "put" | "puts" => OptionType::CashSecuredPut,
        _ => OptionType::CoveredCall,
    }
}

fn build_snapshots(rows: Vec<OptionsCsvRow>) -> Vec<OptionsSnapshot> {
    let mut by_snapshot: HashMap<u64, Vec<OptionsCsvRow>> = HashMap::new();
    for row in rows {
        by_snapshot.entry(row.snapshot).or_default().push(row);
    }

    let mut snapshot_ids: Vec<u64> = by_snapshot.keys().copied().collect();
    snapshot_ids.sort();

    snapshot_ids
        .into_iter()
        .map(|id| {
            let rows = by_snapshot.remove(&id).unwrap();
            let mut strikes: HashMap<String, Vec<StrikeData>> = HashMap::new();
            let mut spot_prices: HashMap<String, f64> = HashMap::new();
            let mut timestamp = 0u64;
            for row in rows {
                timestamp = row.timestamp;
                spot_prices
                    .entry(row.asset.clone())
                    .or_insert(row.spot_price);
                let strike = StrikeData {
                    option_type: parse_option_type(&row.option_type),
                    strike: row.strike,
                    expiry: row.expiry,
                    days_to_expiry: row.days_to_expiry,
                    premium: row.premium,
                    apy: row.apy,
                    delta: row.delta,
                };
                strikes.entry(row.asset.clone()).or_default().push(strike);
            }
            OptionsSnapshot {
                timestamp,
                spot_prices,
                strikes,
            }
        })
        .collect()
}
