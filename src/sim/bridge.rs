use anyhow::{bail, Result};

use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, VenueSimulator};
use crate::model::node::Node;

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

    pub fn total_cost(&self) -> f64 {
        self.total_cost
    }
}

impl VenueSimulator for BridgeSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        _clock: &SimClock,
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

    fn total_value(&self, _clock: &SimClock) -> f64 {
        0.0 // Bridge doesn't hold positions
    }

    fn tick(&mut self, _clock: &SimClock) -> Result<()> {
        Ok(())
    }
}
