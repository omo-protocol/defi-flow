pub mod config;
pub mod scheduler;
pub mod state;

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::engine::Engine;
use crate::model::workflow::Workflow;
use crate::venues::{self, BuildMode};

use config::RuntimeConfig;
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
    println!(
        "Workflow: {} ({} nodes, {} edges)",
        workflow.name,
        workflow.nodes.len(),
        workflow.edges.len()
    );
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

    // Build venues using the unified factory
    let venue_map = venues::build_all(&workflow, &BuildMode::Live { config: &config })?;

    // Build engine
    let mut engine = Engine::new(workflow, venue_map);

    // Load or create persistent state
    let mut state = RunState::load_or_new(&config.state_file)?;

    // Restore persisted balances into the engine
    for (node_id, balance) in &state.balances {
        if *balance > 0.0 {
            engine.balances.add(node_id, "USDC", *balance);
        }
    }

    // Deploy phase: execute non-triggered nodes in topological order
    if !state.deploy_completed {
        println!("── Deploy phase ──");
        println!("Deploy order: {:?}", engine.deploy_order());
        engine.deploy().await.context("deploy phase")?;
        state.deploy_completed = true;
        sync_balances(&engine, &mut state);
        state.save(&config.state_file)?;
        println!("Deploy complete. State saved.\n");
    } else {
        println!("Deploy already completed (loaded from state). Skipping.\n");
    }

    // Execution phase
    if config.once {
        println!("── Single pass (--once) ──");
        let mut scheduler = CronScheduler::new(&engine.workflow);
        let triggered = scheduler.get_all_due();
        if triggered.is_empty() {
            println!("No triggered nodes to execute.");
        } else {
            for node_id in &triggered {
                println!("  Execute: {}", node_id);
                engine.execute_node(node_id).await?;
            }
        }
        state.last_tick = chrono::Utc::now().timestamp() as u64;
        sync_balances(&engine, &mut state);
        state.save(&config.state_file)?;

        let tvl = engine.total_tvl().await;
        println!("\nTVL: ${:.2}", tvl);
        println!("State saved. Exiting.");
    } else {
        println!("── Daemon mode ──");
        let mut scheduler = CronScheduler::new(&engine.workflow);

        if !scheduler.has_triggers() {
            println!("WARNING: No triggered nodes in workflow. Nothing to do in daemon mode.");
            println!("Use --once for a single deploy-only pass.");
            return Ok(());
        }

        loop {
            let triggered = scheduler.wait_for_next().await;
            let now = chrono::Utc::now();
            println!(
                "[{}] Triggered: {:?}",
                now.format("%Y-%m-%d %H:%M:%S"),
                triggered
            );

            for node_id in &triggered {
                if let Err(e) = engine.execute_node(node_id).await {
                    eprintln!("  ERROR executing node '{}': {:#}", node_id, e);
                }
            }

            // Tick all venues (accrue interest, update positions, etc.)
            let now_ts = now.timestamp() as u64;
            let dt = now_ts.saturating_sub(state.last_tick) as f64;
            if let Err(e) = engine.tick_venues(now_ts, dt).await {
                eprintln!("  ERROR ticking venues: {:#}", e);
            }

            state.last_tick = now_ts;
            sync_balances(&engine, &mut state);
            state.save(&config.state_file)?;

            let tvl = engine.total_tvl().await;
            println!("[{}] TVL: ${:.2}\n", now.format("%H:%M:%S"), tvl);
        }
    }

    Ok(())
}

/// Sync the engine's per-node balance totals into the persistent RunState.
fn sync_balances(engine: &Engine, state: &mut RunState) {
    state.balances.clear();
    for node in &engine.workflow.nodes {
        let id = node.id().to_string();
        // Sum all token balances for this node into one USD-denominated balance
        let total = engine.balances.node_total(&id);
        if total > 0.0 {
            state.balances.insert(id, total);
        }
    }
}
