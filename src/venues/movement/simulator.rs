use anyhow::{bail, Result};
use async_trait::async_trait;

use super::super::perps::data::PerpCsvRow;
use crate::model::node::Node;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

/// Swap simulator with fixed slippage + fee model.
/// Optionally tracks held non-USD output tokens using a price feed,
/// so that `total_value()` reflects mark-to-market.
pub struct SwapSimulator {
    slippage_bps: f64,
    fee_bps: f64,
    total_cost: f64,
    /// Price feed for valuing held tokens (e.g. ETH from a USDC→ETH swap).
    price_feed: Option<Vec<PerpCsvRow>>,
    price_cursor: usize,
    current_ts: u64,
    /// Amount of output token held (bought via swap, not yet consumed).
    held_amount: f64,
    /// Entry price at which the token was bought.
    entry_price: f64,
}

impl SwapSimulator {
    pub fn new(slippage_bps: f64, fee_bps: f64) -> Self {
        Self {
            slippage_bps,
            fee_bps,
            total_cost: 0.0,
            price_feed: None,
            price_cursor: 0,
            current_ts: 0,
            held_amount: 0.0,
            entry_price: 0.0,
        }
    }

    /// Attach a price feed so the swap can track spot value of held tokens.
    pub fn with_price_feed(mut self, data: Vec<PerpCsvRow>) -> Self {
        self.price_feed = Some(data);
        self
    }

    fn current_price(&self) -> f64 {
        match &self.price_feed {
            Some(data) if !data.is_empty() => {
                let idx = self.price_cursor.min(data.len() - 1);
                data[idx].mark_price
            }
            _ => 1.0, // stablecoin-to-stablecoin, treat as 1:1
        }
    }

    fn advance_price_cursor(&mut self) {
        if let Some(ref data) = self.price_feed {
            while self.price_cursor + 1 < data.len()
                && data[self.price_cursor + 1].timestamp <= self.current_ts
            {
                self.price_cursor += 1;
            }
        }
    }
}

#[async_trait]
impl Venue for SwapSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let to_token = match node {
            Node::Swap { to_token, .. } => to_token.clone(),
            _ => bail!("SwapSimulator called on non-swap node"),
        };

        let cost_fraction = (self.slippage_bps + self.fee_bps) / 10_000.0;
        let cost = input_amount * cost_fraction;
        let output_usd = input_amount - cost;

        self.total_cost += cost;

        // If we have a price feed, track the held token amount
        if self.price_feed.is_some() {
            self.advance_price_cursor();
            let price = self.current_price();
            if price > 0.0 {
                let token_amount = output_usd / price;
                // Weighted average entry price
                let old_notional = self.held_amount * self.entry_price;
                self.held_amount += token_amount;
                self.entry_price = if self.held_amount > 0.0 {
                    (old_notional + output_usd) / self.held_amount
                } else {
                    0.0
                };
            }
            // The tokens are "held" inside the swap venue, not output to balances.
            // Return PositionUpdate so the engine doesn't create an idle balance.
            Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            })
        } else {
            Ok(ExecutionResult::TokenOutput {
                token: to_token,
                amount: output_usd,
            })
        }
    }

    async fn total_value(&self) -> Result<f64> {
        if self.price_feed.is_some() && self.held_amount > 0.0 {
            Ok(self.held_amount * self.current_price())
        } else {
            Ok(0.0)
        }
    }

    async fn tick(&mut self, now: u64, _dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_price_cursor();
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            swap_costs: self.total_cost,
            ..Default::default()
        }
    }
}

/// Bridge simulator with fixed fee model.
pub struct BridgeSimulator {
    fee_bps: f64,
    total_cost: f64,
}

impl BridgeSimulator {
    pub fn new(fee_bps: f64) -> Self {
        Self {
            fee_bps,
            total_cost: 0.0,
        }
    }
}

#[async_trait]
impl Venue for BridgeSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let token = match node {
            Node::Bridge { token, .. } => token.clone(),
            _ => bail!("BridgeSimulator called on non-bridge node"),
        };

        let cost = input_amount * (self.fee_bps / 10_000.0);
        let output = input_amount - cost;
        self.total_cost += cost;

        Ok(ExecutionResult::TokenOutput {
            token,
            amount: output,
        })
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        Ok(())
    }
}
