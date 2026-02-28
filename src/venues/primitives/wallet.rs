use anyhow::Result;
use async_trait::async_trait;

use crate::model::node::Node;
use crate::venues::{ExecutionResult, Venue};

/// Trivial venue for wallet nodes — just tracks a balance.
pub struct WalletVenue {
    balance: f64,
}

impl WalletVenue {
    pub fn new() -> Self {
        Self { balance: 0.0 }
    }
}

#[async_trait]
impl Venue for WalletVenue {
    async fn execute(&mut self, _node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        self.balance += input_amount;
        Ok(ExecutionResult::Noop)
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.balance)
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        if self.balance <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let freed = self.balance * f;
        self.balance -= freed;
        Ok(freed)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        Ok(())
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        None
    }
}
