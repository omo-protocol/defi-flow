use std::collections::HashMap;

use crate::model::amount::Amount;
use crate::model::node::NodeId;

/// Per-node token balance tracking.
#[derive(Debug, Default)]
pub struct NodeBalances {
    inner: HashMap<NodeId, HashMap<String, f64>>,
}

impl NodeBalances {
    pub fn get(&self, node_id: &str, token: &str) -> f64 {
        self.inner
            .get(node_id)
            .and_then(|m| m.get(token))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn add(&mut self, node_id: &str, token: &str, amount: f64) {
        *self
            .inner
            .entry(node_id.to_string())
            .or_default()
            .entry(token.to_string())
            .or_insert(0.0) += amount;
    }

    /// Deduct up to `amount` from a node's token balance. Returns actual amount deducted.
    pub fn deduct(&mut self, node_id: &str, token: &str, amount: f64) -> f64 {
        let entry = self
            .inner
            .entry(node_id.to_string())
            .or_default()
            .entry(token.to_string())
            .or_insert(0.0);
        let actual = amount.min(*entry);
        *entry -= actual;
        actual
    }

    /// Resolve an Amount enum against a node's current token balance.
    pub fn resolve_amount(&self, node_id: &str, token: &str, amount: &Amount) -> f64 {
        let balance = self.get(node_id, token);
        resolve(balance, amount)
    }

    /// Total undeployed value across all node balances (sum of everything).
    pub fn total_value(&self) -> f64 {
        self.inner
            .values()
            .flat_map(|m| m.values())
            .sum()
    }
}

/// Resolve an Amount against a given balance.
pub fn resolve(balance: f64, amount: &Amount) -> f64 {
    match amount {
        Amount::Fixed { value } => value.parse::<f64>().unwrap_or(0.0),
        Amount::Percentage { value } => balance * (value / 100.0),
        Amount::All => balance,
    }
}
