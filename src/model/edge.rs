use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::amount::Amount;
use super::node::NodeId;

/// An edge representing token flow between two nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edge {
    /// Source node ID.
    pub from_node: NodeId,
    /// Destination node ID.
    pub to_node: NodeId,
    /// Token being transferred, e.g. "USDC", "ETH", "WBTC".
    pub token: String,
    /// Amount to transfer.
    pub amount: Amount,
}
