pub mod clock;
pub mod optimizer;
pub mod state;
pub mod topo;
pub mod venue;

use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::model::node::{CronInterval, Node, NodeId, Trigger};
use crate::model::workflow::Workflow;

use clock::SimClock;
use state::NodeBalances;
use venue::{ExecutionResult, VenueSimulator};

/// Execution engine that walks a workflow DAG and dispatches to venue simulators.
pub struct Engine {
    pub workflow: Workflow,
    pub simulators: HashMap<NodeId, Box<dyn VenueSimulator>>,
    pub balances: NodeBalances,
    pub clock: SimClock,

    /// Topological order of non-triggered nodes for the deploy phase.
    deploy_order: Vec<NodeId>,
    /// Triggered nodes with their cron intervals.
    triggered_nodes: Vec<(NodeId, CronInterval)>,
    /// Last-fired timestamp for each triggered node.
    trigger_last_fired: HashMap<NodeId, u64>,

    // ── Counters for metrics ──
    pub rebalances: u32,
    pub liquidations: u32,
    pub swap_costs: f64,
    pub funding_pnl: f64,
    pub premium_pnl: f64,
    pub lp_fees: f64,
    pub lending_interest: f64,
}

impl Engine {
    pub fn new(
        workflow: Workflow,
        simulators: HashMap<NodeId, Box<dyn VenueSimulator>>,
        clock: SimClock,
    ) -> Self {
        let deploy_order = topo::deploy_order(&workflow);

        let triggered_nodes: Vec<(NodeId, CronInterval)> = workflow
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
            .collect();

        Self {
            workflow,
            simulators,
            balances: NodeBalances::default(),
            clock,
            deploy_order,
            triggered_nodes,
            trigger_last_fired: HashMap::new(),
            rebalances: 0,
            liquidations: 0,
            swap_costs: 0.0,
            funding_pnl: 0.0,
            premium_pnl: 0.0,
            lp_fees: 0.0,
            lending_interest: 0.0,
        }
    }

    /// Run the one-time deploy phase: execute non-triggered nodes in topological order.
    pub fn deploy(&mut self) -> Result<()> {
        let order = self.deploy_order.clone();
        for node_id in &order {
            self.execute_node(node_id)
                .with_context(|| format!("deploying node '{node_id}'"))?;
        }
        Ok(())
    }

    /// Advance the simulation by one tick:
    /// 1. Advance clock
    /// 2. Tick all active simulators (accrue interest/funding, check liquidations)
    /// 3. Fire triggered nodes whose interval has elapsed
    ///
    /// Returns false when the clock is exhausted.
    pub fn tick(&mut self) -> Result<bool> {
        if !self.clock.advance() {
            return Ok(false);
        }

        // Tick all simulators
        let node_ids: Vec<NodeId> = self.simulators.keys().cloned().collect();
        for node_id in &node_ids {
            if let Some(sim) = self.simulators.get_mut(node_id) {
                sim.tick(&self.clock)?;
            }
        }

        // Fire triggered nodes
        let triggers = self.triggered_nodes.clone();
        for (node_id, interval) in &triggers {
            if self.should_fire(node_id, interval) {
                self.execute_node(node_id)
                    .with_context(|| format!("firing triggered node '{node_id}'"))?;
                self.trigger_last_fired
                    .insert(node_id.clone(), self.clock.current_timestamp());
            }
        }

        Ok(true)
    }

    /// Current total value of the portfolio (all simulator positions + undeployed balances).
    pub fn total_tvl(&self) -> f64 {
        let sim_value: f64 = self
            .simulators
            .values()
            .map(|s| s.total_value(&self.clock))
            .sum();
        let balance_value = self.balances.total_value();
        sim_value + balance_value
    }

    /// Execute a single node: gather inputs, call simulator (or optimizer), distribute outputs.
    fn execute_node(&mut self, node_id: &str) -> Result<()> {
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
        if let Node::Optimizer { ref id, drift_threshold, .. } = node {
            return self.execute_optimizer(&node, id, input_amount, drift_threshold);
        }

        // Normal node: call simulator
        if let Some(sim) = self.simulators.get_mut(node_id) {
            let result = sim.execute(&node, input_amount, &self.clock)?;
            self.distribute_result(node_id, &input_token, result)?;
        } else {
            // No simulator (e.g. wallet during deploy) — just pass through balances
            // The input was already added to this node's balance by gather_inputs
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
                // Remove the input token balance (was consumed by simulator)
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
            ExecutionResult::Allocations(_) => {
                // Should not happen here — optimizer handled separately
            }
            ExecutionResult::Noop => {}
        }
        Ok(())
    }

    /// Execute an optimizer node: compute Kelly allocations, distribute capital to targets.
    fn execute_optimizer(
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
            let current_values: Vec<(NodeId, f64)> = alloc_result
                .allocations
                .iter()
                .map(|(target_id, _)| {
                    let value = self
                        .simulators
                        .get(target_id)
                        .map(|s| s.total_value(&self.clock))
                        .unwrap_or(0.0);
                    (target_id.clone(), value)
                })
                .collect();

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

            // Execute the target node's simulator
            let target_node = self
                .workflow
                .nodes
                .iter()
                .find(|n| n.id() == target_id)
                .cloned();

            if let Some(ref target_node) = target_node {
                if let Some(sim) = self.simulators.get_mut(target_id.as_str()) {
                    let result = sim.execute(target_node, amount, &self.clock)?;
                    self.distribute_result(target_id, &token, result)?;
                }
            }
        }

        Ok(())
    }

    fn should_fire(&self, node_id: &str, interval: &CronInterval) -> bool {
        let last_fired = self.trigger_last_fired.get(node_id).copied().unwrap_or(0);
        let current = self.clock.current_timestamp();
        let elapsed = current.saturating_sub(last_fired);
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
