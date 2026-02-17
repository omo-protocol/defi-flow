use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::edge::Edge;
use super::node::Node;

/// A named workflow: a directed acyclic graph of DeFi operation nodes
/// connected by token-flow edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workflow {
    /// Human-readable name for this workflow.
    pub name: String,
    /// Optional description of the strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The nodes (operations) in this workflow.
    pub nodes: Vec<Node>,
    /// The edges (token flows) connecting nodes.
    pub edges: Vec<Edge>,
}
