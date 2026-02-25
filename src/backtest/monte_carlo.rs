use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use rand::prelude::*;
use rayon::prelude::*;
use serde::Serialize;

use super::result::BacktestResult;

/// Configuration for Monte Carlo simulation.
pub struct MonteCarloConfig {
    pub n_simulations: u32,
}

/// Monte Carlo results: historical baseline + all simulation results.
#[derive(Serialize)]
pub struct MonteCarloResult {
    pub historical: BacktestResult,
    pub simulations: Vec<BacktestResult>,
}

// ── Estimated parameters from historical data ────────────────────────

/// Parameters estimated from a perp CSV for synthetic generation.
struct PerpParams {
    n_periods: usize,
    start_price: f64,
    /// Per-period log-return drift
    price_drift: f64,
    /// Per-period log-return volatility
    price_vol: f64,
    /// Mean funding rate (per period)
    funding_mean: f64,
    /// Funding rate OU mean-reversion speed (per period, 0..1)
    funding_theta: f64,
    /// Funding rate OU volatility
    funding_sigma: f64,
    /// Mean rewards APY
    rewards_mean: f64,
    /// Mean bid-ask spread as fraction of price
    spread_frac: f64,
    /// Original symbol
    symbol: String,
    /// Original timestamps
    timestamps: Vec<u64>,
}

/// Parameters estimated from a price/spot CSV.
struct PriceParams {
    n_periods: usize,
    start_price: f64,
    spread_frac: f64,
    timestamps: Vec<u64>,
}

/// Parameters estimated from a lending CSV.
struct LendingParams {
    n_periods: usize,
    supply_apy_mean: f64,
    supply_apy_std: f64,
    borrow_apy_mean: f64,
    reward_apy_mean: f64,
    reward_apy_std: f64,
    utilization_mean: f64,
    /// AR(1) coefficient for supply APY
    ar1_coeff: f64,
    timestamps: Vec<u64>,
}

/// Parameters estimated from a vault CSV.
struct VaultParams {
    n_periods: usize,
    apy_mean: f64,
    apy_std: f64,
    reward_apy_mean: f64,
    reward_apy_std: f64,
    ar1_coeff: f64,
    timestamps: Vec<u64>,
}

/// Parameters estimated from an LP CSV.
struct LpParams {
    n_periods: usize,
    /// Starting tick
    tick_start: i32,
    /// Tick OU mean-reversion speed
    tick_theta: f64,
    /// Tick OU volatility
    tick_sigma: f64,
    /// Starting price_a (token A, e.g. WETH)
    start_price_a: f64,
    /// Price_b (token B, e.g. USDC) — typically stable
    price_b: f64,
    /// Fee APY AR(1) parameters
    fee_apy_mean: f64,
    fee_apy_std: f64,
    fee_ar1: f64,
    /// Reward rate AR(1) parameters
    reward_rate_mean: f64,
    reward_rate_std: f64,
    reward_ar1: f64,
    /// Mean reward token price (e.g. AERO)
    reward_token_price: f64,
    timestamps: Vec<u64>,
}

// ── Public API ────────────────────────────────────────────────────────

/// Run Monte Carlo simulations with parametric synthetic data generation.
///
/// Instead of shuffling historical data, we estimate the statistical
/// parameters (mean, vol, mean-reversion) from the historical CSVs
/// and generate synthetic paths from those distributions:
/// - Prices: GBM (shared across spot+perp for correlation)
/// - Funding rate: Ornstein-Uhlenbeck (mean-reverting)
/// - APY: AR(1) around historical mean
pub fn run(
    config: &super::BacktestConfig,
    mc_config: &MonteCarloConfig,
    historical: BacktestResult,
) -> Result<MonteCarloResult> {
    let manifest = crate::data::load_manifest(&config.data_dir)?;

    // Collect unique CSV files
    let unique_files: HashSet<(String, String)> = manifest
        .values()
        .map(|e| (e.file.clone(), e.kind.clone()))
        .collect();

    // Estimate parameters from each CSV file
    let file_params: Vec<(String, String, CsvParams)> = unique_files
        .iter()
        .filter_map(|(file, kind)| {
            let path = config.data_dir.join(file);
            let params = estimate_params(&path, kind).ok()?;
            Some((file.clone(), kind.clone(), params))
        })
        .collect();

    // Find the shared price GBM params (from the perp file, if present)
    let perp_params: Option<&PerpParams> = file_params.iter().find_map(|(_, kind, params)| {
        if kind == "perp" {
            if let CsvParams::Perp(p) = params {
                return Some(p);
            }
        }
        None
    });

    // Extract shared GBM parameters for price correlation
    let shared_price_drift = perp_params.map(|p| p.price_drift).unwrap_or(0.0);
    let shared_price_vol = perp_params.map(|p| p.price_vol).unwrap_or(0.01);
    let shared_n_periods = perp_params
        .map(|p| p.n_periods)
        .unwrap_or_else(|| {
            file_params.iter().map(|(_, _, p)| p.n_periods()).max().unwrap_or(100)
        });

    // Build a timestamp→GBM_index map from the perp's timestamps.
    // This ensures spot and perp use the same GBM factor at the same calendar time.
    let shared_timestamps: Vec<u64> = perp_params
        .map(|p| p.timestamps.clone())
        .unwrap_or_default();
    let ts_to_gbm_idx: std::collections::HashMap<u64, usize> = shared_timestamps
        .iter()
        .enumerate()
        .map(|(i, &ts)| (ts, i))
        .collect();
    // Perp's start price — used to align spot price levels so delta-neutral
    // strategies buy/short the same number of tokens at the same price.
    let perp_start_price = perp_params.map(|p| p.start_price);

    let pb = indicatif::ProgressBar::new(mc_config.n_simulations as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("  Monte Carlo [{bar:40}] {pos}/{len} ({eta})")
            .unwrap(),
    );

    let sim_results: Vec<BacktestResult> = (0..mc_config.n_simulations)
        .into_par_iter()
        .filter_map(|i| {
            let sim_seed = config.seed.wrapping_add(i as u64 + 1);
            let mut rng = StdRng::seed_from_u64(sim_seed);

            // Create temp directory
            let temp_dir =
                std::env::temp_dir().join(format!("defi-flow-mc-{}-{}", config.seed, i));
            if std::fs::create_dir_all(&temp_dir).is_err() {
                pb.inc(1);
                return None;
            }

            // Generate ONE shared GBM price path for all correlated files
            let shared_gbm = generate_gbm_prices(
                shared_n_periods,
                shared_price_drift,
                shared_price_vol,
                &mut rng,
            );

            // Generate synthetic CSV for each file
            for (file, kind, params) in &file_params {
                let output_path = temp_dir.join(file);
                if generate_synthetic_csv(
                    &output_path,
                    kind,
                    params,
                    &shared_gbm,
                    &ts_to_gbm_idx,
                    perp_start_price,
                    &mut rng,
                )
                .is_err()
                {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    pb.inc(1);
                    return None;
                }
            }

            // Copy manifest
            if std::fs::copy(
                config.data_dir.join("manifest.json"),
                temp_dir.join("manifest.json"),
            )
            .is_err()
            {
                let _ = std::fs::remove_dir_all(&temp_dir);
                pb.inc(1);
                return None;
            }

            // Run backtest
            let sim_config = super::BacktestConfig {
                workflow_path: config.workflow_path.clone(),
                data_dir: temp_dir.clone(),
                capital: config.capital,
                slippage_bps: config.slippage_bps,
                seed: sim_seed,
                verbose: false,
                output: None,
                tick_csv: None,
                monte_carlo: None,
            };

            let result = super::run_single_backtest(&sim_config).ok();
            let _ = std::fs::remove_dir_all(&temp_dir);
            pb.inc(1);
            result
        })
        .collect();

    pb.finish_and_clear();

    Ok(MonteCarloResult {
        historical,
        simulations: sim_results,
    })
}

/// Print Monte Carlo results summary.
pub fn print_results(mc: &MonteCarloResult) {
    let h = &mc.historical;
    let sims = &mc.simulations;

    if sims.is_empty() {
        println!("  No successful simulations.");
        return;
    }

    let mut twrrs: Vec<f64> = sims.iter().map(|r| r.twrr_pct).collect();
    let mut drawdowns: Vec<f64> = sims.iter().map(|r| r.max_drawdown_pct).collect();
    let mut sharpes: Vec<f64> = sims.iter().map(|r| r.sharpe).collect();
    let mut pnls: Vec<f64> = sims.iter().map(|r| r.net_pnl).collect();

    twrrs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    drawdowns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sharpes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    pnls.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    println!("\n{}", "═".repeat(68));
    println!("  Monte Carlo Results ({} simulations)", sims.len());
    println!("{}", "═".repeat(68));
    println!(
        "  Historical:  TWRR={:+.2}%  MxDD={:.2}%  Sharpe={:.3}",
        h.twrr_pct, h.max_drawdown_pct, h.sharpe
    );
    println!();
    println!(
        "  {:>12}  {:>8}  {:>8}  {:>8}  {:>10}",
        "Percentiles", "TWRR%", "MxDD%", "Sharpe", "NetPnL"
    );
    println!("  {}", "─".repeat(52));

    let pct_levels = [5.0, 25.0, 50.0, 75.0, 95.0];
    let pct_labels = ["5th", "25th", "50th", "75th", "95th"];

    for (label, pct) in pct_labels.iter().zip(pct_levels.iter()) {
        println!(
            "  {:>12}  {:>+8.2}  {:>8.2}  {:>8.3}  {:>+10.0}",
            label,
            percentile(&twrrs, *pct),
            percentile(&drawdowns, *pct),
            percentile(&sharpes, *pct),
            percentile(&pnls, *pct),
        );
    }

    println!();
    let var95 = percentile(&pnls, 5.0);
    let var99 = percentile(&pnls, 1.0);
    println!(
        "  VaR(95%): ${:+.0}   VaR(99%): ${:+.0}",
        var95, var99,
    );
    println!("{}", "═".repeat(68));
}

// ── Parameter estimation ──────────────────────────────────────────────

enum CsvParams {
    Perp(PerpParams),
    Price(PriceParams),
    Lending(LendingParams),
    Vault(VaultParams),
    Lp(LpParams),
    /// Unknown CSV type — will be copied as-is.
    Passthrough(std::path::PathBuf),
}

impl CsvParams {
    fn n_periods(&self) -> usize {
        match self {
            CsvParams::Perp(p) => p.n_periods,
            CsvParams::Price(p) => p.n_periods,
            CsvParams::Lending(p) => p.n_periods,
            CsvParams::Vault(p) => p.n_periods,
            CsvParams::Lp(p) => p.n_periods,
            CsvParams::Passthrough(_) => 0,
        }
    }
}

fn estimate_params(path: &Path, kind: &str) -> Result<CsvParams> {
    match kind {
        "perp" => estimate_perp_params(path).map(CsvParams::Perp),
        "price" | "spot" => estimate_price_params(path).map(CsvParams::Price),
        "lending" => estimate_lending_params(path).map(CsvParams::Lending),
        "vault" => estimate_vault_params(path).map(CsvParams::Vault),
        "lp" => estimate_lp_params(path).map(CsvParams::Lp),
        _ => Ok(CsvParams::Passthrough(path.to_path_buf())),
    }
}

fn estimate_perp_params(path: &Path) -> Result<PerpParams> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let rows: Vec<crate::venues::perps::data::PerpCsvRow> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", path.display()))?;

    if rows.len() < 10 {
        anyhow::bail!("too few rows for perp params estimation");
    }

    let n = rows.len();
    let timestamps: Vec<u64> = rows.iter().map(|r| r.timestamp).collect();

    // Price GBM parameters from log returns
    let log_returns: Vec<f64> = rows
        .windows(2)
        .filter_map(|w| {
            if w[0].mark_price > 0.0 && w[1].mark_price > 0.0 {
                Some((w[1].mark_price / w[0].mark_price).ln())
            } else {
                None
            }
        })
        .collect();
    let price_drift = mean(&log_returns);
    let price_vol = std_dev(&log_returns);

    // Funding rate OU parameters
    let funding_rates: Vec<f64> = rows.iter().map(|r| r.funding_rate).collect();
    let funding_mean = mean(&funding_rates);
    let (funding_theta, funding_sigma) = estimate_ou(&funding_rates);

    // Rewards APY
    let rewards_mean = mean(&rows.iter().map(|r| r.rewards_apy).collect::<Vec<_>>());

    // Bid-ask spread
    let spreads: Vec<f64> = rows
        .iter()
        .filter(|r| r.mark_price > 0.0)
        .map(|r| (r.ask - r.bid) / r.mark_price)
        .collect();
    let spread_frac = mean(&spreads).max(0.0001);

    Ok(PerpParams {
        n_periods: n,
        start_price: rows[0].mark_price,
        price_drift,
        price_vol,
        funding_mean,
        funding_theta,
        funding_sigma,
        rewards_mean,
        spread_frac,
        symbol: rows[0].symbol.clone(),
        timestamps,
    })
}

fn estimate_price_params(path: &Path) -> Result<PriceParams> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let rows: Vec<crate::venues::perps::data::PriceCsvRow> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", path.display()))?;

    if rows.len() < 2 {
        anyhow::bail!("too few rows for price params");
    }

    let spreads: Vec<f64> = rows
        .iter()
        .filter(|r| r.price > 0.0)
        .map(|r| (r.ask - r.bid) / r.price)
        .collect();

    Ok(PriceParams {
        n_periods: rows.len(),
        start_price: rows[0].price,
        spread_frac: mean(&spreads).max(0.0001),
        timestamps: rows.iter().map(|r| r.timestamp).collect(),
    })
}

fn estimate_lending_params(path: &Path) -> Result<LendingParams> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let rows: Vec<crate::venues::lending::data::LendingCsvRow> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", path.display()))?;

    if rows.len() < 2 {
        anyhow::bail!("too few rows for lending params");
    }

    let supply_apys: Vec<f64> = rows.iter().map(|r| r.supply_apy).collect();
    let borrow_apys: Vec<f64> = rows.iter().map(|r| r.borrow_apy).collect();
    let reward_apys: Vec<f64> = rows.iter().map(|r| r.reward_apy).collect();
    let utils: Vec<f64> = rows.iter().map(|r| r.utilization).collect();

    Ok(LendingParams {
        n_periods: rows.len(),
        supply_apy_mean: mean(&supply_apys),
        supply_apy_std: std_dev(&supply_apys),
        borrow_apy_mean: mean(&borrow_apys),
        reward_apy_mean: mean(&reward_apys),
        reward_apy_std: std_dev(&reward_apys),
        utilization_mean: mean(&utils),
        ar1_coeff: estimate_ar1(&supply_apys),
        timestamps: rows.iter().map(|r| r.timestamp).collect(),
    })
}

fn estimate_vault_params(path: &Path) -> Result<VaultParams> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let rows: Vec<crate::venues::vault::data::VaultCsvRow> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", path.display()))?;

    if rows.len() < 2 {
        anyhow::bail!("too few rows for vault params");
    }

    let apys: Vec<f64> = rows.iter().map(|r| r.apy).collect();
    let reward_apys: Vec<f64> = rows.iter().map(|r| r.reward_apy).collect();

    Ok(VaultParams {
        n_periods: rows.len(),
        apy_mean: mean(&apys),
        apy_std: std_dev(&apys),
        reward_apy_mean: mean(&reward_apys),
        reward_apy_std: std_dev(&reward_apys),
        ar1_coeff: estimate_ar1(&apys),
        timestamps: rows.iter().map(|r| r.timestamp).collect(),
    })
}

// ── Synthetic data generation ─────────────────────────────────────────

/// Generate a GBM price path: S_t = S_0 * exp(sum(drift + vol*Z_i))
/// Returns price multipliers (relative to start) for each period.
fn generate_gbm_prices(
    n: usize,
    drift: f64,
    vol: f64,
    rng: &mut impl Rng,
) -> Vec<f64> {
    let mut prices = Vec::with_capacity(n);
    let mut log_cum = 0.0;

    for _ in 0..n {
        let z = standard_normal(rng);
        log_cum += drift - 0.5 * vol * vol + vol * z;
        prices.push(log_cum.exp());
    }
    prices
}

/// Generate an OU (mean-reverting) path for funding rates.
/// dx = theta * (mu - x) * dt + sigma * dW
fn generate_ou_path(
    n: usize,
    mu: f64,
    theta: f64,
    sigma: f64,
    rng: &mut impl Rng,
) -> Vec<f64> {
    let mut path = Vec::with_capacity(n);
    let mut x = mu;

    for _ in 0..n {
        let z = standard_normal(rng);
        x += theta * (mu - x) + sigma * z;
        path.push(x);
    }
    path
}

/// Generate an AR(1) path around a mean: x_t = mean + phi*(x_{t-1} - mean) + sigma*Z
fn generate_ar1_path(
    n: usize,
    mu: f64,
    sigma: f64,
    phi: f64,
    rng: &mut impl Rng,
) -> Vec<f64> {
    let mut path = Vec::with_capacity(n);
    let mut x = mu;

    for _ in 0..n {
        let z = standard_normal(rng);
        x = mu + phi * (x - mu) + sigma * z;
        path.push(x.max(0.0)); // APY can't be negative
    }
    path
}

/// Write a synthetic CSV for the given kind and parameters.
/// `shared_gbm` provides correlated price multipliers across files.
/// `ts_to_gbm_idx` maps timestamps to GBM array indices for cross-file alignment.
fn generate_synthetic_csv(
    output_path: &Path,
    _kind: &str,
    params: &CsvParams,
    shared_gbm: &[f64],
    ts_to_gbm_idx: &std::collections::HashMap<u64, usize>,
    perp_start_price: Option<f64>,
    rng: &mut impl Rng,
) -> Result<()> {
    match params {
        CsvParams::Perp(p) => generate_perp_csv(output_path, p, shared_gbm, rng),
        CsvParams::Price(p) => generate_price_csv(output_path, p, shared_gbm, ts_to_gbm_idx, perp_start_price),
        CsvParams::Lending(p) => generate_lending_csv(output_path, p, rng),
        CsvParams::Vault(p) => generate_vault_csv(output_path, p, rng),
        CsvParams::Lp(p) => generate_lp_csv(output_path, p, shared_gbm, rng),
        CsvParams::Passthrough(src) => {
            std::fs::copy(src, output_path)?;
            Ok(())
        }
    }
}

fn generate_perp_csv(
    output_path: &Path,
    params: &PerpParams,
    shared_gbm: &[f64],
    rng: &mut impl Rng,
) -> Result<()> {
    let n = params.n_periods;

    // Generate funding rate path (OU process)
    let funding_path = generate_ou_path(
        n,
        params.funding_mean,
        params.funding_theta,
        params.funding_sigma,
        rng,
    );

    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;

    writer.write_record([
        "symbol",
        "mark_price",
        "index_price",
        "funding_rate",
        "open_interest",
        "volume_24h",
        "bid",
        "ask",
        "mid_price",
        "last_price",
        "premium",
        "basis",
        "timestamp",
        "funding_apy",
        "rewards_apy",
    ])?;

    for i in 0..n {
        let gbm_factor = if i < shared_gbm.len() { shared_gbm[i] } else { 1.0 };
        let price = params.start_price * gbm_factor;
        let half_spread = price * params.spread_frac * 0.5;
        let bid = price - half_spread;
        let ask = price + half_spread;
        let fr = funding_path[i];
        let ts = if i < params.timestamps.len() {
            params.timestamps[i]
        } else {
            params.timestamps.last().copied().unwrap_or(0) + (i as u64 * 28800)
        };

        writer.write_record(&[
            params.symbol.clone(),
            format!("{price}"),
            format!("{price}"),
            format!("{fr}"),
            "0".to_string(),
            "0".to_string(),
            format!("{bid}"),
            format!("{ask}"),
            format!("{price}"),
            format!("{price}"),
            "0".to_string(),
            "0".to_string(),
            format!("{ts}"),
            format!("{}", fr * 8760.0),
            format!("{}", params.rewards_mean),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn generate_price_csv(
    output_path: &Path,
    params: &PriceParams,
    shared_gbm: &[f64],
    ts_to_gbm_idx: &std::collections::HashMap<u64, usize>,
    perp_start_price: Option<f64>,
) -> Result<()> {
    let n = params.n_periods;
    // Use perp's start_price to ensure spot and perp have the same absolute price
    // level — critical for delta-neutral strategies to buy/short equal token amounts.
    let base_price = perp_start_price.unwrap_or(params.start_price);

    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;

    writer.write_record(["timestamp", "price", "bid", "ask"])?;

    for i in 0..n {
        let ts = if i < params.timestamps.len() {
            params.timestamps[i]
        } else {
            params.timestamps.last().copied().unwrap_or(0) + (i as u64 * 28800)
        };

        // Use timestamp-aligned GBM index so spot tracks the same price as perp
        let gbm_idx = ts_to_gbm_idx.get(&ts).copied().unwrap_or(i);
        let gbm_factor = if gbm_idx < shared_gbm.len() { shared_gbm[gbm_idx] } else { 1.0 };
        let price = base_price * gbm_factor;
        let half_spread = price * params.spread_frac * 0.5;

        writer.write_record(&[
            format!("{ts}"),
            format!("{price}"),
            format!("{}", price - half_spread),
            format!("{}", price + half_spread),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn generate_lending_csv(
    output_path: &Path,
    params: &LendingParams,
    rng: &mut impl Rng,
) -> Result<()> {
    let supply_path = generate_ar1_path(
        params.n_periods,
        params.supply_apy_mean,
        params.supply_apy_std,
        params.ar1_coeff,
        rng,
    );
    let reward_path = generate_ar1_path(
        params.n_periods,
        params.reward_apy_mean,
        params.reward_apy_std,
        params.ar1_coeff,
        rng,
    );

    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;

    writer.write_record(["timestamp", "supply_apy", "borrow_apy", "utilization", "reward_apy"])?;

    for i in 0..params.n_periods {
        let ts = if i < params.timestamps.len() {
            params.timestamps[i]
        } else {
            params.timestamps.last().copied().unwrap_or(0) + (i as u64 * 28800)
        };
        // borrow_apy scales with supply_apy using historical ratio
        let borrow_ratio = if params.supply_apy_mean > 0.0 {
            params.borrow_apy_mean / params.supply_apy_mean
        } else {
            1.5
        };

        writer.write_record(&[
            format!("{ts}"),
            format!("{}", supply_path[i]),
            format!("{}", (supply_path[i] * borrow_ratio).max(0.0)),
            format!("{}", params.utilization_mean),
            format!("{}", reward_path[i]),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn generate_vault_csv(
    output_path: &Path,
    params: &VaultParams,
    rng: &mut impl Rng,
) -> Result<()> {
    let apy_path = generate_ar1_path(
        params.n_periods,
        params.apy_mean,
        params.apy_std,
        params.ar1_coeff,
        rng,
    );
    let reward_path = generate_ar1_path(
        params.n_periods,
        params.reward_apy_mean,
        params.reward_apy_std,
        params.ar1_coeff,
        rng,
    );

    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;

    writer.write_record(["timestamp", "apy", "reward_apy"])?;

    for i in 0..params.n_periods {
        let ts = if i < params.timestamps.len() {
            params.timestamps[i]
        } else {
            params.timestamps.last().copied().unwrap_or(0) + (i as u64 * 28800)
        };

        writer.write_record(&[
            format!("{ts}"),
            format!("{}", apy_path[i]),
            format!("{}", reward_path[i]),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn estimate_lp_params(path: &Path) -> Result<LpParams> {
    let mut reader = csv::Reader::from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let rows: Vec<crate::venues::lp::data::LpCsvRow> = reader
        .deserialize()
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", path.display()))?;

    if rows.len() < 10 {
        anyhow::bail!("too few rows for LP params estimation");
    }

    let timestamps: Vec<u64> = rows.iter().map(|r| r.timestamp).collect();

    // Tick OU parameters
    let ticks: Vec<f64> = rows.iter().map(|r| r.current_tick as f64).collect();
    let (tick_theta, tick_sigma) = estimate_ou(&ticks);

    // Fee APY AR(1) parameters
    let fee_apys: Vec<f64> = rows.iter().map(|r| r.fee_apy).collect();
    let fee_ar1 = estimate_ar1(&fee_apys);

    // Reward rate AR(1) parameters
    let reward_rates: Vec<f64> = rows.iter().map(|r| r.reward_rate).collect();
    let reward_ar1 = estimate_ar1(&reward_rates);

    Ok(LpParams {
        n_periods: rows.len(),
        tick_start: rows[0].current_tick,
        tick_theta,
        tick_sigma,
        start_price_a: rows[0].price_a,
        price_b: rows[0].price_b,
        fee_apy_mean: mean(&fee_apys),
        fee_apy_std: std_dev(&fee_apys),
        fee_ar1,
        reward_rate_mean: mean(&reward_rates),
        reward_rate_std: std_dev(&reward_rates),
        reward_ar1,
        reward_token_price: mean(&rows.iter().map(|r| r.reward_token_price).collect::<Vec<_>>()),
        timestamps,
    })
}

fn generate_lp_csv(
    output_path: &Path,
    params: &LpParams,
    shared_gbm: &[f64],
    rng: &mut impl Rng,
) -> Result<()> {
    let n = params.n_periods;

    // Tick follows OU process (mean-reverting around starting tick)
    let tick_mean = params.tick_start as f64;
    let tick_path = generate_ou_path(n, tick_mean, params.tick_theta, params.tick_sigma, rng);

    // Fee APY follows AR(1)
    let fee_path = generate_ar1_path(
        n,
        params.fee_apy_mean,
        params.fee_apy_std,
        params.fee_ar1,
        rng,
    );

    // Reward rate follows AR(1)
    let reward_path = generate_ar1_path(
        n,
        params.reward_rate_mean,
        params.reward_rate_std,
        params.reward_ar1,
        rng,
    );

    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;

    writer.write_record([
        "timestamp",
        "current_tick",
        "price_a",
        "price_b",
        "fee_apy",
        "reward_rate",
        "reward_token_price",
    ])?;

    for i in 0..n {
        // price_a (e.g. WETH) follows shared GBM for correlation with spot/perp
        let gbm_factor = if i < shared_gbm.len() { shared_gbm[i] } else { 1.0 };
        let price_a = params.start_price_a * gbm_factor;

        let ts = if i < params.timestamps.len() {
            params.timestamps[i]
        } else {
            params.timestamps.last().copied().unwrap_or(0) + (i as u64 * 28800)
        };

        writer.write_record(&[
            format!("{ts}"),
            format!("{}", tick_path[i] as i32),
            format!("{price_a}"),
            format!("{}", params.price_b),
            format!("{}", fee_path[i]),
            format!("{}", reward_path[i]),
            format!("{}", params.reward_token_price),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

// ── Statistical helpers ──────────────────────────────────────────────

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() { return 0.0; }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn std_dev(xs: &[f64]) -> f64 {
    if xs.len() < 2 { return 0.0; }
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
    var.sqrt()
}

/// Estimate AR(1) coefficient: phi = corr(x_t, x_{t-1})
fn estimate_ar1(xs: &[f64]) -> f64 {
    if xs.len() < 3 { return 0.5; }
    let m = mean(xs);
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 1..xs.len() {
        num += (xs[i] - m) * (xs[i - 1] - m);
        den += (xs[i - 1] - m).powi(2);
    }
    if den < 1e-12 { return 0.5; }
    (num / den).clamp(0.0, 0.99)
}

/// Estimate OU parameters: theta (mean-reversion speed) and sigma (noise vol).
/// From discrete observations: x_{t+1} = x_t + theta*(mu - x_t) + sigma*Z
fn estimate_ou(xs: &[f64]) -> (f64, f64) {
    if xs.len() < 3 {
        return (0.1, std_dev(xs));
    }

    let mu = mean(xs);
    // Regress dx on (mu - x) to get theta
    let mut sum_xy = 0.0;
    let mut sum_x2 = 0.0;
    let mut residuals = Vec::new();

    for i in 1..xs.len() {
        let dx = xs[i] - xs[i - 1];
        let deviation = mu - xs[i - 1];
        sum_xy += dx * deviation;
        sum_x2 += deviation * deviation;
    }

    let theta = if sum_x2 > 1e-20 {
        (sum_xy / sum_x2).clamp(0.001, 1.0)
    } else {
        0.1
    };

    // Estimate sigma from residuals
    for i in 1..xs.len() {
        let dx = xs[i] - xs[i - 1];
        let predicted = theta * (mu - xs[i - 1]);
        residuals.push(dx - predicted);
    }

    let sigma = std_dev(&residuals).max(1e-10);

    (theta, sigma)
}

/// Box-Muller transform to generate N(0,1) samples.
fn standard_normal(rng: &mut impl Rng) -> f64 {
    let u1: f64 = rng.random_range(0.0001f64..1.0);
    let u2: f64 = rng.random_range(0.0f64..std::f64::consts::TAU);
    (-2.0 * u1.ln()).sqrt() * u2.cos()
}

/// Linear interpolation percentile on a sorted slice.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    let frac = idx - lo as f64;

    if hi >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}
