use anyhow::{Result, bail};
use async_trait::async_trait;

use super::data::LendingCsvRow;
use crate::model::node::{LendingAction, Node};
use crate::venues::perps::data::PriceCsvRow;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

/// Lending simulator — supply APY accrual, borrow tracking, reward emissions.
///
/// When `price_feed` is `Some`, all amounts (`supplied`, `borrowed`, interest)
/// are tracked in **token units** (e.g. ETH) and converted to USD via the
/// current spot price for `total_value()`.  This is required for non-stablecoin
/// lending (e.g. ETH lending in a delta-neutral strategy) so that the position
/// correctly marks-to-market.
///
/// When `price_feed` is `None` (stablecoin lending), all amounts are in USD.
pub struct LendingSimulator {
    market_data: Vec<LendingCsvRow>,
    cursor: usize,
    current_ts: u64,
    pub supplied: f64,
    pub borrowed: f64,
    pub accrued_supply_interest: f64,
    pub accrued_borrow_interest: f64,
    pub accrued_rewards: f64,

    // Optional spot price feed for non-stablecoin assets
    price_feed: Option<Vec<PriceCsvRow>>,
    price_cursor: usize,
}

impl LendingSimulator {
    pub fn new(market_data: Vec<LendingCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            supplied: 0.0,
            borrowed: 0.0,
            accrued_supply_interest: 0.0,
            accrued_borrow_interest: 0.0,
            accrued_rewards: 0.0,
            price_feed: None,
            price_cursor: 0,
        }
    }

    pub fn with_price_feed(mut self, price_data: Vec<PriceCsvRow>) -> Self {
        self.price_feed = Some(price_data);
        self
    }

    fn current_row(&self) -> &LendingCsvRow {
        &self.market_data[self.cursor.min(self.market_data.len() - 1)]
    }

    fn advance_cursor(&mut self) {
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= self.current_ts
        {
            self.cursor += 1;
        }
    }

    /// Current spot price of the asset (1.0 for stablecoins / no price feed).
    fn current_price(&self) -> f64 {
        match &self.price_feed {
            Some(feed) => {
                let idx = self.price_cursor.min(feed.len() - 1);
                feed[idx].price
            }
            None => 1.0,
        }
    }

    fn advance_price_cursor(&mut self) {
        if let Some(ref feed) = self.price_feed {
            while self.price_cursor + 1 < feed.len()
                && feed[self.price_cursor + 1].timestamp <= self.current_ts
            {
                self.price_cursor += 1;
            }
        }
    }
}

#[async_trait]
impl Venue for LendingSimulator {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        let (action, asset) = match node {
            Node::Lending { action, asset, .. } => (action, asset.clone()),
            _ => bail!("LendingSimulator called on non-lending node"),
        };

        self.advance_cursor();
        self.advance_price_cursor();

        let price = self.current_price();

        match action {
            LendingAction::Supply => {
                // input_amount is USD; convert to token units if price-aware
                let tokens = input_amount / price;
                self.supplied += tokens;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            LendingAction::Withdraw => {
                let available = self.supplied + self.accrued_supply_interest;
                let withdraw_tokens = (input_amount / price).min(available);
                self.supplied = (self.supplied - withdraw_tokens).max(0.0);
                self.accrued_supply_interest = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: asset,
                    amount: withdraw_tokens * price,
                })
            }
            LendingAction::Borrow => {
                let tokens = input_amount / price;
                self.borrowed += tokens;
                Ok(ExecutionResult::TokenOutput {
                    token: asset,
                    amount: input_amount,
                })
            }
            LendingAction::Repay => {
                let repay_tokens =
                    (input_amount / price).min(self.borrowed + self.accrued_borrow_interest);
                self.borrowed = (self.borrowed - repay_tokens).max(0.0);
                self.accrued_borrow_interest = 0.0;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: repay_tokens * price,
                    output: None,
                })
            }
            LendingAction::ClaimRewards => {
                let reward_tokens = self.accrued_rewards;
                self.accrued_rewards = 0.0;
                if reward_tokens > 0.0 {
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: reward_tokens * price,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        let price = self.current_price();
        Ok(
            (self.supplied + self.accrued_supply_interest + self.accrued_rewards
                - self.borrowed
                - self.accrued_borrow_interest)
                * price,
        )
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();
        self.advance_price_cursor();

        let dt = dt_secs / SECS_PER_YEAR;
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();

        // Interest accrues in token units (or USD if no price feed)
        if self.supplied > 0.0 {
            self.accrued_supply_interest += self.supplied * row.supply_apy * dt;
        }
        if self.borrowed > 0.0 {
            self.accrued_borrow_interest += self.borrowed * row.borrow_apy * dt;
        }
        if self.supplied > 0.0 {
            self.accrued_rewards += self.supplied * row.reward_apy * dt;
        }

        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        let price = self.current_price();
        SimMetrics {
            lending_interest: self.accrued_supply_interest * price,
            ..Default::default()
        }
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let freed = total * f;
        self.supplied *= 1.0 - f;
        self.accrued_supply_interest *= 1.0 - f;
        self.accrued_rewards *= 1.0 - f;
        Ok(freed)
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        if self.market_data.len() < 2 {
            return None;
        }

        let slice = &self.market_data;
        let n = slice.len() as f64;

        // supply_apy + reward_apy are already annualized
        let apys: Vec<f64> = slice.iter().map(|r| r.supply_apy + r.reward_apy).collect();
        let mean = apys.iter().sum::<f64>() / n;
        let var = apys.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
        let std = var.sqrt();

        Some((mean, std))
    }
}
