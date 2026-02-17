pub mod clock;
pub mod optimizer;
pub mod state;
pub mod topo;

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::model::node::{CronInterval, Node, NodeId, Trigger};
use crate::model::workflow::Workflow;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

use state::NodeBalances;

/// Execution engine that walks a workflow DAG and dispatches to venues.
///
/// Both backtest (data-driven simulators) and live (on-chain executors) share
/// this engine.  The caller controls timing: for backtests, a `SimClock`
/// drives `tick(now, dt_secs)`; for live runs, real wall-clock time does.
pub struct Engine {
    pub workflow: Workflow,
    pub venues: HashMap<NodeId, Box<dyn Venue>>,
    pub balances: NodeBalances,

    /// Topological order of non-triggered nodes for the deploy phase.
    deploy_order: Vec<NodeId>,
    /// Triggered nodes with their cron intervals.
    triggered_nodes: Vec<(NodeId, CronInterval)>,
    /// Last-fired timestamp for each triggered node.
    trigger_last_fired: HashMap<NodeId, u64>,

    // ── Counters for metrics ──
    pub rebalances: u32,
}

impl Engine {
    pub fn new(
        workflow: Workflow,
        venues: HashMap<NodeId, Box<dyn Venue>>,
    ) -> Self {
        let deploy_order = topo::deploy_order(&workflow);
        let triggered_nodes = extract_triggered_nodes(&workflow);

        Self {
            workflow,
            venues,
            balances: NodeBalances::default(),
            deploy_order,
            triggered_nodes,
            trigger_last_fired: HashMap::new(),
            rebalances: 0,
        }
    }

    /// Return the deploy order (for diagnostic printing).
    pub fn deploy_order(&self) -> &[NodeId] {
        &self.deploy_order
    }

    /// Run the one-time deploy phase: execute non-triggered nodes in topological order.
    pub async fn deploy(&mut self) -> Result<()> {
        let order = self.deploy_order.clone();
        for node_id in &order {
            self.execute_node(node_id)
                .await
                .with_context(|| format!("deploying node '{node_id}'"))?;
        }
        Ok(())
    }

    /// Advance the simulation by one tick:
    /// 1. Tick all active venues (accrue interest/funding, check liquidations)
    /// 2. Fire triggered nodes whose interval has elapsed
    ///
    /// Returns the list of triggered node IDs that fired this tick.
    pub async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<Vec<NodeId>> {
        // Tick all venues
        self.tick_venues(now, dt_secs).await?;

        // Fire triggered nodes
        let mut fired = Vec::new();
        let triggers = self.triggered_nodes.clone();
        for (node_id, interval) in &triggers {
            if self.should_fire(node_id, interval, now) {
                self.execute_node(node_id)
                    .await
                    .with_context(|| format!("firing triggered node '{node_id}'"))?;
                self.trigger_last_fired.insert(node_id.clone(), now);
                fired.push(node_id.clone());
            }
        }

        Ok(fired)
    }

    /// Tick all venues without checking triggers.
    /// Used by the live runner which manages its own scheduling.
    pub async fn tick_venues(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        let node_ids: Vec<NodeId> = self.venues.keys().cloned().collect();
        for node_id in &node_ids {
            if let Some(venue) = self.venues.get_mut(node_id) {
                venue.tick(now, dt_secs).await?;
            }
        }
        Ok(())
    }

    /// Current total value of the portfolio (all venue positions + undeployed balances).
    pub async fn total_tvl(&self) -> f64 {
        let mut venue_value = 0.0;
        for v in self.venues.values() {
            venue_value += v.total_value().await.unwrap_or(0.0);
        }
        venue_value + self.balances.total_value()
    }

    /// Collect aggregated metrics from all venues.
    pub fn collect_metrics(&self) -> SimMetrics {
        let mut m = SimMetrics::default();
        for v in self.venues.values() {
            let vm = v.metrics();
            m.funding_pnl += vm.funding_pnl;
            m.premium_pnl += vm.premium_pnl;
            m.lp_fees += vm.lp_fees;
            m.lending_interest += vm.lending_interest;
            m.swap_costs += vm.swap_costs;
            m.liquidations += vm.liquidations;
        }
        m
    }

    /// Execute a single node: gather inputs, call venue (or optimizer), distribute outputs.
    pub async fn execute_node(&mut self, node_id: &str) -> Result<()> {
        let node = self
            .workflow
            .nodes
            .iter()
            .find(|n| n.id() == node_id)
            .cloned()
            .with_context(|| format!("node '{node_id}' not found"))?;

        // Gather inputs from incoming edges
        let (input_token, input_amount) = self.gather_inputs(node_id);

        // Handle optimizer specially
        if let Node::Optimizer {
            ref id,
            drift_threshold,
            ..
        } = node
        {
            return self
                .execute_optimizer(&node, id, input_amount, drift_threshold)
                .await;
        }

        // Normal node: call venue
        if let Some(venue) = self.venues.get_mut(node_id) {
            let result = venue.execute(&node, input_amount).await?;
            self.distribute_result(node_id, &input_token, result)?;
        }

        Ok(())
    }

    /// Resolve incoming edges and transfer tokens from source nodes to this node.
    /// Returns (primary_token, total_amount).
    fn gather_inputs(&mut self, node_id: &str) -> (String, f64) {
        let edges: Vec<_> = self
            .workflow
            .edges
            .iter()
            .filter(|e| e.to_node == node_id)
            .cloned()
            .collect();

        let mut total = 0.0;
        let mut primary_token = String::new();

        for edge in &edges {
            let available = self.balances.get(&edge.from_node, &edge.token);
            let amount = state::resolve(available, &edge.amount);
            if amount > 0.0 {
                self.balances.deduct(&edge.from_node, &edge.token, amount);
                self.balances.add(node_id, &edge.token, amount);
                total += amount;
                if primary_token.is_empty() {
                    primary_token = edge.token.clone();
                }
            }
        }

        (primary_token, total)
    }

    /// Distribute an execution result to outgoing edges.
    fn distribute_result(
        &mut self,
        node_id: &str,
        input_token: &str,
        result: ExecutionResult,
    ) -> Result<()> {
        match result {
            ExecutionResult::TokenOutput { token, amount } => {
                // Remove the input token balance (was consumed by venue)
                self.balances.deduct(node_id, input_token, f64::MAX);
                // Add the output token
                self.balances.add(node_id, &token, amount);
            }
            ExecutionResult::PositionUpdate { consumed, output } => {
                // Deduct consumed input
                self.balances.deduct(node_id, input_token, consumed);
                // Add any output (e.g. premium)
                if let Some((token, amount)) = output {
                    self.balances.add(node_id, &token, amount);
                }
            }
            ExecutionResult::Noop => {}
        }
        Ok(())
    }

    /// Execute an optimizer node: compute Kelly allocations, distribute capital to targets.
    async fn execute_optimizer(
        &mut self,
        node: &Node,
        node_id: &str,
        input_amount: f64,
        drift_threshold: f64,
    ) -> Result<()> {
        // Get total capital available at this optimizer
        let existing_balance = self.balances.get(node_id, "USDC");
        let total_capital = existing_balance + input_amount;

        if total_capital <= 0.0 {
            return Ok(());
        }

        // Check drift — if this is a periodic rebalance, only proceed if drifted
        if drift_threshold > 0.0 && self.trigger_last_fired.contains_key(node_id) {
            let alloc_result = optimizer::compute_kelly_allocations(node, total_capital)?;
            let mut current_values: Vec<(NodeId, f64)> = Vec::new();
            for (target_id, _) in &alloc_result.allocations {
                let value = if let Some(venue) = self.venues.get(target_id.as_str()) {
                    venue.total_value().await.unwrap_or(0.0)
                } else {
                    0.0
                };
                current_values.push((target_id.clone(), value));
            }

            if !optimizer::should_rebalance(
                &current_values,
                &alloc_result.allocations,
                drift_threshold,
            ) {
                return Ok(());
            }
            self.rebalances += 1;
        }

        // Compute and distribute allocations
        let alloc_result = optimizer::compute_kelly_allocations(node, total_capital)?;

        // Deduct all capital from optimizer's balance
        self.balances.deduct(node_id, "USDC", total_capital);

        // Find outgoing edges to determine token
        let outgoing_edges: Vec<_> = self
            .workflow
            .edges
            .iter()
            .filter(|e| e.from_node == node_id)
            .cloned()
            .collect();

        for (target_id, fraction) in &alloc_result.allocations {
            let amount = total_capital * fraction;
            if amount <= 0.0 {
                continue;
            }

            // Find the edge token for this target
            let token = outgoing_edges
                .iter()
                .find(|e| e.to_node == *target_id)
                .map(|e| e.token.clone())
                .unwrap_or_else(|| "USDC".to_string());

            // Add to target node's balance
            self.balances.add(target_id, &token, amount);

            // Execute the target node's venue
            let target_node = self
                .workflow
                .nodes
                .iter()
                .find(|n| n.id() == target_id)
                .cloned();

            if let Some(ref target_node) = target_node {
                if let Some(venue) = self.venues.get_mut(target_id.as_str()) {
                    let result = venue.execute(target_node, amount).await?;
                    self.distribute_result(target_id, &token, result)?;
                }
            }
        }

        Ok(())
    }

    fn should_fire(&self, node_id: &str, interval: &CronInterval, now: u64) -> bool {
        let last_fired = self.trigger_last_fired.get(node_id).copied().unwrap_or(0);
        let elapsed = now.saturating_sub(last_fired);
        let period_seconds = cron_to_seconds(interval);
        elapsed >= period_seconds
    }
}

fn cron_to_seconds(interval: &CronInterval) -> u64 {
    match interval {
        CronInterval::Hourly => 3600,
        CronInterval::Daily => 86400,
        CronInterval::Weekly => 604800,
        CronInterval::Monthly => 2592000,
    }
}

fn extract_triggered_nodes(workflow: &Workflow) -> Vec<(NodeId, CronInterval)> {
    workflow
        .nodes
        .iter()
        .filter_map(|n| {
            if let Some(trigger) = get_trigger(n) {
                if let Trigger::Cron { interval } = trigger {
                    return Some((n.id().to_string(), *interval));
                }
            }
            None
        })
        .collect()
}

/// Extract the trigger from a node, if present.
fn get_trigger(node: &Node) -> Option<&Trigger> {
    match node {
        Node::Perp { trigger, .. }
        | Node::Options { trigger, .. }
        | Node::Spot { trigger, .. }
        | Node::Lp { trigger, .. }
        | Node::Swap { trigger, .. }
        | Node::Bridge { trigger, .. }
        | Node::Lending { trigger, .. }
        | Node::Pendle { trigger, .. }
        | Node::Optimizer { trigger, .. } => trigger.as_ref(),
        Node::Wallet { .. } => None,
    }
}
