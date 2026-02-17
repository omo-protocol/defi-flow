use anyhow::Result;
use async_trait::async_trait;

use crate::engine::venue::{ExecutionResult, SimMetrics};
use crate::model::node::Node;

use super::VenueExecutor;

/// Stub executor for venues not yet implemented in Phase 1.
/// Logs what action would be taken and returns Noop.
pub struct StubExecutor {
    node_type: String,
}

impl StubExecutor {
    pub fn new(node_type: &str) -> Self {
        StubExecutor {
            node_type: node_type.to_string(),
        }
    }
}

#[async_trait]
impl VenueExecutor for StubExecutor {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        println!(
            "  STUB [{}] {} — would execute with ${:.2} input",
            self.node_type,
            node.label(),
            input_amount,
        );

        // For wallet nodes, pass through the balance
        if matches!(node, Node::Wallet { .. }) {
            return Ok(ExecutionResult::Noop);
        }

        // For stubs, consume the input as a "position"
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0)
    }

    async fn tick(&mut self) -> Result<()> {
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics::default()
    }
}
