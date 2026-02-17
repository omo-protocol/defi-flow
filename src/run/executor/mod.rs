pub mod aerodrome;
pub mod evm;
pub mod hyperliquid;
pub mod lending;
pub mod lifi;
pub mod pendle;
pub mod rysk;
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

    for node in &workflow.nodes {
        let id = node.id().to_string();
        let executor: Box<dyn VenueExecutor> = match node {
            Node::Perp { venue, .. } => {
                match venue {
                    crate::model::node::PerpVenue::Hyperliquid
                    | crate::model::node::PerpVenue::Hyena => {
                        Box::new(hyperliquid::HyperliquidExecutor::new(config)?)
                    }
                }
            }
            Node::Spot { .. } => {
                Box::new(hyperliquid::HyperliquidExecutor::new(config)?)
            }
            Node::Swap { .. } => {
                Box::new(lifi::LiFiExecutor::new(config)?)
            }
            Node::Bridge { .. } => {
                Box::new(lifi::LiFiExecutor::new(config)?)
            }
            Node::Lending { .. } => {
                Box::new(lending::LendingExecutor::new(config)?)
            }
            Node::Lp { .. } => {
                Box::new(aerodrome::AerodromeExecutor::new(config)?)
            }
            Node::Pendle { .. } => {
                Box::new(pendle::PendleExecutor::new(config)?)
            }
            Node::Options { .. } => {
                Box::new(rysk::RyskExecutor::new(config)?)
            }
            Node::Wallet { .. } => Box::new(stub::StubExecutor::new("wallet")),
            Node::Optimizer { .. } => Box::new(stub::StubExecutor::new("optimizer")),
        };
        executors.insert(id, executor);
    }

    Ok(executors)
}
