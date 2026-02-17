use anyhow::{bail, Result};

use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, SimMetrics, VenueSimulator};
use crate::model::node::Node;

/// Swap simulator with fixed slippage + fee model.
pub struct SwapSimulator {
    slippage_bps: f64,
    fee_bps: f64,
    total_cost: f64,
}

impl SwapSimulator {
    pub fn new(slippage_bps: f64, fee_bps: f64) -> Self {
        Self {
            slippage_bps,
            fee_bps,
            total_cost: 0.0,
        }
    }

    pub fn total_cost(&self) -> f64 {
        self.total_cost
    }
}

impl VenueSimulator for SwapSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        _clock: &SimClock,
    ) -> Result<ExecutionResult> {
        let to_token = match node {
            Node::Swap { to_token, .. } => to_token.clone(),
            _ => bail!("SwapSimulator called on non-swap node"),
        };

        let cost_fraction = (self.slippage_bps + self.fee_bps) / 10_000.0;
        let cost = input_amount * cost_fraction;
        let output = input_amount - cost;

        self.total_cost += cost;

        Ok(ExecutionResult::TokenOutput {
            token: to_token,
            amount: output,
        })
    }

    fn total_value(&self, _clock: &SimClock) -> f64 {
        0.0 // Swap doesn't hold positions
    }

    fn tick(&mut self, _clock: &SimClock) -> Result<()> {
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            swap_costs: self.total_cost,
            ..Default::default()
        }
    }
}
