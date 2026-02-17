use anyhow::{bail, Result};

use crate::data::csv_types::PendleCsvRow;
use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, VenueSimulator};
use crate::model::node::{Node, PendleAction};

/// Pendle simulator — PT/YT yield tokenization.
/// PT (principal token) appreciates toward 1:1 with underlying at maturity.
/// YT (yield token) receives variable yield stream.
pub struct PendleSimulator {
    market_data: Vec<PendleCsvRow>,
    cursor: usize,
    pt_amount: f64,
    yt_amount: f64,
    entry_pt_price: f64,
    entry_yt_price: f64,
    pub accrued_yield: f64,
}

impl PendleSimulator {
    pub fn new(market_data: Vec<PendleCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
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

    fn advance_cursor(&mut self, clock: &SimClock) {
        let ts = clock.current_timestamp();
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= ts
        {
            self.cursor += 1;
        }
    }
}

impl VenueSimulator for PendleSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
    ) -> Result<ExecutionResult> {
        let action = match node {
            Node::Pendle { action, .. } => action,
            _ => bail!("PendleSimulator called on non-pendle node"),
        };

        self.advance_cursor(clock);
        let row = self.current_row().clone();

        match action {
            PendleAction::MintPt => {
                // Buy PT with input USD. PT is priced at a discount to underlying.
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
                // Redeem PT for underlying. At maturity PT = 1:1.
                let value = self.pt_amount * row.pt_price * row.underlying_price;
                self.pt_amount = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: value,
                })
            }
            PendleAction::MintYt => {
                // Buy YT with input USD.
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

    fn total_value(&self, _clock: &SimClock) -> f64 {
        let row = self.current_row();
        let pt_value = self.pt_amount * row.pt_price * row.underlying_price;
        let yt_value = self.yt_amount * row.yt_price * row.underlying_price;
        pt_value + yt_value + self.accrued_yield
    }

    fn tick(&mut self, clock: &SimClock) -> Result<()> {
        self.advance_cursor(clock);

        let dt = clock.dt_years();
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
