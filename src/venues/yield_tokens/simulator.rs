use anyhow::{bail, Result};
use async_trait::async_trait;

use super::data::PendleCsvRow;
use crate::model::node::{Node, PendleAction};
use crate::venues::{ExecutionResult, Venue};

const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

/// Pendle simulator — PT/YT yield tokenization.
/// PT (principal token) appreciates toward 1:1 with underlying at maturity.
/// YT (yield token) receives variable yield stream.
pub struct YieldSimulator {
    market_data: Vec<PendleCsvRow>,
    cursor: usize,
    current_ts: u64,
    pt_amount: f64,
    yt_amount: f64,
    entry_pt_price: f64,
    entry_yt_price: f64,
    pub accrued_yield: f64,
}

impl YieldSimulator {
    pub fn new(market_data: Vec<PendleCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            pt_amount: 0.0,
            yt_amount: 0.0,
            entry_pt_price: 0.0,
            entry_yt_price: 0.0,
            accrued_yield: 0.0,
        }
    }

    fn current_row(&self) -> &PendleCsvRow {
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
impl Venue for YieldSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let action = match node {
            Node::Pendle { action, .. } => action,
            _ => bail!("YieldSimulator called on non-pendle node"),
        };

        self.advance_cursor();
        let row = self.current_row().clone();

        match action {
            PendleAction::MintPt => {
                if row.pt_price > 0.0 {
                    let pt_units = input_amount / (row.pt_price * row.underlying_price);
                    self.pt_amount += pt_units;
                    self.entry_pt_price = row.pt_price;
                }
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            PendleAction::RedeemPt => {
                let value = self.pt_amount * row.pt_price * row.underlying_price;
                self.pt_amount = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: value,
                })
            }
            PendleAction::MintYt => {
                if row.yt_price > 0.0 {
                    let yt_units = input_amount / (row.yt_price * row.underlying_price);
                    self.yt_amount += yt_units;
                    self.entry_yt_price = row.yt_price;
                }
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            PendleAction::RedeemYt => {
                let value = self.yt_amount * row.yt_price * row.underlying_price;
                self.yt_amount = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: value,
                })
            }
            PendleAction::ClaimRewards => {
                let yield_amount = self.accrued_yield;
                self.accrued_yield = 0.0;
                if yield_amount > 0.0 {
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: yield_amount,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        let row = self.current_row();
        let pt_value = self.pt_amount * row.pt_price * row.underlying_price;
        let yt_value = self.yt_amount * row.yt_price * row.underlying_price;
        Ok(pt_value + yt_value + self.accrued_yield)
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();

        let dt = dt_secs / SECS_PER_YEAR;
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();

        // YT holders earn the variable yield
        if self.yt_amount > 0.0 {
            let yield_earned = self.yt_amount * row.implied_apy * row.underlying_price * dt;
            self.accrued_yield += yield_earned;
        }

        Ok(())
    }
}
