use anyhow::{bail, Result};

use crate::data::csv_types::PriceCsvRow;
use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, VenueSimulator};
use crate::model::node::{Node, SpotSide};

/// Spot trade simulator — uses price data with slippage.
pub struct SpotSimulator {
    market_data: Vec<PriceCsvRow>,
    cursor: usize,
    slippage_bps: f64,
}

impl SpotSimulator {
    pub fn new(market_data: Vec<PriceCsvRow>, slippage_bps: f64) -> Self {
        Self {
            market_data,
            cursor: 0,
            slippage_bps,
        }
    }

    fn current_row(&self) -> &PriceCsvRow {
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

impl VenueSimulator for SpotSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
    ) -> Result<ExecutionResult> {
        let (pair, side) = match node {
            Node::Spot { pair, side, .. } => (pair.clone(), *side),
            _ => bail!("SpotSimulator called on non-spot node"),
        };

        self.advance_cursor(clock);
        let row = self.current_row();
        let slippage = self.slippage_bps / 10_000.0;

        // Extract output token from pair (e.g. "ETH/USDC" -> buy gives ETH, sell gives USDC)
        let parts: Vec<&str> = pair.split('/').collect();
        let (output_token, fill_price) = match side {
            SpotSide::Buy => {
                let price = row.ask * (1.0 + slippage);
                let output = input_amount / price;
                (parts.first().unwrap_or(&"TOKEN").to_string(), output)
            }
            SpotSide::Sell => {
                let price = row.bid * (1.0 - slippage);
                let output = input_amount * price;
                (parts.get(1).unwrap_or(&"USDC").to_string(), output)
            }
        };

        Ok(ExecutionResult::TokenOutput {
            token: output_token,
            amount: fill_price,
        })
    }

    fn total_value(&self, _clock: &SimClock) -> f64 {
        0.0 // Spot doesn't hold positions
    }

    fn tick(&mut self, _clock: &SimClock) -> Result<()> {
        Ok(())
    }
}
