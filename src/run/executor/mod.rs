pub mod hyperliquid;
pub mod stub;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::engine::venue::{ExecutionResult, SimMetrics};
use crate::model::node::{Node, NodeId};
use crate::model::workflow::Workflow;

use super::config::RuntimeConfig;

/// Async trait for live on-chain execution (equivalent of VenueSimulator for backtest).
#[async_trait]
pub trait VenueExecutor: Send + Sync {
    /// Execute the node's action (on-chain transaction or API call).
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult>;

    /// Current total value of positions held at this venue (USD).
    async fn total_value(&self) -> Result<f64>;

    /// Periodic maintenance (report positions, etc.).
    async fn tick(&mut self) -> Result<()>;

    /// Report accumulated metrics.
    fn metrics(&self) -> SimMetrics;
}

/// Build executors for all nodes in the workflow.
pub fn build_executors(
    workflow: &Workflow,
    config: &RuntimeConfig,
) -> Result<HashMap<NodeId, Box<dyn VenueExecutor>>> {
    let mut executors: HashMap<NodeId, Box<dyn VenueExecutor>> = HashMap::new();

    // Create a shared HyperliquidExecutor for all Hyperliquid/Hyena nodes
    let hl_executor = hyperliquid::HyperliquidExecutor::new(config)?;

    for node in &workflow.nodes {
        let id = node.id().to_string();
        let executor: Box<dyn VenueExecutor> = match node {
            Node::Perp { venue, .. } => {
                match venue {
                    crate::model::node::PerpVenue::Hyperliquid
                    | crate::model::node::PerpVenue::Hyena => {
                        // Each node gets its own executor instance sharing the same API config
                        Box::new(hyperliquid::HyperliquidExecutor::new(config)?)
                    }
                }
            }
            Node::Spot { .. } => {
                Box::new(hyperliquid::HyperliquidExecutor::new(config)?)
            }
            Node::Wallet { .. } => Box::new(stub::StubExecutor::new("wallet")),
            Node::Optimizer { .. } => Box::new(stub::StubExecutor::new("optimizer")),
            Node::Options { .. } => Box::new(stub::StubExecutor::new("options")),
            Node::Lp { .. } => Box::new(stub::StubExecutor::new("lp")),
            Node::Swap { .. } => Box::new(stub::StubExecutor::new("swap")),
            Node::Bridge { .. } => Box::new(stub::StubExecutor::new("bridge")),
            Node::Lending { .. } => Box::new(stub::StubExecutor::new("lending")),
            Node::Pendle { .. } => Box::new(stub::StubExecutor::new("pendle")),
        };
        executors.insert(id, executor);
    }

    // Drop the prototype
    drop(hl_executor);

    Ok(executors)
}
