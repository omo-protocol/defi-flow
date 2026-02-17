use anyhow::{bail, Result};

use crate::data::csv_types::LendingCsvRow;
use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, SimMetrics, VenueSimulator};
use crate::model::node::{LendingAction, Node};

/// Lending simulator — supply APY accrual, borrow tracking, reward emissions.
pub struct LendingSimulator {
    market_data: Vec<LendingCsvRow>,
    cursor: usize,
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

    fn advance_cursor(&mut self, clock: &SimClock) {
        let ts = clock.current_timestamp();
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= ts
        {
            self.cursor += 1;
        }
    }
}

impl VenueSimulator for LendingSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
    ) -> Result<ExecutionResult> {
        let (action, asset) = match node {
            Node::Lending { action, asset, .. } => (action, asset.clone()),
            _ => bail!("LendingSimulator called on non-lending node"),
        };

        self.advance_cursor(clock);

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

    fn total_value(&self, _clock: &SimClock) -> f64 {
        self.supplied + self.accrued_supply_interest + self.accrued_rewards
            - self.borrowed
            - self.accrued_borrow_interest
    }

    fn tick(&mut self, clock: &SimClock) -> Result<()> {
        self.advance_cursor(clock);

        let dt = clock.dt_years();
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();

        // Accrue supply interest
        if self.supplied > 0.0 {
            self.accrued_supply_interest += self.supplied * row.supply_apy * dt;
        }

        // Accrue borrow interest (cost)
        if self.borrowed > 0.0 {
            self.accrued_borrow_interest += self.borrowed * row.borrow_apy * dt;
        }

        // Accrue rewards
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
}
