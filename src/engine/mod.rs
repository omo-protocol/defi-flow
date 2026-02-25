pub mod clock;
pub mod optimizer;
pub mod reserve;
pub mod state;
pub mod topo;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

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

        // Protect perp margins — pull from lending/idle if approaching liquidation
        self.protect_margins().await?;

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
                println!(
                    "[reload]   Edge {}->{} changed",
                    old.from_node, old.to_node
                );
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
            return self
                .execute_optimizer(&node, id, input_amount, drift_threshold)
                .await;
        }

        // Normal node: call venue
        if let Some(venue) = self.venues.get_mut(node_id) {
            let result = venue.execute(&node, input_amount).await?;
            self.distribute_result(node_id, &input_token, result)?;
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
    ) -> Result<()> {
        // Gather adaptive stats and risk params
        let venue_stats = self.gather_venue_stats();
        let venue_risks = self.gather_venue_risks();

        // Optimizer's available cash
        let optimizer_balance = self.balances.get(node_id, "USDC");

        // Compute group-level Kelly allocations
        let alloc_result = optimizer::compute_kelly_allocations(
            node, 0.0, &venue_stats, &venue_risks,
        )?;

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

                let perp_value = self.venues.get(perp_id.as_str())
                    .unwrap().total_value().await.unwrap_or(0.0);
                if perp_value <= 0.0 {
                    continue;
                }

                let notional = if ratio > 0.0 { perp_value / ratio } else { perp_value * 10.0 };
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
                        perp_id, ratio * 100.0, total_freed, donor_ids,
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
        let total_portfolio = venue_total + self.balances.get(node_id, "USDC");
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

                for target_id in &group.targets {
                    // Compute the effective value (includes downstream) to determine
                    // how much USD to free from this leg.
                    let eff_value = self.effective_venue_value(target_id).await;
                    let leg_unwind_usd = eff_value * unwind_frac;
                    if leg_unwind_usd <= 0.0 {
                        continue;
                    }

                    // Unwind downstream venues first (e.g. lend_eth under buy_eth).
                    // Lending unwind already returns USD value.
                    let downstream_edges: Vec<_> = self
                        .workflow
                        .edges
                        .iter()
                        .filter(|e| e.from_node == *target_id && e.token != "USDC")
                        .cloned()
                        .collect();
                    for edge in &downstream_edges {
                        if let Some(ds_venue) = self.venues.get_mut(edge.to_node.as_str()) {
                            let freed = ds_venue.unwind(unwind_frac).await.unwrap_or(0.0);
                            if freed > 0.0 {
                                self.balances.add(node_id, "USDC", freed);
                            }
                        }
                    }

                    // Also unwind the spot venue to reduce its held amount proportionally.
                    // SpotSimulator.unwind() returns USD from selling held tokens.
                    if let Some(venue) = self.venues.get_mut(target_id.as_str()) {
                        let freed = venue.unwind(unwind_frac).await.unwrap_or(0.0);
                        if freed > 0.0 {
                            self.balances.add(node_id, "USDC", freed);
                        }
                    }
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
                let available = self.balances.get(node_id, "USDC");
                let amount = deficit.min(available);
                if amount <= 0.0 {
                    continue;
                }

                // Split equally among legs in the group
                let per_leg = amount / group.targets.len().max(1) as f64;

                for target_id in &group.targets {
                    let leg_amount = per_leg.min(self.balances.get(node_id, "USDC"));
                    if leg_amount <= 0.0 {
                        continue;
                    }

                    let token = outgoing_edges
                        .iter()
                        .find(|e| e.to_node == *target_id)
                        .map(|e| e.token.clone())
                        .unwrap_or_else(|| "USDC".to_string());

                    self.balances.deduct(node_id, "USDC", leg_amount);
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

    /// Emergency safety threshold: last-resort margin protection that fires per-tick.
    /// The optimizer handles margin at 50% — this only fires if something slips past.
    /// 0.10 means "emergency top-up when equity is only 10% of position size".
    const MARGIN_SAFETY_THRESHOLD: f64 = 0.10;
    /// Emergency target margin ratio after top-up.
    const MARGIN_TARGET_RATIO: f64 = 0.30;

    /// Check all perp venues for margin health. If any are below the safety
    /// threshold, unwind capital from non-perp venues in the same optimizer
    /// group (lending, spot) and add it as margin.
    async fn protect_margins(&mut self) -> Result<()> {
        // Collect optimizer groups from workflow
        let groups = self.collect_optimizer_groups();

        for (perp_ids, donor_ids) in &groups {
            for perp_id in perp_ids {
                let margin_ratio = match self.venues.get(perp_id.as_str()) {
                    Some(v) => v.margin_ratio(),
                    None => continue,
                };

                let ratio = match margin_ratio {
                    Some(r) if r < Self::MARGIN_SAFETY_THRESHOLD => r,
                    _ => continue,
                };

                // How much margin do we need to reach target?
                let perp_value = self.venues.get(perp_id.as_str())
                    .unwrap().total_value().await.unwrap_or(0.0);
                if perp_value <= 0.0 {
                    continue;
                }

                // notional = equity / margin_ratio, need = notional * (target - current)
                let notional = if ratio > 0.0 { perp_value / ratio } else { perp_value * 10.0 };
                let needed = notional * (Self::MARGIN_TARGET_RATIO - ratio);
                if needed <= 0.0 {
                    continue;
                }

                // Pull from donor venues (lending, etc.) proportionally
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

                let pull_amount = needed.min(total_donor_value * 0.5); // never drain more than 50%
                let mut total_freed = 0.0;

                for (did, dval) in &donor_values {
                    let share = pull_amount * (dval / total_donor_value);
                    let frac = share / dval;
                    if frac <= 0.0 {
                        continue;
                    }
                    if let Some(venue) = self.venues.get_mut(did.as_str()) {
                        let freed = venue.unwind(frac.min(0.5)).await.unwrap_or(0.0);
                        total_freed += freed;
                    }
                }

                if total_freed > 0.0 {
                    if let Some(venue) = self.venues.get_mut(perp_id.as_str()) {
                        venue.add_margin(total_freed);
                    }
                    eprintln!(
                        "  [margin-protect] {} ratio {:.1}% → added ${:.2} margin from {:?}",
                        perp_id, ratio * 100.0, total_freed,
                        donor_ids,
                    );
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
                        let is_perp = self.venues.get(*t)
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

        // Add downstream venue values for non-USDC edges (spot→lending chains)
        for edge in &self.workflow.edges {
            if edge.from_node == node_id && edge.token != "USDC" {
                if let Some(downstream) = self.venues.get(edge.to_node.as_str()) {
                    value += downstream.total_value().await.unwrap_or(0.0);
                }
            }
        }

        value
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
    async fn route_spot_downstream(&mut self, source_id: &str) -> Result<()> {
        let edges: Vec<_> = self
            .workflow
            .edges
            .iter()
            .filter(|e| e.from_node == source_id && e.token != "USDC")
            .cloned()
            .collect();

        for edge in &edges {
            let available = self.balances.get(source_id, &edge.token);
            let amount = state::resolve(available, &edge.amount);
            if amount <= 0.0 {
                continue;
            }

            self.balances.deduct(source_id, &edge.token, amount);
            self.balances.add(&edge.to_node, &edge.token, amount);

            // Execute downstream venue with the routed tokens
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
        | Node::Movement { trigger, .. }
        | Node::Lending { trigger, .. }
        | Node::Vault { trigger, .. }
        | Node::Pendle { trigger, .. }
        | Node::Lp { trigger, .. }
        | Node::Optimizer { trigger, .. } => trigger.as_ref(),
        Node::Wallet { .. } => None,
    }
}
