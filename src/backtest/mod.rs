pub mod metrics;
pub mod monte_carlo;
pub mod result;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::data;
use crate::engine::clock::SimClock;
use crate::engine::Engine;
use crate::model::node::Node;
use crate::validate;
use crate::venues::{self, BuildMode};

use metrics::BacktestMetrics;
use result::BacktestResult;

/// Top-level JSON output for `--output`.
#[derive(Serialize)]
pub struct BacktestOutput {
    pub historical: BacktestResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monte_carlo: Option<MonteCarloOutput>,
}

/// Monte Carlo section of JSON output.
#[derive(Serialize)]
pub struct MonteCarloOutput {
    pub n_simulations: usize,
    pub simulations: Vec<BacktestResult>,
}

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

    let mc_output = if let Some(ref mc_config) = config.monte_carlo {
        BacktestResult::print_table(&[historical.clone()]);
        let mc_result = monte_carlo::run(config, mc_config, historical.clone())?;
        monte_carlo::print_results(&mc_result);
        Some(MonteCarloOutput {
            n_simulations: mc_result.simulations.len(),
            simulations: mc_result.simulations,
        })
    } else {
        BacktestResult::print_table(&[historical.clone()]);
        None
    };

    if let Some(ref output_path) = config.output {
        let output = BacktestOutput {
            historical,
            monte_carlo: mc_output,
        };
        let file = std::fs::File::create(output_path)
            .with_context(|| format!("creating output file {}", output_path.display()))?;
        serde_json::to_writer_pretty(file, &output)
            .context("writing JSON output")?;
        println!("  JSON output written to {}", output_path.display());
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
    bt_metrics.record_tick(clock.current_timestamp(), initial_tvl);

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
        bt_metrics.record_tick(now, tvl);
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
/// Only sets the balance tracker — the wallet venue stays at zero.
/// Deploy phase will transfer the balance through edges to downstream venues.
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
    }
}

fn estimate_periods_per_year(clock: &SimClock) -> f64 {
    let n = clock.total_ticks();
    if n < 2 {
        return 365.0; // default daily
    }
    let duration_secs = (clock.last_timestamp() - clock.first_timestamp()) as f64;
    if duration_secs <= 0.0 {
        return 365.0;
    }
    let avg_period_secs = duration_secs / (n - 1) as f64;
    const SECS_PER_YEAR: f64 = 365.25 * 24.0 * 3600.0;
    SECS_PER_YEAR / avg_period_secs
}
