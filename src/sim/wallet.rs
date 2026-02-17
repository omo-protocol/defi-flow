use anyhow::Result;

use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, VenueSimulator};
use crate::model::node::Node;

/// Trivial simulator for wallet nodes — just tracks a balance.
pub struct WalletSimulator {
    balance: f64,
}

impl WalletSimulator {
    pub fn new(initial_balance: f64) -> Self {
        Self {
            balance: initial_balance,
        }
    }
}

impl VenueSimulator for WalletSimulator {
    fn execute(
        &mut self,
        _node: &Node,
        input_amount: f64,
        _clock: &SimClock,
    ) -> Result<ExecutionResult> {
        // Wallet just holds tokens — pass through
        self.balance += input_amount;
        Ok(ExecutionResult::Noop)
    }

    fn total_value(&self, _clock: &SimClock) -> f64 {
        self.balance
    }

    fn tick(&mut self, _clock: &SimClock) -> Result<()> {
        Ok(())
    }
}
