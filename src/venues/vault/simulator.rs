use anyhow::{bail, Result};
use async_trait::async_trait;

use super::data::VaultCsvRow;
use crate::model::node::{Node, VaultAction};
use crate::venues::{ExecutionResult, SimMetrics, Venue};

const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

/// Vault simulator — deposit APY accrual and reward emissions.
pub struct VaultSimulator {
    market_data: Vec<VaultCsvRow>,
    cursor: usize,
    current_ts: u64,
    pub deposited: f64,
    pub accrued_yield: f64,
    pub accrued_rewards: f64,
}

impl VaultSimulator {
    pub fn new(market_data: Vec<VaultCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            deposited: 0.0,
            accrued_yield: 0.0,
            accrued_rewards: 0.0,
        }
    }

    fn current_row(&self) -> &VaultCsvRow {
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
impl Venue for VaultSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let (action, asset) = match node {
            Node::Vault { action, asset, .. } => (action, asset.clone()),
            _ => bail!("VaultSimulator called on non-vault node"),
        };

        self.advance_cursor();

        match action {
            VaultAction::Deposit => {
                self.deposited += input_amount;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            VaultAction::Withdraw => {
                let available = self.deposited + self.accrued_yield;
                let withdraw = input_amount.min(available);
                self.deposited = (self.deposited - withdraw).max(0.0);
                self.accrued_yield = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: asset,
                    amount: withdraw,
                })
            }
            VaultAction::ClaimRewards => {
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
        Ok(self.deposited + self.accrued_yield + self.accrued_rewards)
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();

        let dt = dt_secs / SECS_PER_YEAR;
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();

        if self.deposited > 0.0 {
            self.accrued_yield += self.deposited * row.apy * dt;
            self.accrued_rewards += self.deposited * row.reward_apy * dt;
        }

        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lending_interest: self.accrued_yield,
            ..Default::default()
        }
    }
}
