use anyhow::Result;

use crate::model::node::{Node, NodeId};

use super::clock::SimClock;

/// Result of executing a node action.
pub enum ExecutionResult {
    /// Produced tokens (swap output, claimed rewards, withdrawal, etc.)
    TokenOutput { token: String, amount: f64 },
    /// Position opened/modified. Consumed input, optionally produced output (e.g. premium).
    PositionUpdate {
        consumed: f64,
        output: Option<(String, f64)>,
    },
    /// Optimizer: per-target capital splits. Engine handles distribution.
    Allocations(Vec<(NodeId, f64)>),
    /// No output (e.g. stake gauge, adjust leverage).
    Noop,
}

/// Metrics reported by a simulator at finalization.
#[derive(Debug, Default)]
pub struct SimMetrics {
    pub funding_pnl: f64,
    pub premium_pnl: f64,
    pub lp_fees: f64,
    pub lending_interest: f64,
    pub swap_costs: f64,
    pub liquidations: u32,
}

/// Trait every venue simulator must implement.
/// Each simulator handles one node instance and maintains its own state.
pub trait VenueSimulator: Send + Sync {
    /// Execute the node's action given available input amount (in the edge's token).
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
    ) -> Result<ExecutionResult>;

    /// Current total value of positions held at this venue (USD terms).
    fn total_value(&self, clock: &SimClock) -> f64;

    /// Advance internal state by one tick (accrue funding/interest, check liquidation, etc.)
    fn tick(&mut self, clock: &SimClock) -> Result<()>;

    /// Report accumulated metrics. Default: all zeros.
    fn metrics(&self) -> SimMetrics {
        SimMetrics::default()
    }
}
