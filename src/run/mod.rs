pub mod config;
pub mod registry;
pub mod scheduler;
pub mod state;
pub mod valuer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::engine::Engine;
use crate::engine::reserve;
use crate::model::workflow::Workflow;
use crate::venues::{self, BuildMode};

use config::RuntimeConfig;
use registry::{Registry, RegistryEntry};
use scheduler::CronScheduler;
use state::RunState;

/// CLI-facing config struct (before env var resolution).
pub struct RunConfig {
    pub network: String,
    pub state_file: PathBuf,
    pub dry_run: bool,
    pub once: bool,
    pub slippage_bps: f64,
    pub log_file: Option<PathBuf>,
    pub registry_dir: Option<PathBuf>,
}

/// Entry point for the `run` command.
pub fn run(workflow_path: &Path, cli_config: &RunConfig) -> Result<()> {
    let workflow = crate::validate::load_and_validate(workflow_path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
    })?;

    let config = RuntimeConfig::from_cli(cli_config)?;

    // Ensure log file parent dir exists
    if let Some(ref log_path) = cli_config.log_file {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
    }

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

    // Register in daemon registry
    let strategy_name = workflow.name.clone();
    let registry_dir = cli_config.registry_dir.clone();
    let reg_dir_ref = registry_dir.as_deref();

    let log_file = cli_config.log_file.clone().unwrap_or_else(|| {
        let dir = registry_dir
            .clone()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".defi-flow"));
        dir.join("logs").join(format!("{}.log", strategy_name))
    });

    let entry = RegistryEntry {
        pid: std::process::id(),
        strategy_file: workflow_path
            .canonicalize()
            .unwrap_or_else(|_| workflow_path.to_path_buf()),
        state_file: cli_config.state_file.clone(),
        log_file,
        mode: if config.dry_run {
            "dry-run".into()
        } else {
            "live".into()
        },
        network: cli_config.network.clone(),
        capital: 0.0, // Updated after deploy
        started_at: chrono::Utc::now().to_rfc3339(),
    };

    if let Err(e) = Registry::register(reg_dir_ref, &strategy_name, entry) {
        eprintln!("Warning: failed to register in daemon registry: {:#}", e);
    } else {
        println!("Registered in daemon registry as '{}'", strategy_name);
    }

    // Set up SIGTERM/SIGINT handler — save state but DON'T deregister.
    // Registry entries must survive container restarts so `resume-all` can relaunch.
    // Only `defi-flow stop` (explicit user action) deregisters.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    let name_for_handler = strategy_name.clone();

    ctrlc::set_handler(move || {
        eprintln!("\n[signal] Shutting down '{}' (state will be saved, registry entry kept for resume)...", name_for_handler);
        shutdown_clone.store(true, Ordering::SeqCst);
    })
    .ok();

    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    let result = rt.block_on(run_async(
        workflow,
        config,
        workflow_path,
        shutdown,
        registry_dir.clone(),
    ));

    // Deregister only on normal exit (--once mode, no errors).
    // Signal-based shutdown leaves registry intact for resume-all.
    if result.is_ok() && cli_config.once {
        if let Err(e) = Registry::deregister(reg_dir_ref, &strategy_name) {
            eprintln!(
                "Warning: failed to deregister from daemon registry: {:#}",
                e
            );
        }
    }

    result
}

async fn run_async(
    workflow: Workflow,
    config: RuntimeConfig,
    workflow_path: &Path,
    shutdown: Arc<AtomicBool>,
    registry_dir: Option<PathBuf>,
) -> Result<()> {
    // Install rustls crypto provider (required by ferrofluid's TLS)
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Build venues using the unified factory
    let tokens = workflow.token_manifest();
    let contracts = workflow.contracts.clone().unwrap_or_default();
    let venue_map = venues::build_all(
        &workflow,
        &BuildMode::Live {
            config: &config,
            tokens: &tokens,
            contracts: &contracts,
        },
    )?;

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

    let strategy_name = engine.workflow.name.clone();
    let reg_dir_ref = registry_dir.as_deref();
    let mut valuer_state = valuer::ValuerState::default();

    // Deploy phase: execute non-triggered nodes in topological order
    if !state.deploy_completed {
        println!("── Deploy phase ──");
        println!("Deploy order: {:?}", engine.deploy_order());
        engine.deploy().await.context("deploy phase")?;
        state.deploy_completed = true;
        sync_balances(&engine, &mut state);

        // Record initial capital for performance tracking
        state.initial_capital = state.balances.values().sum();
        state.peak_tvl = state.initial_capital;

        state.save(&config.state_file)?;
        println!("Deploy complete. Capital: ${:.2}. State saved.\n", state.initial_capital);

        // Update registry with deployed capital
        if let Ok(mut reg) = Registry::load(reg_dir_ref) {
            if let Some(entry) = reg.daemons.get_mut(&strategy_name) {
                entry.capital = state.initial_capital;
            }
            let _ = reg.save(reg_dir_ref);
        }
    } else {
        // Backfill initial_capital for old state files (pre-perf-tracking)
        if state.initial_capital == 0.0 {
            state.initial_capital = state.balances.values().sum();
            state.peak_tvl = state.initial_capital;
        }
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

        // Reserve management (--once mode)
        if let Some(rc) = engine.workflow.reserve.clone() {
            match reserve::check_and_manage(
                &mut engine,
                &rc,
                &contracts,
                &tokens,
                &config.private_key,
                config.dry_run,
            )
            .await
            {
                Ok(Some(action)) => {
                    println!(
                        "[reserve] Unwound ${:.2} (deficit ${:.2}, ratio was {:.1}%)",
                        action.freed,
                        action.deficit,
                        action.reserve_ratio * 100.0,
                    );
                    sync_balances(&engine, &mut state);
                    state.reserve_actions.push(action);
                }
                Ok(None) => {}
                Err(e) => eprintln!("[reserve] ERROR: {:#}", e),
            }
        }

        // Update performance metrics
        let tvl = engine.total_tvl().await;
        if tvl > state.peak_tvl {
            state.peak_tvl = tvl;
        }
        let metrics = engine.collect_metrics();
        state.cumulative_funding = metrics.funding_pnl;
        state.cumulative_interest = metrics.lending_interest;
        state.cumulative_rewards = metrics.rewards_pnl;
        state.cumulative_costs = metrics.swap_costs;

        state.save(&config.state_file)?;
        println!("\nTVL: ${:.2}", tvl);

        // Push TVL to onchain valuer (if configured)
        if let Some(ref vc) = engine.workflow.valuer {
            match valuer::maybe_push_value(
                vc, &contracts, &config.private_key, tvl,
                &mut valuer_state, config.dry_run,
            ).await {
                Ok(true) | Ok(false) => {}
                Err(e) => eprintln!("[valuer] ERROR: {:#}", e),
            }
        }

        println!("State saved. Exiting.");
    } else {
        println!("── Daemon mode (hot reload enabled) ──");
        let mut scheduler = CronScheduler::new(&engine.workflow);

        if !scheduler.has_triggers() {
            println!("WARNING: No triggered nodes in workflow. Nothing to do in daemon mode.");
            println!("Use --once for a single deploy-only pass.");
            return Ok(());
        }

        // Set up file watcher for hot reload
        let workflow_path_buf = workflow_path.to_path_buf();
        let workflow_filename = workflow_path
            .file_name()
            .map(|f| f.to_os_string())
            .unwrap_or_default();
        let (_watcher, mut file_rx) = setup_file_watcher(&workflow_path_buf)?;
        println!(
            "  Watching {} for parameter changes...",
            workflow_path.display()
        );

        loop {
            if shutdown.load(Ordering::SeqCst) {
                println!("[shutdown] Saving state and exiting...");
                state.save(&config.state_file)?;
                break;
            }

            tokio::select! {
                triggered = scheduler.wait_for_next() => {
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

                    // Reserve management: check vault reserve and unwind if depleted
                    if let Some(rc) = engine.workflow.reserve.clone() {
                        match reserve::check_and_manage(
                            &mut engine, &rc, &contracts, &tokens, &config.private_key, config.dry_run,
                        ).await {
                            Ok(Some(action)) => {
                                println!(
                                    "[reserve] Unwound ${:.2} (deficit ${:.2}, ratio was {:.1}%)",
                                    action.freed, action.deficit,
                                    action.reserve_ratio * 100.0,
                                );
                                sync_balances(&engine, &mut state);
                                state.reserve_actions.push(action);
                            }
                            Ok(None) => {} // Reserve healthy
                            Err(e) => eprintln!("[reserve] ERROR: {:#}", e),
                        }
                    }

                    // Update performance metrics
                    let tvl = engine.total_tvl().await;
                    if tvl > state.peak_tvl {
                        state.peak_tvl = tvl;
                    }
                    let metrics = engine.collect_metrics();
                    state.cumulative_funding = metrics.funding_pnl;
                    state.cumulative_interest = metrics.lending_interest;
                    state.cumulative_rewards = metrics.rewards_pnl;
                    state.cumulative_costs = metrics.swap_costs;

                    state.save(&config.state_file)?;
                    println!("[{}] TVL: ${:.2}\n", now.format("%H:%M:%S"), tvl);

                    // Push TVL to onchain valuer (if configured)
                    if let Some(ref vc) = engine.workflow.valuer {
                        match valuer::maybe_push_value(
                            vc, &contracts, &config.private_key, tvl,
                            &mut valuer_state, config.dry_run,
                        ).await {
                            Ok(true) | Ok(false) => {}
                            Err(e) => eprintln!("[valuer] ERROR: {:#}", e),
                        }
                    }
                }
                Some(changed_path) = file_rx.recv() => {
                    // Only reload if the changed file matches our workflow file
                    let matches = changed_path
                        .file_name()
                        .map(|f| f == workflow_filename)
                        .unwrap_or(false);

                    if !matches {
                        continue;
                    }

                    // Debounce: drain queued events and wait for writes to settle
                    while file_rx.try_recv().is_ok() {}
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    while file_rx.try_recv().is_ok() {}

                    match try_reload_workflow(&workflow_path_buf, &mut engine) {
                        Ok(true) => {
                            // Rebuild scheduler with potentially new trigger intervals
                            scheduler = CronScheduler::new(&engine.workflow);
                            println!("[reload] Workflow parameters updated successfully.");
                        }
                        Ok(false) => {}
                        Err(e) => {
                            eprintln!("[reload] Failed to reload workflow: {:#}", e);
                        }
                    }
                }
            }
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

// ── File watcher ─────────────────────────────────────────────────────

/// Set up a file watcher that sends change events to a tokio channel.
/// Watches the parent directory to catch atomic saves (vim, emacs).
fn setup_file_watcher(
    workflow_path: &Path,
) -> Result<(RecommendedWatcher, mpsc::Receiver<PathBuf>)> {
    let (tx, rx) = mpsc::channel::<PathBuf>(4);

    let watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    for path in event.paths {
                        let _ = tx.try_send(path);
                    }
                }
            }
        },
        notify::Config::default(),
    )
    .context("creating file watcher")?;

    // Watch the parent directory (handles atomic renames by editors)
    let watch_dir = workflow_path.parent().unwrap_or(Path::new("."));
    let mut w = watcher;
    w.watch(watch_dir, RecursiveMode::NonRecursive)
        .context("watching workflow directory")?;

    Ok((w, rx))
}

// ── Reload logic ─────────────────────────────────────────────────────

/// Attempt to reload the workflow JSON and apply parameter changes.
/// Returns Ok(true) if changes were applied, Ok(false) if no meaningful changes.
fn try_reload_workflow(path: &Path, engine: &mut Engine) -> Result<bool> {
    // Re-read and re-validate
    let new_workflow = crate::validate::load_and_validate(path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Validation failed:\n  {}", msgs.join("\n  "))
    })?;

    // Structural check: same nodes, same edges, same types
    if !engine.is_structurally_compatible(&new_workflow) {
        println!(
            "[reload] Structural changes detected (nodes/edges/types changed). \
             Skipping — restart required."
        );
        return Ok(false);
    }

    // Apply parameter updates
    let changed = engine.update_workflow(new_workflow);
    Ok(changed)
}

// ── Helpers ──────────────────────────────────────────────────────────

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
