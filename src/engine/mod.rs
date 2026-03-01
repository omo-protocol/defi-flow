pub mod clock;
pub mod optimizer;
pub mod reserve;
pub mod state;
pub mod topo;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::model::amount::Amount;
use crate::model::node::{CronInterval, Node, NodeId, SpotSide, Trigger};
use crate::model::workflow::Workflow;
use crate::venues::{ExecutionResult, RiskParams, SimMetrics, Venue};

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

    /// Snapshot of balances before deploy, so percentage edges resolve against
    /// the original balance (not the depleted one after earlier edges fire).
    edge_balance_snapshots: HashMap<(NodeId, String), f64>,
}

impl Engine {
    pub fn new(workflow: Workflow, venues: HashMap<NodeId, Box<dyn Venue>>) -> Self {
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
            edge_balance_snapshots: HashMap::new(),
        }
    }

    /// Return the deploy order (for diagnostic printing).
    pub fn deploy_order(&self) -> &[NodeId] {
        &self.deploy_order
    }

    /// Run the one-time deploy phase: execute non-triggered nodes in topological order.
    pub async fn deploy(&mut self) -> Result<()> {
        self.snapshot_deploy_balances();
        let order = self.deploy_order.clone();
        for node_id in &order {
            self.execute_node(node_id)
                .await
                .with_context(|| format!("deploying node '{node_id}'"))?;
        }
        self.edge_balance_snapshots.clear();
        Ok(())
    }

    /// Snapshot balances for all source nodes that have outgoing percentage edges.
    /// This ensures each percentage edge resolves against the original balance,
    /// not the depleted one after earlier edges have already deducted.
    fn snapshot_deploy_balances(&mut self) {
        self.edge_balance_snapshots.clear();
        for edge in &self.workflow.edges {
            if matches!(edge.amount, Amount::Percentage { .. }) {
                let key = (edge.from_node.clone(), edge.token.clone());
                self.edge_balance_snapshots
                    .entry(key)
                    .or_insert_with(|| self.balances.get(&edge.from_node, &edge.token));
            }
        }
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
            m.rewards_pnl += vm.rewards_pnl;
            m.premium_pnl += vm.premium_pnl;
            m.lp_fees += vm.lp_fees;
            m.lending_interest += vm.lending_interest;
            m.swap_costs += vm.swap_costs;
            m.liquidations += vm.liquidations;
        }
        m
    }

    /// Check if a new workflow is structurally compatible (same nodes, types, edges).
    /// Parameter-only changes are compatible; adding/removing nodes or edges is not.
    pub fn is_structurally_compatible(&self, new_workflow: &Workflow) -> bool {
        if self.workflow.nodes.len() != new_workflow.nodes.len()
            || self.workflow.edges.len() != new_workflow.edges.len()
        {
            return false;
        }

        // Same node IDs and types
        let current_nodes: HashMap<&str, &str> = self
            .workflow
            .nodes
            .iter()
            .map(|n| (n.id(), n.type_name()))
            .collect();

        for node in &new_workflow.nodes {
            match current_nodes.get(node.id()) {
                Some(&type_name) if type_name == node.type_name() => {}
                _ => return false,
            }
        }

        // Same edge topology
        let current_edges: HashSet<(&str, &str)> = self
            .workflow
            .edges
            .iter()
            .map(|e| (e.from_node.as_str(), e.to_node.as_str()))
            .collect();
        let new_edges: HashSet<(&str, &str)> = new_workflow
            .edges
            .iter()
            .map(|e| (e.from_node.as_str(), e.to_node.as_str()))
            .collect();

        current_edges == new_edges
    }

    /// Replace the workflow with an updated version, preserving venue state.
    /// Recomputes deploy order and triggered nodes from the new workflow.
    /// Returns true if any parameters actually changed.
    pub fn update_workflow(&mut self, new_workflow: Workflow) -> bool {
        if self.workflow == new_workflow {
            return false;
        }

        // Log which nodes changed
        for (old, new) in self.workflow.nodes.iter().zip(new_workflow.nodes.iter()) {
            if old != new {
                println!("[reload]   Node '{}' parameters changed", old.id());
            }
        }
        for (old, new) in self.workflow.edges.iter().zip(new_workflow.edges.iter()) {
            if old != new {
                println!("[reload]   Edge {}->{} changed", old.from_node, old.to_node);
            }
        }

        self.workflow = new_workflow;
        self.deploy_order = topo::deploy_order(&self.workflow);
        self.triggered_nodes = extract_triggered_nodes(&self.workflow);
        true
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
            // Resolve cash token: use input_token if available, else derive from
            // the incoming edge definition (optimizer may fire on cron with 0 input).
            let cash_tok = if !input_token.is_empty() {
                input_token.clone()
            } else {
                self.workflow
                    .edges
                    .iter()
                    .find(|e| e.to_node == node_id)
                    .map(|e| e.token.clone())
                    .unwrap_or_else(|| "USDC".to_string())
            };
            return self
                .execute_optimizer(&node, id, input_amount, drift_threshold, &cash_tok)
                .await;
        }

        // Recovery: if a movement node receives its output token (e.g. after a
        // partial failure where the swap succeeded but downstream failed), skip the
        // swap and pass through. The tokens stay in the node's balance and flow
        // downstream via edges normally.
        if let Node::Movement {
            from_token,
            to_token,
            ..
        } = &node
        {
            if input_token == *to_token && input_token != *from_token && input_amount > 0.0 {
                println!(
                    "  [recovery] {} already holds {}, skipping swap",
                    node_id, to_token
                );
                // Balance is already on this node from gather_inputs — just return
                return Ok(());
            }
        }

        // Normal node: call venue (skip if no input — avoids pointless on-chain txns)
        if input_amount > 0.0 {
            if let Some(venue) = self.venues.get_mut(node_id) {
                let result = venue.execute(&node, input_amount).await?;
                self.distribute_result(node_id, &input_token, result)?;
            }
        }

        // For spot buys with downstream edges, extract held tokens so they can route
        self.extract_spot_for_downstream(node_id, &node).await?;

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
            // For percentage edges, resolve against the snapshot (pre-depletion)
            // so that 50%+50% fan-out actually gives 50/50, not 50/25.
            let resolve_balance = if matches!(edge.amount, Amount::Percentage { .. }) {
                let key = (edge.from_node.clone(), edge.token.clone());
                self.edge_balance_snapshots
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| self.balances.get(&edge.from_node, &edge.token))
            } else {
                self.balances.get(&edge.from_node, &edge.token)
            };
            let amount = state::resolve(resolve_balance, &edge.amount);
            // Cap at actually available balance
            let available = self.balances.get(&edge.from_node, &edge.token);
            let amount = amount.min(available);
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

    /// Gather alpha_stats from all venues for adaptive Kelly.
    fn gather_venue_stats(&self) -> HashMap<NodeId, (f64, f64)> {
        let mut stats = HashMap::new();
        for (node_id, venue) in &self.venues {
            if let Some(s) = venue.alpha_stats() {
                stats.insert(node_id.clone(), s);
            }
        }
        stats
    }

    /// Gather risk_params from all venues for smooth Kelly.
    fn gather_venue_risks(&self) -> HashMap<NodeId, RiskParams> {
        let mut risks = HashMap::new();
        for (node_id, venue) in &self.venues {
            if let Some(r) = venue.risk_params() {
                risks.insert(node_id.clone(), r);
            }
        }
        risks
    }

    /// Execute an optimizer node: compute Kelly allocations and rebalance.
    ///
    /// Group-aware rebalancing:
    /// 1. Compute total portfolio value (all venue positions + optimizer cash)
    /// 2. Compute Kelly target fractions per group
    /// 3. Check drift at the GROUP level (not individual legs)
    /// 4. Unwind over-allocated groups → all legs shrink proportionally
    /// 5. Deploy to under-allocated groups → all legs grow equally
    ///
    /// This preserves hedges: a delta-neutral group (spot+perp) never has
    /// capital moved between legs, only scaled as a whole.
    /// Optimizer margin threshold: when a perp in a hedged group has margin ratio
    /// below this, the optimizer pulls from lending donors to add margin.
    /// Higher than the emergency threshold — this is the "smooth" rebalance.
    const OPTIMIZER_MARGIN_THRESHOLD: f64 = 0.50;
    /// Target margin ratio the optimizer aims for after topping up.
    const OPTIMIZER_MARGIN_TARGET: f64 = 0.80;

    async fn execute_optimizer(
        &mut self,
        node: &Node,
        node_id: &str,
        _input_amount: f64,
        drift_threshold: f64,
        cash_token: &str,
    ) -> Result<()> {
        // Gather adaptive stats and risk params
        let venue_stats = self.gather_venue_stats();
        let venue_risks = self.gather_venue_risks();

        // Optimizer's available cash (uses wallet token, e.g. "USDT0")
        let optimizer_balance = self.balances.get(node_id, cash_token);

        // Compute group-level Kelly allocations
        let alloc_result =
            optimizer::compute_kelly_allocations(node, 0.0, &venue_stats, &venue_risks)?;

        // Compute current GROUP values
        let mut group_values: Vec<f64> = Vec::new();
        let mut venue_total = 0.0;
        for group in &alloc_result.groups {
            let mut gv = 0.0;
            for target_id in &group.targets {
                gv += self.effective_venue_value(target_id).await;
            }
            group_values.push(gv);
            venue_total += gv;
        }

        let total_portfolio = venue_total + optimizer_balance;
        if total_portfolio <= 0.0 {
            return Ok(());
        }

        // ── Phase 0: Margin health check (asymmetric — only when perp is stressed) ──
        // Like the keeper's PerpSpot optimizer: check margin ratio and pull from
        // lending donors to add margin. Only acts when margin is low (asymmetric),
        // never rebalances the other direction (that's just lower yield, no risk).
        let groups_info = self.collect_optimizer_groups();
        let mut margin_topped_up = false;
        for (perp_ids, donor_ids) in &groups_info {
            for perp_id in perp_ids {
                let ratio = match self.venues.get(perp_id.as_str()) {
                    Some(v) => match v.margin_ratio() {
                        Some(r) if r < Self::OPTIMIZER_MARGIN_THRESHOLD => r,
                        _ => continue,
                    },
                    None => continue,
                };

                let perp_value = self
                    .venues
                    .get(perp_id.as_str())
                    .unwrap()
                    .total_value()
                    .await
                    .unwrap_or(0.0);
                if perp_value <= 0.0 {
                    continue;
                }

                let notional = if ratio > 0.0 {
                    perp_value / ratio
                } else {
                    perp_value * 10.0
                };
                let needed = notional * (Self::OPTIMIZER_MARGIN_TARGET - ratio);
                if needed <= 0.0 {
                    continue;
                }

                // Pull from lending donors (other groups, never hedge legs)
                let mut total_donor_value = 0.0;
                let mut donor_values: Vec<(NodeId, f64)> = Vec::new();
                for did in donor_ids {
                    if let Some(v) = self.venues.get(did.as_str()) {
                        let val = v.total_value().await.unwrap_or(0.0);
                        if val > 0.0 {
                            donor_values.push((did.clone(), val));
                            total_donor_value += val;
                        }
                    }
                }
                if total_donor_value <= 0.0 {
                    continue;
                }

                // Cap at 50% of donor value to avoid draining lending entirely
                let pull_amount = needed.min(total_donor_value * 0.5);
                let mut total_freed = 0.0;
                for (did, dval) in &donor_values {
                    let share = pull_amount * (dval / total_donor_value);
                    let frac = (share / dval).min(0.5);
                    if frac <= 0.0 {
                        continue;
                    }
                    if let Some(venue) = self.venues.get_mut(did.as_str()) {
                        total_freed += venue.unwind(frac).await.unwrap_or(0.0);
                    }
                }

                if total_freed > 0.0 {
                    if let Some(venue) = self.venues.get_mut(perp_id.as_str()) {
                        venue.add_margin(total_freed);
                    }
                    eprintln!(
                        "  [optimizer] {} margin {:.1}% → added ${:.2} from {:?}",
                        perp_id,
                        ratio * 100.0,
                        total_freed,
                        donor_ids,
                    );
                    margin_topped_up = true;
                }
            }
        }

        // If we topped up margin, recompute group values since donor values changed
        if margin_topped_up {
            group_values.clear();
            venue_total = 0.0;
            for group in &alloc_result.groups {
                let mut gv = 0.0;
                for target_id in &group.targets {
                    gv += self.effective_venue_value(target_id).await;
                }
                group_values.push(gv);
                venue_total += gv;
            }
        }

        // ── Drift check at GROUP level ──
        let total_portfolio = venue_total + self.balances.get(node_id, cash_token);
        if total_portfolio <= 0.0 {
            return Ok(());
        }

        if drift_threshold > 0.0 && self.trigger_last_fired.contains_key(node_id) {
            let mut max_drift = 0.0_f64;
            for (group, &group_value) in alloc_result.groups.iter().zip(group_values.iter()) {
                let actual_frac = group_value / total_portfolio;
                max_drift = max_drift.max((actual_frac - group.fraction).abs());
            }
            if max_drift <= drift_threshold && !margin_topped_up {
                return Ok(());
            }
            if max_drift > drift_threshold {
                self.rebalances += 1;
            }
        }

        // Find outgoing edges to determine token per target
        let outgoing_edges: Vec<_> = self
            .workflow
            .edges
            .iter()
            .filter(|e| e.from_node == node_id)
            .cloned()
            .collect();

        // ── Phase 1: Unwind over-allocated GROUPS (all legs shrink proportionally) ──
        for (group, &group_value) in alloc_result.groups.iter().zip(group_values.iter()) {
            let target_value = total_portfolio * group.fraction;

            if group_value > target_value + 1.0 {
                let excess = group_value - target_value;
                let unwind_frac = excess / group_value;

                let freed = self.unwind_group(&group.targets, unwind_frac).await;
                if freed > 0.0 {
                    self.balances.add(node_id, cash_token, freed);
                }
                eprintln!(
                    "  [rebalance] {} over-allocated by ${:.2}, unwound proportionally",
                    group.targets.join("+"),
                    excess,
                );
            }
        }

        // ── Phase 2: Deploy to under-allocated GROUPS (all legs grow equally) ──
        for (group, &group_value) in alloc_result.groups.iter().zip(group_values.iter()) {
            let target_value = total_portfolio * group.fraction;

            if group_value < target_value {
                let deficit = target_value - group_value;
                let available = self.balances.get(node_id, cash_token);
                let amount = deficit.min(available);
                if amount <= 0.0 {
                    continue;
                }

                // Split equally among legs in the group
                let per_leg = amount / group.targets.len().max(1) as f64;

                for target_id in &group.targets {
                    let leg_amount = per_leg.min(self.balances.get(node_id, cash_token));
                    if leg_amount <= 0.0 {
                        continue;
                    }

                    let token = outgoing_edges
                        .iter()
                        .find(|e| e.to_node == *target_id)
                        .map(|e| e.token.clone())
                        .unwrap_or_else(|| cash_token.to_string());

                    self.balances.deduct(node_id, cash_token, leg_amount);
                    self.balances.add(target_id, &token, leg_amount);

                    let target_node = self
                        .workflow
                        .nodes
                        .iter()
                        .find(|n| n.id() == target_id)
                        .cloned();

                    if let Some(ref target_node) = target_node {
                        if let Some(venue) = self.venues.get_mut(target_id.as_str()) {
                            let result = venue.execute(target_node, leg_amount).await?;
                            self.distribute_result(target_id, &token, result)?;
                        }

                        // Extract spot holdings and route downstream
                        self.extract_spot_for_downstream(target_id, target_node)
                            .await?;
                        self.route_spot_downstream(target_id).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Collect optimizer groups: returns (perp_node_ids, donor_node_ids) pairs.
    /// Perps are nodes that have margin_ratio. Donors are targets from OTHER
    /// allocation groups (not the same group — never pull from hedge legs).
    fn collect_optimizer_groups(&self) -> Vec<(Vec<NodeId>, Vec<NodeId>)> {
        let mut results = Vec::new();

        for node in &self.workflow.nodes {
            if let Node::Optimizer { allocations, .. } = node {
                let mut all_perps = Vec::new();
                // Track which group each perp belongs to
                let mut perp_group_indices: Vec<usize> = Vec::new();

                // First pass: find all perps and their group index
                for (gi, alloc) in allocations.iter().enumerate() {
                    for t in &alloc.targets() {
                        let is_perp = self
                            .venues
                            .get(*t)
                            .map(|v| v.margin_ratio().is_some())
                            .unwrap_or(false);
                        if is_perp {
                            all_perps.push(t.to_string());
                            perp_group_indices.push(gi);
                        }
                    }
                }

                if all_perps.is_empty() {
                    continue;
                }

                // Donors = targets from groups that DON'T contain perps
                // (never pull from hedge legs in the same delta-neutral group)
                let perp_groups: std::collections::HashSet<usize> =
                    perp_group_indices.iter().copied().collect();
                let mut donors = Vec::new();
                for (gi, alloc) in allocations.iter().enumerate() {
                    if perp_groups.contains(&gi) {
                        continue; // skip the delta-neutral group
                    }
                    for t in &alloc.targets() {
                        donors.push(t.to_string());
                    }
                }

                results.push((all_perps, donors));
            }
        }

        results
    }

    /// Compute the effective value of a venue, including any downstream venues
    /// connected via non-USDC edges (e.g. spot buy → ETH lending).
    /// This ensures the optimizer sees buy_eth's value as including lend_eth's value
    /// after extraction routes ETH downstream.
    pub async fn effective_venue_value(&self, node_id: &str) -> f64 {
        let mut value = if let Some(venue) = self.venues.get(node_id) {
            venue.total_value().await.unwrap_or(0.0)
        } else {
            0.0
        };

        // Walk ALL downstream edges, adding venue values along the way.
        // Always continue walking — bridge/movement venues exist but are
        // pass-through (value=0), and we need to reach venues behind them.
        // No token filter: USDC is just another token, and optimizer edges
        // originate FROM the optimizer node (not from targets).
        let mut frontier: Vec<String> = vec![node_id.to_string()];
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(node_id.to_string());
        while let Some(current) = frontier.pop() {
            for edge in &self.workflow.edges {
                if edge.from_node == current && !visited.contains(&edge.to_node) {
                    visited.insert(edge.to_node.clone());
                    if let Some(downstream) = self.venues.get(edge.to_node.as_str()) {
                        value += downstream.total_value().await.unwrap_or(0.0);
                    }
                    // Always continue walking downstream
                    frontier.push(edge.to_node.clone());
                }
            }
        }

        value
    }

    /// Unwind all legs of an allocation group by the given fraction, including
    /// downstream venues (e.g. lend_eth under buy_eth). Returns total USD freed.
    /// Chains through pass-through nodes (bridge/movement) to reach actual venues.
    async fn unwind_group(&mut self, targets: &[String], fraction: f64) -> f64 {
        let mut total_freed = 0.0;
        for target_id in targets {
            // Downstream venues first (e.g. lend_eth under buy_eth)
            total_freed += self.unwind_downstream(target_id, fraction).await;
            // Then the target itself
            if let Some(venue) = self.venues.get_mut(target_id.as_str()) {
                total_freed += venue.unwind(fraction).await.unwrap_or(0.0);
            }
        }
        total_freed
    }

    /// Unwind downstream venues reachable from a node.
    /// Chains through all downstream edges (including pass-through venues).
    async fn unwind_downstream(&mut self, node_id: &str, fraction: f64) -> f64 {
        let mut total_freed = 0.0;
        let mut frontier: Vec<String> = vec![node_id.to_string()];
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(node_id.to_string());
        while let Some(current) = frontier.pop() {
            let edges: Vec<_> = self
                .workflow
                .edges
                .iter()
                .filter(|e| e.from_node == current)
                .cloned()
                .collect();
            for edge in &edges {
                if visited.contains(&edge.to_node) {
                    continue;
                }
                visited.insert(edge.to_node.clone());
                if let Some(ds_venue) = self.venues.get_mut(edge.to_node.as_str()) {
                    total_freed += ds_venue.unwind(fraction).await.unwrap_or(0.0);
                }
                // Always continue walking — bridge/movement venues are pass-through
                frontier.push(edge.to_node.clone());
            }
        }
        total_freed
    }

    /// Optimizer-aware unwind: use Kelly allocations to decide which groups
    /// to unwind and how much, preferring to take from low-alpha (over-allocated)
    /// groups. Returns total USD freed.
    ///
    /// Returns `Err` if no optimizer node exists (caller should fall back to
    /// flat pro-rata).
    pub async fn optimizer_unwind(&mut self, deficit: f64) -> Result<f64> {
        // 1. Find the Optimizer node
        let optimizer_node = self
            .workflow
            .nodes
            .iter()
            .find(|n| matches!(n, Node::Optimizer { .. }))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no optimizer node in workflow"))?;

        // 2. Gather stats and compute Kelly allocations
        let venue_stats = self.gather_venue_stats();
        let venue_risks = self.gather_venue_risks();
        let alloc_result =
            optimizer::compute_kelly_allocations(&optimizer_node, 0.0, &venue_stats, &venue_risks)?;

        if alloc_result.groups.is_empty() {
            anyhow::bail!("optimizer has no allocation groups");
        }

        // 3. Compute current group values via effective_venue_value
        let mut group_values: Vec<f64> = Vec::new();
        let mut total_venue_value = 0.0;
        for group in &alloc_result.groups {
            let mut gv = 0.0;
            for target_id in &group.targets {
                gv += self.effective_venue_value(target_id).await;
            }
            group_values.push(gv);
            total_venue_value += gv;
        }

        if total_venue_value <= 0.0 {
            anyhow::bail!("no venue value to unwind");
        }

        // 4. Post-unwind portfolio target
        let post_unwind_total = (total_venue_value - deficit).max(0.0);

        // 5. Compute excess per group: how much each group exceeds its target
        //    allocation in the smaller (post-unwind) portfolio. Low-alpha groups
        //    have smaller Kelly fractions → more likely to be over-allocated.
        let mut excesses: Vec<f64> = Vec::new();
        let mut total_excess = 0.0;
        for (group, &gv) in alloc_result.groups.iter().zip(group_values.iter()) {
            let target_in_smaller = post_unwind_total * group.fraction;
            let excess = (gv - target_in_smaller).max(0.0);
            excesses.push(excess);
            total_excess += excess;
        }

        // 6-7. Compute unwind amounts per group
        let mut unwind_amounts: Vec<f64> = vec![0.0; alloc_result.groups.len()];

        if total_excess >= deficit {
            // Enough excess to cover deficit — scale proportionally
            let scale = deficit / total_excess;
            for (i, excess) in excesses.iter().enumerate() {
                unwind_amounts[i] = excess * scale;
            }
        } else {
            // Take all excess, then pro-rata the remainder across all groups
            let remainder = deficit - total_excess;
            for (i, excess) in excesses.iter().enumerate() {
                unwind_amounts[i] = *excess;
            }
            for (i, &gv) in group_values.iter().enumerate() {
                if total_venue_value > 0.0 {
                    unwind_amounts[i] += remainder * (gv / total_venue_value);
                }
            }
        }

        // 8. Execute unwinds per group
        let mut total_freed = 0.0;
        for (i, group) in alloc_result.groups.iter().enumerate() {
            let amount = unwind_amounts[i];
            let gv = group_values[i];
            if amount <= 0.0 || gv <= 0.0 {
                continue;
            }
            let unwind_frac = (amount / gv).min(1.0);
            let freed = self.unwind_group(&group.targets, unwind_frac).await;
            total_freed += freed;
            eprintln!(
                "[reserve] optimizer unwind: {} → ${:.2} ({:.1}%)",
                group.targets.join("+"),
                freed,
                unwind_frac * 100.0,
            );
        }

        Ok(total_freed)
    }

    /// For spot buy nodes with downstream edges expecting the base token,
    /// extract the held amount from the venue and place it on the node's balance
    /// so the edge system can route it to downstream nodes (e.g. lending).
    async fn extract_spot_for_downstream(&mut self, node_id: &str, node: &Node) -> Result<()> {
        if let Node::Spot {
            pair,
            side: SpotSide::Buy,
            ..
        } = node
        {
            let base = pair.split('/').next().unwrap_or("ETH").to_string();

            // Only extract if there's a downstream edge for the base token
            let has_downstream = self
                .workflow
                .edges
                .iter()
                .any(|e| e.from_node == node_id && e.token == base);

            if has_downstream {
                if let Some(venue) = self.venues.get(node_id) {
                    let value = venue.total_value().await.unwrap_or(0.0);
                    if value > 0.0 {
                        // Unwind 100% — moves tokens from venue internal state to balances
                        let venue = self.venues.get_mut(node_id).unwrap();
                        let freed = venue.unwind(1.0).await.unwrap_or(0.0);
                        if freed > 0.0 {
                            self.balances.add(node_id, &base, freed);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Route extracted spot tokens to downstream nodes.
    /// Used by the optimizer which bypasses the normal deploy flow.
    /// Chains through pass-through nodes (e.g. movement/bridge nodes that
    /// have no venue) so tokens reach the actual destination venue.
    async fn route_spot_downstream(&mut self, source_id: &str) -> Result<()> {
        // Walk from source through pass-through nodes until we hit a venue
        let mut frontier: Vec<(String, String)> = vec![]; // (from_node, token)
        frontier.push((source_id.to_string(), String::new()));

        // Seed with all downstream edges from the source
        let initial_edges: Vec<_> = self
            .workflow
            .edges
            .iter()
            .filter(|e| e.from_node == source_id)
            .cloned()
            .collect();

        // Process each edge, chaining through pass-through nodes
        let mut to_process = initial_edges;
        while let Some(edge) = to_process.pop() {
            let available = self.balances.get(&edge.from_node, &edge.token);
            let amount = state::resolve(available, &edge.amount);
            if amount <= 0.0 {
                continue;
            }

            self.balances.deduct(&edge.from_node, &edge.token, amount);
            self.balances.add(&edge.to_node, &edge.token, amount);

            let downstream_node = self
                .workflow
                .nodes
                .iter()
                .find(|n| n.id() == edge.to_node)
                .cloned();

            if let Some(ref dn) = downstream_node {
                if let Some(venue) = self.venues.get_mut(edge.to_node.as_str()) {
                    let result = venue.execute(dn, amount).await?;
                    self.distribute_result(&edge.to_node, &edge.token, result)?;
                }
                // Always continue routing downstream — bridge/movement venues
                // produce TokenOutput that lands in balances, then downstream
                // edges pick it up and forward to the next venue.
                let next_edges: Vec<_> = self
                    .workflow
                    .edges
                    .iter()
                    .filter(|e| e.from_node == edge.to_node)
                    .cloned()
                    .collect();
                to_process.extend(next_edges);
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
        CronInterval::Every10m => 600,
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
        | Node::Movement { trigger, .. }
        | Node::Lending { trigger, .. }
        | Node::Vault { trigger, .. }
        | Node::Pendle { trigger, .. }
        | Node::Lp { trigger, .. }
        | Node::Optimizer { trigger, .. } => trigger.as_ref(),
        Node::Wallet { .. } => None,
    }
}
