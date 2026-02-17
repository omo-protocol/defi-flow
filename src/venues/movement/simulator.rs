use anyhow::{bail, Result};
use async_trait::async_trait;

use crate::model::node::Node;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

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
        let output = input_amount - cost;

        self.total_cost += cost;

        Ok(ExecutionResult::TokenOutput {
            token: to_token,
            amount: output,
        })
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
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
