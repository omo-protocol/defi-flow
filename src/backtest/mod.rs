pub mod metrics;
pub mod monte_carlo;
pub mod result;

use anyhow::{Context, Result};

use crate::data;
use crate::engine::clock::SimClock;
use crate::engine::Engine;
use crate::model::node::Node;
use crate::validate;
use crate::venues::{self, BuildMode};

use metrics::BacktestMetrics;
use result::BacktestResult;

/// Configuration for a backtest run.
pub struct BacktestConfig {
    pub workflow_path: std::path::PathBuf,
    pub data_dir: std::path::PathBuf,
    pub capital: f64,
    pub slippage_bps: f64,
    pub seed: u64,
    pub verbose: bool,
    pub output: Option<std::path::PathBuf>,
    pub monte_carlo: Option<monte_carlo::MonteCarloConfig>,
}

/// Run a backtest from the CLI.
pub fn run(config: &BacktestConfig) -> Result<()> {
    let historical = run_single_backtest(config)?;

    if let Some(ref mc_config) = config.monte_carlo {
        // Print historical first, then run MC
        BacktestResult::print_table(&[historical.clone()]);
        let mc_result = monte_carlo::run(config, mc_config, historical)?;
        monte_carlo::print_results(&mc_result);
    } else {
        BacktestResult::print_table(&[historical]);
    }

    // Optional JSON output
    if let Some(ref output_path) = config.output {
        let _ = output_path;
        println!("  (JSON output: not yet implemented)");
    }

    Ok(())
}

/// Run a single backtest and return the result (used by both historical and MC paths).
pub fn run_single_backtest(config: &BacktestConfig) -> Result<BacktestResult> {
    // 1. Load and validate workflow
    let workflow = validate::load_and_validate(&config.workflow_path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
    })?;

    // 2. Load data manifest
    let manifest = data::load_manifest(&config.data_dir).context("loading data manifest")?;

    // 3. Collect timestamps and build clock
    let timestamps = data::collect_timestamps(&config.data_dir, &manifest)?;
    let clock = if timestamps.is_empty() {
        // No CSV data — create a 365-day daily clock for static simulations
        let start = 1704067200; // 2024-01-01
        SimClock::uniform(start, start + 365 * 86400, 86400)
    } else {
        SimClock::new(timestamps)
    };

    let periods_per_year = estimate_periods_per_year(&clock);

    // 4. Build venues for each node
    let venue_map = venues::build_all(
        &workflow,
        &BuildMode::Backtest {
            manifest: &manifest,
            data_dir: &config.data_dir,
            slippage_bps: config.slippage_bps,
            seed: config.seed,
        },
    )?;

    // 5. Build engine
    let mut engine = Engine::new(workflow, venue_map);

    // 6. Run async phases in a tokio runtime
    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    rt.block_on(execute_backtest(&mut engine, clock, config, periods_per_year))
}

async fn execute_backtest(
    engine: &mut Engine,
    mut clock: SimClock,
    config: &BacktestConfig,
    periods_per_year: f64,
) -> Result<BacktestResult> {
    // Seed the wallet node with initial capital
    seed_wallet(engine, config.capital).await;

    // Deploy phase
    if config.verbose {
        println!("Deploying workflow...");
    }
    engine.deploy().await.context("deploy phase")?;

    let initial_tvl = engine.total_tvl().await;
    let mut bt_metrics = BacktestMetrics::new(initial_tvl, periods_per_year);
    bt_metrics.record_tick(initial_tvl);

    if config.verbose {
        println!("[deploy] TVL = {:.2}", initial_tvl);
    }

    // Tick loop
    let mut tick_count = 0u64;
    while clock.advance() {
        let now = clock.current_timestamp();
        let dt_secs = clock.dt_seconds() as f64;
        engine
            .tick(now, dt_secs)
            .await
            .context("tick phase")?;

        let tvl = engine.total_tvl().await;
        bt_metrics.record_tick(tvl);
        tick_count += 1;

        if config.verbose && tick_count % 100 == 0 {
            println!(
                "[tick {:>6}/{:>6}] TVL = {:.2}",
                clock.tick_index(),
                clock.total_ticks(),
                tvl,
            );
        }
    }

    // Collect venue-specific metrics
    let m = engine.collect_metrics();

    let bt_result = bt_metrics.finalize(
        engine.workflow.name.clone(),
        config.capital,
        engine.rebalances,
        m.liquidations,
        m.funding_pnl,
        m.rewards_pnl,
        m.premium_pnl,
        m.lp_fees,
        m.lending_interest,
        m.swap_costs,
    );

    Ok(bt_result)
}

/// Seed the first wallet node with initial capital.
async fn seed_wallet(engine: &mut Engine, capital: f64) {
    let wallet_node = engine
        .workflow
        .nodes
        .iter()
        .find(|n| matches!(n, Node::Wallet { .. }))
        .cloned();

    if let Some(ref node) = wallet_node {
        let id = node.id().to_string();
        engine.balances.add(&id, "USDC", capital);
        // Also seed the wallet venue so its total_value() reports correctly
        if let Some(venue) = engine.venues.get_mut(id.as_str()) {
            let _ = venue.execute(node, capital).await;
        }
    }
}

fn estimate_periods_per_year(clock: &SimClock) -> f64 {
    if clock.total_ticks() < 2 {
        return 365.0; // default daily
    }
    // Use total timespan to estimate
    365.0 * 3.0 // default: 8-hour periods
}
