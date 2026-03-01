pub mod config;
pub mod hl_activation;
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
use crate::model::node::Node;
use crate::model::workflow::Workflow;
use crate::venues::{self, evm, BuildMode};

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
    let wallet_tok = wallet_token(&workflow);
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

    let strategy_name = engine.workflow.name.clone();
    let reg_dir_ref = registry_dir.as_deref();
    let mut valuer_state = valuer::ValuerState::default();

    // ── On-chain reconciliation ──
    // Query actual on-chain state to detect stale/wrong state files.
    // Venues already query on-chain in total_value() — no execute() needed.
    reconcile_onchain_state(&mut engine, &mut state, &config, &tokens).await?;

    // ── HyperLiquid wallet activation ──
    // If the strategy has HL perp/spot nodes and the wallet hasn't been activated
    // on HyperCore yet, swap USDT0 → USDC and deposit via CoreDepositWallet.
    if let Err(e) = hl_activation::ensure_hl_wallet(&engine.workflow, &config).await {
        eprintln!("[hl-activate] WARNING: {:#}", e);
        eprintln!("[hl-activate] Strategy may fail if HL wallet is not activated.");
    }

    // Push initial valuation to on-chain valuer immediately after reconciliation.
    // Must happen BEFORE deploy/allocator since totalAssets() needs a valuer report.
    // Prevents chicken-and-egg: totalAssets() needs valuer → valuer push needs tick → tick needs cron.
    if let Some(ref vc) = engine.workflow.valuer {
        let mut push_tvl = onchain_tvl(&engine, &config, &tokens).await;

        // Fallback: if TVL is 0 but adapter has allocations, use that as initial value.
        // This bootstraps the valuer when totalAssets() reverts because the valuer has
        // no reports yet (chicken-and-egg between vault ↔ valuer).
        if push_tvl <= 0.0 {
            if let Some(ref rc) = engine.workflow.reserve {
                match reserve::read_adapter_allocations(rc, &contracts).await {
                    Ok(alloc) if alloc > 0.0 => {
                        eprintln!(
                            "[valuer] TVL=0 but adapter has ${:.2} allocated — using as bootstrap value",
                            alloc,
                        );
                        push_tvl = alloc;
                    }
                    _ => {}
                }
            }
        }

        if push_tvl > 0.0 {
            match valuer::maybe_push_value(
                vc, &contracts, &config.private_key, push_tvl,
                &mut valuer_state, config.dry_run,
                engine.workflow.reserve.as_ref(),
            ).await {
                Ok(true) => println!("[valuer] Initial valuation pushed: ${:.2}", push_tvl),
                Ok(false) => {} // Throttled (shouldn't happen on startup)
                Err(e) => eprintln!("[valuer] WARNING: initial push failed: {:#}", e),
            }
        }
    }

    // Deploy phase: execute non-triggered nodes in topological order
    if !state.deploy_completed {
        // Vault strategies: pull funds from vault BEFORE deploy so wallet has capital
        if let Some(rc) = engine.workflow.reserve.clone() {
            if rc.adapter_address.is_some() {
                println!("── Pre-deploy allocation (vault strategy) ──");
                match reserve::check_and_allocate(
                    &rc,
                    engine.workflow.valuer.as_ref(),
                    &contracts,
                    &tokens,
                    &config.private_key,
                    config.wallet_address,
                    config.dry_run,
                )
                .await
                {
                    Ok(Some(record)) => {
                        println!(
                            "[allocator] Pulled ${:.2} from vault (excess=${:.2})",
                            record.pulled, record.excess,
                        );
                        engine.balances.add("wallet", &wallet_tok, record.pulled);
                        sync_balances(&engine, &mut state);
                        state.allocation_actions.push(record);
                    }
                    Ok(None) => println!("[allocator] No excess to pull from vault."),
                    Err(e) => eprintln!("[allocator] ERROR: {:#}", e),
                }
            }
        }

        // Tick venues once before deploy so live venues populate alpha stats
        // (needed by the optimizer during the deploy phase).
        let now = chrono::Utc::now().timestamp() as u64;
        engine.tick_venues(now, 0.0).await?;

        println!("── Deploy phase ──");
        println!("Deploy order: {:?}", engine.deploy_order());
        engine.deploy().await.context("deploy phase")?;
        state.deploy_completed = true;
        sync_balances(&engine, &mut state);

        // Record initial capital for performance tracking (use TVL which includes venue positions)
        let deploy_tvl = engine.total_tvl().await;
        state.initial_capital = if deploy_tvl > 0.0 {
            deploy_tvl
        } else {
            state.balances.values().sum()
        };
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
        println!("Deploy already completed (on-chain capital confirmed). Skipping.\n");
    }

    // Execution phase
    if config.once {
        println!("── Single pass (--once) ──");

        // Allocator: pull excess funds from vault before execution
        if let Some(rc) = engine.workflow.reserve.clone() {
            match reserve::check_and_allocate(
                &rc,
                engine.workflow.valuer.as_ref(),
                &contracts,
                &tokens,
                &config.private_key,
                config.wallet_address,
                config.dry_run,
            )
            .await
            {
                Ok(Some(record)) => {
                    println!(
                        "[allocator] Pulled ${:.2} from vault (excess=${:.2})",
                        record.pulled, record.excess,
                    );
                    engine.balances.add("wallet", &wallet_tok, record.pulled);
                    sync_balances(&engine, &mut state);
                    state.allocation_actions.push(record);

                    // Vault strategies: first allocation seeds initial_capital
                    if state.initial_capital == 0.0 {
                        state.initial_capital = state.balances.values().sum();
                        state.peak_tvl = state.initial_capital;
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("[allocator] ERROR: {:#}", e),
            }
        }

        // Tick venues BEFORE executing cron nodes so alpha stats are warm.
        let tick_now = chrono::Utc::now().timestamp() as u64;
        let tick_dt = tick_now.saturating_sub(state.last_tick) as f64;
        engine.tick_venues(tick_now, tick_dt).await?;

        // Recovery pass: check for stranded funds (Bridge2 USDC on Arb, Pendle SY, etc.)
        if let Ok(true) = engine.recovery_pass().await {
            sync_balances(&engine, &mut state);
        }

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
        let tvl = onchain_tvl(&engine, &config, &tokens).await;
        state.last_tvl = tvl;
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
        let push_tvl = tvl;
        if let Some(ref vc) = engine.workflow.valuer {
            match valuer::maybe_push_value(
                vc, &contracts, &config.private_key, push_tvl,
                &mut valuer_state, config.dry_run,
                engine.workflow.reserve.as_ref(),
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

                    // Allocator: pull excess funds from vault before execution
                    if let Some(rc) = engine.workflow.reserve.clone() {
                        match reserve::check_and_allocate(
                            &rc, engine.workflow.valuer.as_ref(),
                            &contracts, &tokens, &config.private_key,
                            config.wallet_address, config.dry_run,
                        ).await {
                            Ok(Some(record)) => {
                                println!(
                                    "[allocator] Pulled ${:.2} from vault (excess=${:.2})",
                                    record.pulled, record.excess,
                                );
                                engine.balances.add("wallet", &wallet_tok, record.pulled);
                                sync_balances(&engine, &mut state);
                                state.allocation_actions.push(record);

                                // Vault strategies: first allocation seeds initial_capital
                                if state.initial_capital == 0.0 {
                                    state.initial_capital = state.balances.values().sum();
                                    state.peak_tvl = state.initial_capital;
                                }
                            }
                            Ok(None) => {}
                            Err(e) => eprintln!("[allocator] ERROR: {:#}", e),
                        }
                    }

                    // Tick venues BEFORE executing cron nodes so alpha stats are warm.
                    // Without this, the optimizer sees 0% return on first tick and unwinds.
                    let now_ts = now.timestamp() as u64;
                    let dt = now_ts.saturating_sub(state.last_tick) as f64;
                    if let Err(e) = engine.tick_venues(now_ts, dt).await {
                        eprintln!("  ERROR ticking venues: {:#}", e);
                    }

                    // Recovery pass: check for stranded funds before executing cron nodes
                    if let Ok(true) = engine.recovery_pass().await {
                        sync_balances(&engine, &mut state);
                    }

                    for node_id in &triggered {
                        if let Err(e) = engine.execute_node(node_id).await {
                            eprintln!("  ERROR executing node '{}': {:#}", node_id, e);
                        }
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
                    let tvl = onchain_tvl(&engine, &config, &tokens).await;
                    state.last_tvl = tvl;
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
                    let push_tvl = tvl;
                    if let Some(ref vc) = engine.workflow.valuer {
                        match valuer::maybe_push_value(
                            vc, &contracts, &config.private_key, push_tvl,
                            &mut valuer_state, config.dry_run,
                            engine.workflow.reserve.as_ref(),
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

// ── On-chain reconciliation ──────────────────────────────────────────

/// Query on-chain state and reconcile with persisted RunState.
///
/// This prevents the strategy from getting stuck on stale state. It:
/// 1. Queries all venue `total_value()` — live venues query on-chain
/// 2. Queries wallet ERC20 balance on-chain
/// 3. Adjusts `deploy_completed` and `initial_capital` based on reality
///
/// Cumulative metrics (funding, interest, costs) are left unchanged — those
/// are the strategy's own tracking and have no on-chain equivalent.
async fn reconcile_onchain_state(
    engine: &mut Engine,
    state: &mut RunState,
    config: &RuntimeConfig,
    tokens: &evm::TokenManifest,
) -> Result<()> {
    println!("── Reconciling on-chain state ──");

    // 1. Query venue on-chain values
    let mut venue_tvl = 0.0;
    for (node_id, venue) in &engine.venues {
        let val = venue.total_value().await.unwrap_or(0.0);
        if val > 0.5 {
            println!("  [reconcile] {} = ${:.2}", node_id, val);
        }
        venue_tvl += val;
    }

    // 2. Query wallet balance for ALL manifest tokens on-chain
    let wallet_tokens = query_wallet_all_tokens(&engine.workflow, config, tokens).await;
    let wallet_balance: f64 = wallet_tokens.iter().map(|(_, b)| b).sum();
    let onchain_tvl = venue_tvl + wallet_balance;

    println!(
        "[reconcile] On-chain TVL: ${:.2} (venues=${:.2}, wallet=${:.2})",
        onchain_tvl, venue_tvl, wallet_balance,
    );

    // 3. Reconcile — distinguish venue capital vs wallet-only capital
    //
    // venue_tvl > 0 → positions exist on-chain → deploy definitely happened
    // venue_tvl = 0 && wallet > 0 → funds in wallet but no positions
    //   could be: allocator ran but deploy crashed, or manual send
    //   → do NOT mark deployed — let deploy phase route wallet→venues
    // both = 0 → nothing on-chain → need fresh allocation + deploy
    if venue_tvl > 1.0 {
        // Venue positions exist — deploy completed for certain
        if !state.deploy_completed {
            println!(
                "[reconcile] On-chain venue capital ${:.2} found — marking deploy as completed",
                venue_tvl,
            );
            state.deploy_completed = true;
        }
        if state.initial_capital <= 0.0 {
            state.initial_capital = onchain_tvl;
            state.peak_tvl = onchain_tvl;
            println!(
                "[reconcile] Set initial_capital = ${:.2} from on-chain TVL",
                onchain_tvl,
            );
        }
    } else if venue_tvl < 1.0 && wallet_balance > 1.0 {
        // Wallet has funds but no venue positions. Either:
        // - deploy_completed=false: allocator ran but deploy crashed
        // - deploy_completed=true: deploy was a no-op (only triggered nodes)
        // Either way, reset deploy and seed wallet so deploy routes to venues.
        if state.deploy_completed {
            println!(
                "[reconcile] Deploy was marked complete but no venue positions (wallet=${:.2}) — resetting for re-deploy",
                wallet_balance,
            );
            state.deploy_completed = false;
        } else {
            println!(
                "[reconcile] Wallet has ${:.2} but no venue positions — will deploy",
                wallet_balance,
            );
        }
        for (tok, bal) in &wallet_tokens {
            engine.balances.add("wallet", tok, *bal);
        }
    } else if state.deploy_completed && onchain_tvl < 1.0 {
        // State says deployed but on-chain is empty — reset for re-deploy
        println!("[reconcile] State says deployed but on-chain TVL is $0 — resetting for re-deploy");
        state.deploy_completed = false;
        state.balances.clear();
    }

    // Save reconciled state
    state.save(&config.state_file)?;
    println!();
    Ok(())
}

/// Query the wallet's balance for ALL tokens in the manifest on-chain.
/// Returns `(symbol, balance)` pairs for every token with balance > $0.01.
/// This catches stranded intermediate tokens after partial deploy failures
/// (e.g. swap succeeded but downstream venue failed → wallet holds output token).
pub(crate) async fn query_wallet_all_tokens(
    workflow: &Workflow,
    config: &RuntimeConfig,
    tokens: &evm::TokenManifest,
) -> Vec<(String, f64)> {
    // Find the wallet node to get its chain
    let chain = match workflow
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Wallet { chain, .. } => Some(chain.clone()),
            _ => None,
        })
    {
        Some(c) => c,
        None => return vec![],
    };

    let rpc_url = match chain.rpc_url() {
        Some(url) => url,
        None => return vec![],
    };

    let mut results = Vec::new();

    for (symbol, chain_map) in tokens.iter() {
        // Resolve token address on the wallet's chain
        let addr = match chain_map
            .get(&chain.name)
            .or_else(|| {
                // Try case-insensitive match
                chain_map
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&chain.name))
                    .map(|(_, v)| v)
            })
            .and_then(|a| a.parse::<alloy::primitives::Address>().ok())
        {
            Some(a) => a,
            None => continue,
        };

        // Skip native token (address 0) — after LiFi swaps, intermediate
        // tokens are always ERC20 (WHYPE, not native HYPE)
        if addr == alloy::primitives::Address::ZERO {
            continue;
        }

        match query_erc20_balance(rpc_url, addr, config.wallet_address).await {
            Ok(bal) if bal > 0.01 => {
                eprintln!("  [reconcile] wallet {} = {:.2}", symbol, bal);
                results.push((symbol.clone(), bal));
            }
            _ => {}
        }
    }

    results
}

/// Query an ERC20 token balance for an address, fetching decimals on-chain.
pub(crate) async fn query_erc20_balance(
    rpc_url: &str,
    token_addr: alloy::primitives::Address,
    wallet_addr: alloy::primitives::Address,
) -> Result<f64> {
    let rp = evm::read_provider(rpc_url)?;
    let token = evm::IERC20::new(token_addr, &rp);
    let decimals = token.decimals().call().await.context("ERC20.decimals")?;
    let balance_raw = token
        .balanceOf(wallet_addr)
        .call()
        .await
        .context("ERC20.balanceOf")?;
    Ok(evm::from_token_units(balance_raw, decimals))
}

/// Compute on-chain TVL: sum of venue positions + wallet token balances.
/// Fully on-chain — no dependency on state.json or engine balances.
/// Scans all manifest tokens so stranded intermediate tokens are included.
async fn onchain_tvl(
    engine: &Engine,
    config: &RuntimeConfig,
    tokens: &evm::TokenManifest,
) -> f64 {
    let mut tvl = 0.0;
    for v in engine.venues.values() {
        tvl += v.total_value().await.unwrap_or(0.0);
    }
    let wallet_tokens = query_wallet_all_tokens(&engine.workflow, config, tokens).await;
    tvl += wallet_tokens.iter().map(|(_, b)| b).sum::<f64>();
    tvl
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Extract the wallet node's token symbol (e.g. "USDT0", "USDC").
/// Falls back to "USDC" if no wallet node exists.
fn wallet_token(workflow: &Workflow) -> String {
    workflow
        .nodes
        .iter()
        .find_map(|n| match n {
            Node::Wallet { token, .. } => Some(token.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "USDC".to_string())
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
