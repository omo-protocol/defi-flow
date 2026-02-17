pub mod metrics;
pub mod result;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::data;
use crate::data::csv_types;
use crate::engine::clock::SimClock;
use crate::engine::venue::VenueSimulator;
use crate::engine::Engine;
use crate::model::node::{Node, NodeId};
use crate::model::workflow::Workflow;
use crate::sim;
use crate::validate;

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
}

/// Run a backtest from the CLI.
pub fn run(config: &BacktestConfig) -> Result<()> {
    // 1. Load and validate workflow
    let workflow = validate::load_and_validate(&config.workflow_path)
        .map_err(|errors| {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
        })?;

    // 2. Load data manifest
    let manifest = data::load_manifest(&config.data_dir)
        .context("loading data manifest")?;

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

    // 4. Build simulators for each node
    let simulators = build_simulators(
        &workflow,
        &manifest,
        &config.data_dir,
        config.slippage_bps,
        config.seed,
    )?;

    // 5. Build engine
    let mut engine = Engine::new(workflow, simulators, clock);

    // 6. Seed the wallet node with initial capital
    seed_wallet(&mut engine, config.capital);

    // 7. Deploy phase
    if config.verbose {
        println!("Deploying workflow...");
    }
    engine.deploy().context("deploy phase")?;

    let initial_tvl = engine.total_tvl();
    let mut bt_metrics = BacktestMetrics::new(initial_tvl, periods_per_year);
    bt_metrics.record_tick(initial_tvl);

    if config.verbose {
        println!("[deploy] TVL = {:.2}", initial_tvl);
    }

    // 8. Tick loop
    let mut tick_count = 0u64;
    while engine.tick().context("tick phase")? {
        let tvl = engine.total_tvl();
        bt_metrics.record_tick(tvl);
        tick_count += 1;

        if config.verbose && tick_count % 100 == 0 {
            println!(
                "[tick {:>6}/{:>6}] TVL = {:.2}",
                engine.clock.tick_index(),
                engine.clock.total_ticks(),
                tvl,
            );
        }
    }

    // 9. Collect venue-specific metrics from simulators
    let (funding_pnl, premium_pnl, lp_fees, lending_interest, swap_costs, liquidations) =
        collect_venue_metrics(&engine);

    let bt_result = bt_metrics.finalize(
        engine.workflow.name.clone(),
        config.capital,
        engine.rebalances,
        liquidations,
        funding_pnl,
        premium_pnl,
        lp_fees,
        lending_interest,
        swap_costs,
    );

    BacktestResult::print_table(&[bt_result]);

    // 10. Optional JSON output
    if let Some(ref output_path) = config.output {
        // Re-run finalize for the serializable result (we consumed it above for printing)
        // Actually let's just serialize — need to re-create or keep a ref
        // For now, just note we already printed. TODO: serialize to file.
        let _ = output_path; // silence unused warning
        println!("  (JSON output: not yet implemented)");
    }

    Ok(())
}

/// Build a VenueSimulator for each node in the workflow.
fn build_simulators(
    workflow: &Workflow,
    manifest: &HashMap<NodeId, data::ManifestEntry>,
    data_dir: &Path,
    slippage_bps: f64,
    seed: u64,
) -> Result<HashMap<NodeId, Box<dyn VenueSimulator>>> {
    let mut simulators: HashMap<NodeId, Box<dyn VenueSimulator>> = HashMap::new();

    for node in &workflow.nodes {
        let id = node.id().to_string();
        let sim: Option<Box<dyn VenueSimulator>> = match node {
            Node::Wallet { .. } => {
                Some(Box::new(sim::wallet::WalletSimulator::new(0.0)))
            }
            Node::Perp { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::PerpCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::perp::PerpSimulator::new(
                        rows,
                        slippage_bps,
                        seed,
                    )))
                } else {
                    // No data — create with empty data (will be a no-op)
                    Some(Box::new(sim::perp::PerpSimulator::new(
                        vec![default_perp_row()],
                        slippage_bps,
                        seed,
                    )))
                }
            }
            Node::Options { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::OptionsCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::options::OptionsSimulator::new(rows)))
                } else {
                    Some(Box::new(sim::options::OptionsSimulator::new(vec![])))
                }
            }
            Node::Spot { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::PriceCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::spot::SpotSimulator::new(rows, slippage_bps)))
                } else {
                    Some(Box::new(sim::spot::SpotSimulator::new(
                        vec![default_price_row()],
                        slippage_bps,
                    )))
                }
            }
            Node::Lp { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::LpCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::lp::LpSimulator::new(rows)))
                } else {
                    Some(Box::new(sim::lp::LpSimulator::new(vec![default_lp_row()])))
                }
            }
            Node::Swap { .. } => {
                Some(Box::new(sim::swap::SwapSimulator::new(slippage_bps, 30.0)))
            }
            Node::Bridge { .. } => {
                Some(Box::new(sim::bridge::BridgeSimulator::new(10.0)))
            }
            Node::Lending { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::LendingCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::lending::LendingSimulator::new(rows)))
                } else {
                    Some(Box::new(sim::lending::LendingSimulator::new(vec![
                        default_lending_row(),
                    ])))
                }
            }
            Node::Pendle { .. } => {
                if let Some(entry) = manifest.get(&id) {
                    let rows: Vec<csv_types::PendleCsvRow> =
                        data::load_csv(data_dir, &entry.file)?;
                    Some(Box::new(sim::pendle::PendleSimulator::new(rows)))
                } else {
                    Some(Box::new(sim::pendle::PendleSimulator::new(vec![
                        default_pendle_row(),
                    ])))
                }
            }
            Node::Optimizer { .. } => {
                // Optimizer is handled by the engine directly, not a simulator
                None
            }
        };

        if let Some(s) = sim {
            simulators.insert(id, s);
        }
    }

    Ok(simulators)
}

/// Seed the first wallet node with initial capital.
fn seed_wallet(engine: &mut Engine, capital: f64) {
    for node in &engine.workflow.nodes {
        if let Node::Wallet { id, .. } = node {
            engine.balances.add(id, "USDC", capital);
            // Also seed the wallet simulator
            if let Some(sim) = engine.simulators.get_mut(id.as_str()) {
                let _ = sim.execute(
                    node,
                    capital,
                    &engine.clock,
                );
            }
            break; // Only seed the first wallet
        }
    }
}

/// Collect metrics from all simulators.
fn collect_venue_metrics(engine: &Engine) -> (f64, f64, f64, f64, f64, u32) {
    let mut funding_pnl = 0.0;
    let mut premium_pnl = 0.0;
    let mut lp_fees = 0.0;
    let mut lending_interest = 0.0;
    let mut swap_costs = 0.0;
    let mut liquidations = 0u32;

    for sim in engine.simulators.values() {
        let m = sim.metrics();
        funding_pnl += m.funding_pnl;
        premium_pnl += m.premium_pnl;
        lp_fees += m.lp_fees;
        lending_interest += m.lending_interest;
        swap_costs += m.swap_costs;
        liquidations += m.liquidations;
    }

    (funding_pnl, premium_pnl, lp_fees, lending_interest, swap_costs, liquidations)
}

fn estimate_periods_per_year(clock: &SimClock) -> f64 {
    if clock.total_ticks() < 2 {
        return 365.0; // default daily
    }
    // Use total timespan to estimate
    365.0 * 3.0 // default: 8-hour periods
}

// ── Default rows for nodes without CSV data ──────────────────────────

fn default_perp_row() -> csv_types::PerpCsvRow {
    csv_types::PerpCsvRow {
        symbol: "DEFAULT".to_string(),
        mark_price: 1.0,
        index_price: 1.0,
        funding_rate: 0.0,
        open_interest: 0.0,
        volume_24h: 0.0,
        bid: 1.0,
        ask: 1.0,
        mid_price: 1.0,
        last_price: 1.0,
        premium: 0.0,
        basis: 0.0,
        timestamp: 0,
        funding_apy: 0.0,
        rewards_apy: 0.0,
    }
}

fn default_price_row() -> csv_types::PriceCsvRow {
    csv_types::PriceCsvRow {
        timestamp: 0,
        price: 1.0,
        bid: 1.0,
        ask: 1.0,
    }
}

fn default_lp_row() -> csv_types::LpCsvRow {
    csv_types::LpCsvRow {
        timestamp: 0,
        current_tick: 0,
        price_a: 1.0,
        price_b: 1.0,
        fee_apy: 0.0,
        reward_rate: 0.0,
        reward_token_price: 0.0,
    }
}

fn default_lending_row() -> csv_types::LendingCsvRow {
    csv_types::LendingCsvRow {
        timestamp: 0,
        supply_apy: 0.0,
        borrow_apy: 0.0,
        utilization: 0.0,
        reward_apy: 0.0,
    }
}

fn default_pendle_row() -> csv_types::PendleCsvRow {
    csv_types::PendleCsvRow {
        timestamp: 0,
        pt_price: 0.95,
        yt_price: 0.05,
        implied_apy: 0.05,
        underlying_price: 1.0,
        maturity: 0,
    }
}
