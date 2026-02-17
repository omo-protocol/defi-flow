pub mod config;
pub mod executor;
pub mod scheduler;
pub mod state;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::engine::optimizer;
use crate::engine::topo;
use crate::engine::venue::ExecutionResult;
use crate::model::node::{Node, NodeId};
use crate::model::workflow::Workflow;

use config::RuntimeConfig;
use executor::VenueExecutor;
use scheduler::CronScheduler;
use state::RunState;

/// CLI-facing config struct (before env var resolution).
pub struct RunConfig {
    pub network: String,
    pub state_file: PathBuf,
    pub dry_run: bool,
    pub once: bool,
    pub slippage_bps: f64,
}

/// Entry point for the `run` command.
pub fn run(workflow_path: &std::path::Path, cli_config: &RunConfig) -> Result<()> {
    let workflow = crate::validate::load_and_validate(workflow_path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
    })?;

    let config = RuntimeConfig::from_cli(cli_config)?;

    println!("=== defi-flow run ===");
    println!("Workflow: {} ({} nodes, {} edges)", workflow.name, workflow.nodes.len(), workflow.edges.len());
    println!("Network:  {:?}", config.network);
    println!("Wallet:   {:?}", config.wallet_address);
    println!("Dry run:  {}", config.dry_run);
    println!("Once:     {}", config.once);
    println!("Slippage: {} bps", config.slippage_bps);
    println!();

    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    rt.block_on(run_async(workflow, config))
}

async fn run_async(workflow: Workflow, config: RuntimeConfig) -> Result<()> {
    // Install rustls crypto provider (required by ferrofluid's TLS)
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Build executors for all nodes
    let mut executors = executor::build_executors(&workflow, &config)?;

    // Load or create state
    let mut state = RunState::load_or_new(&config.state_file)?;

    // Deploy phase: execute non-triggered nodes in topological order
    if !state.deploy_completed {
        println!("── Deploy phase ──");
        deploy(&workflow, &mut executors, &mut state).await?;
        state.deploy_completed = true;
        state.save(&config.state_file)?;
        println!("Deploy complete. State saved.\n");
    } else {
        println!("Deploy already completed (loaded from state). Skipping.\n");
    }

    // Execution phase
    if config.once {
        println!("── Single pass (--once) ──");
        let mut scheduler = CronScheduler::new(&workflow);
        let triggered = scheduler.get_all_due();
        if triggered.is_empty() {
            println!("No triggered nodes to execute.");
        } else {
            for node_id in &triggered {
                execute_node(&workflow, &mut executors, node_id, &mut state).await?;
            }
        }
        state.last_tick = chrono::Utc::now().timestamp() as u64;
        state.save(&config.state_file)?;

        let tvl = total_tvl(&executors).await;
        println!("\nTVL: ${:.2}", tvl);
        println!("State saved. Exiting.");
    } else {
        println!("── Daemon mode ──");
        let mut scheduler = CronScheduler::new(&workflow);

        if !scheduler.has_triggers() {
            println!("WARNING: No triggered nodes in workflow. Nothing to do in daemon mode.");
            println!("Use --once for a single deploy-only pass.");
            return Ok(());
        }

        loop {
            let triggered = scheduler.wait_for_next().await;
            let now = chrono::Utc::now();
            println!("[{}] Triggered: {:?}", now.format("%Y-%m-%d %H:%M:%S"), triggered);

            for node_id in &triggered {
                if let Err(e) = execute_node(&workflow, &mut executors, node_id, &mut state).await {
                    eprintln!("  ERROR executing node '{}': {:#}", node_id, e);
                }
            }

            // Tick all executors
            for (id, exec) in executors.iter_mut() {
                if let Err(e) = exec.tick().await {
                    eprintln!("  ERROR ticking executor '{}': {:#}", id, e);
                }
            }

            state.last_tick = now.timestamp() as u64;
            state.save(&config.state_file)?;

            let tvl = total_tvl(&executors).await;
            println!("[{}] TVL: ${:.2}\n", now.format("%H:%M:%S"), tvl);
        }
    }

    Ok(())
}

/// Deploy phase: walk non-triggered nodes in topological order.
async fn deploy(
    workflow: &Workflow,
    executors: &mut HashMap<NodeId, Box<dyn VenueExecutor>>,
    state: &mut RunState,
) -> Result<()> {
    let order = topo::deploy_order(workflow);
    println!("Deploy order: {:?}", order);

    for node_id in &order {
        let node = workflow
            .nodes
            .iter()
            .find(|n| n.id() == node_id)
            .cloned()
            .with_context(|| format!("node '{node_id}' not found"))?;

        // Gather inputs from edges
        let input_amount = gather_inputs(workflow, state, node_id);

        println!(
            "  Deploy: {} ({}) — input ${:.2}",
            node_id,
            node.label(),
            input_amount
        );

        // Handle optimizer specially
        if let Node::Optimizer { ref id, drift_threshold, .. } = node {
            execute_optimizer_live(workflow, executors, state, &node, id, input_amount, drift_threshold).await?;
            continue;
        }

        // Normal node
        if let Some(exec) = executors.get_mut(node_id.as_str()) {
            let result = exec.execute(&node, input_amount).await?;
            distribute_result(workflow, state, node_id, result);
        }
    }

    Ok(())
}

/// Execute a single triggered node.
async fn execute_node(
    workflow: &Workflow,
    executors: &mut HashMap<NodeId, Box<dyn VenueExecutor>>,
    node_id: &str,
    state: &mut RunState,
) -> Result<()> {
    let node = workflow
        .nodes
        .iter()
        .find(|n| n.id() == node_id)
        .cloned()
        .with_context(|| format!("node '{node_id}' not found"))?;

    let input_amount = gather_inputs(workflow, state, node_id);

    println!(
        "  Execute: {} ({}) — input ${:.2}",
        node_id,
        node.label(),
        input_amount
    );

    // Handle optimizer
    if let Node::Optimizer { ref id, drift_threshold, .. } = node {
        return execute_optimizer_live(workflow, executors, state, &node, id, input_amount, drift_threshold).await;
    }

    // Normal node
    if let Some(exec) = executors.get_mut(node_id) {
        let result = exec.execute(&node, input_amount).await?;
        distribute_result(workflow, state, node_id, result);
    }

    Ok(())
}

/// Gather input amounts from edges leading to this node.
fn gather_inputs(
    workflow: &Workflow,
    state: &mut RunState,
    node_id: &str,
) -> f64 {
    let mut total = 0.0;

    for edge in &workflow.edges {
        if edge.to_node != node_id {
            continue;
        }

        let available = state.get_balance(&edge.from_node);
        let amount = resolve_amount(available, &edge.amount);

        if amount > 0.0 {
            state.deduct_balance(&edge.from_node, amount);
            state.add_balance(node_id, amount);
            total += amount;
        }
    }

    total
}

/// Resolve an Amount against a balance.
fn resolve_amount(balance: f64, amount: &crate::model::amount::Amount) -> f64 {
    match amount {
        crate::model::amount::Amount::Fixed { value } => {
            value.parse::<f64>().unwrap_or(0.0)
        }
        crate::model::amount::Amount::Percentage { value } => {
            balance * (value / 100.0)
        }
        crate::model::amount::Amount::All => balance,
    }
}

/// Distribute execution results along outgoing edges.
fn distribute_result(
    workflow: &Workflow,
    state: &mut RunState,
    node_id: &str,
    result: ExecutionResult,
) {
    match result {
        ExecutionResult::TokenOutput { amount, .. } => {
            // Output replaces the node's balance
            state.add_balance(node_id, amount);
        }
        ExecutionResult::PositionUpdate { consumed, output } => {
            state.deduct_balance(node_id, consumed);
            if let Some((_token, amount)) = output {
                state.add_balance(node_id, amount);
            }
        }
        ExecutionResult::Allocations(_) => {
            // Handled by execute_optimizer_live
        }
        ExecutionResult::Noop => {}
    }
}

/// Execute an optimizer node in live mode.
async fn execute_optimizer_live(
    workflow: &Workflow,
    executors: &mut HashMap<NodeId, Box<dyn VenueExecutor>>,
    state: &mut RunState,
    node: &Node,
    node_id: &str,
    input_amount: f64,
    drift_threshold: f64,
) -> Result<()> {
    let existing = state.get_balance(node_id);
    let total_capital = existing + input_amount;

    if total_capital <= 0.0 {
        return Ok(());
    }

    // Check drift before rebalancing (if threshold set and not first run)
    if drift_threshold > 0.0 {
        let alloc_result = optimizer::compute_kelly_allocations(node, total_capital)?;
        let mut current_values = Vec::new();
        for (target_id, _) in &alloc_result.allocations {
            let value = if let Some(exec) = executors.get(target_id.as_str()) {
                exec.total_value().await.unwrap_or(0.0)
            } else {
                0.0
            };
            current_values.push((target_id.clone(), value));
        }

        if !optimizer::should_rebalance(&current_values, &alloc_result.allocations, drift_threshold) {
            println!("  Optimizer: drift below threshold, skipping rebalance");
            return Ok(());
        }
    }

    // Compute allocations
    let alloc_result = optimizer::compute_kelly_allocations(node, total_capital)?;
    state.deduct_balance(node_id, total_capital);

    println!("  Optimizer: distributing ${:.2} across {} targets", total_capital, alloc_result.allocations.len());

    for (target_id, fraction) in &alloc_result.allocations {
        let amount = total_capital * fraction;
        if amount <= 0.0 {
            continue;
        }

        println!("    → {} gets ${:.2} ({:.1}%)", target_id, amount, fraction * 100.0);
        state.add_balance(target_id, amount);

        // Execute the target node
        let target_node = workflow
            .nodes
            .iter()
            .find(|n| n.id() == target_id)
            .cloned();

        if let Some(ref target_node) = target_node {
            if let Some(exec) = executors.get_mut(target_id.as_str()) {
                let result = exec.execute(target_node, amount).await?;
                distribute_result(workflow, state, target_id, result);
            }
        }
    }

    Ok(())
}

/// Sum total value across all executors.
async fn total_tvl(executors: &HashMap<NodeId, Box<dyn VenueExecutor>>) -> f64 {
    let mut total = 0.0;
    for exec in executors.values() {
        total += exec.total_value().await.unwrap_or(0.0);
    }
    total
}
