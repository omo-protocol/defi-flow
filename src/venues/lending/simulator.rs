use anyhow::{bail, Result};
use async_trait::async_trait;

use super::data::LendingCsvRow;
use crate::model::node::{LendingAction, Node};
use crate::venues::{ExecutionResult, SimMetrics, Venue};

const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

/// Lending simulator — supply APY accrual, borrow tracking, reward emissions.
pub struct LendingSimulator {
    market_data: Vec<LendingCsvRow>,
    cursor: usize,
    current_ts: u64,
    pub supplied: f64,
    pub borrowed: f64,
    pub accrued_supply_interest: f64,
    pub accrued_borrow_interest: f64,
    pub accrued_rewards: f64,
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
        }
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
}

#[async_trait]
impl Venue for LendingSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let (action, asset) = match node {
            Node::Lending { action, asset, .. } => (action, asset.clone()),
            _ => bail!("LendingSimulator called on non-lending node"),
        };

        self.advance_cursor();

        match action {
            LendingAction::Supply => {
                self.supplied += input_amount;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            LendingAction::Withdraw => {
                let available = self.supplied + self.accrued_supply_interest;
                let withdraw = input_amount.min(available);
                self.supplied = (self.supplied - withdraw).max(0.0);
                self.accrued_supply_interest = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: asset,
                    amount: withdraw,
                })
            }
            LendingAction::Borrow => {
                self.borrowed += input_amount;
                Ok(ExecutionResult::TokenOutput {
                    token: asset,
                    amount: input_amount,
                })
            }
            LendingAction::Repay => {
                let repay_amount = input_amount.min(self.borrowed + self.accrued_borrow_interest);
                self.borrowed = (self.borrowed - repay_amount).max(0.0);
                self.accrued_borrow_interest = 0.0;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: repay_amount,
                    output: None,
                })
            }
            LendingAction::ClaimRewards => {
                let rewards = self.accrued_rewards;
                self.accrued_rewards = 0.0;
                if rewards > 0.0 {
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: rewards,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.supplied + self.accrued_supply_interest + self.accrued_rewards
            - self.borrowed
            - self.accrued_borrow_interest)
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();

        let dt = dt_secs / SECS_PER_YEAR;
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();

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
        SimMetrics {
            lending_interest: self.accrued_supply_interest,
            ..Default::default()
        }
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
