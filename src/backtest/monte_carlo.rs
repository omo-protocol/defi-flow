use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use rand::prelude::*;

use super::result::BacktestResult;

/// Configuration for Monte Carlo simulation.
pub struct MonteCarloConfig {
    pub n_simulations: u32,
    pub block_size: usize,
    pub gbm_vol_scale: f64,
}

/// Monte Carlo results: historical baseline + all simulation results.
pub struct MonteCarloResult {
    pub historical: BacktestResult,
    pub simulations: Vec<BacktestResult>,
}

// ── Public API ────────────────────────────────────────────────────────

/// Run Monte Carlo simulations alongside the historical backtest.
pub fn run(
    config: &super::BacktestConfig,
    mc_config: &MonteCarloConfig,
    historical: BacktestResult,
) -> Result<MonteCarloResult> {
    let manifest = crate::data::load_manifest(&config.data_dir)?;

    // Collect unique CSV files to resample (avoid resampling the same file twice)
    let unique_files: HashSet<(String, String)> = manifest
        .values()
        .map(|e| (e.file.clone(), e.kind.clone()))
        .collect();

    let mut sim_results = Vec::with_capacity(mc_config.n_simulations as usize);
    let pb = indicatif::ProgressBar::new(mc_config.n_simulations as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("  Monte Carlo [{bar:40}] {pos}/{len} ({eta})")
            .unwrap(),
    );

    for i in 0..mc_config.n_simulations {
        let sim_seed = config.seed.wrapping_add(i as u64 + 1);

        // Create temp directory for this simulation's resampled data
        let temp_dir = std::env::temp_dir().join(format!("defi-flow-mc-{}-{}", config.seed, i));
        std::fs::create_dir_all(&temp_dir)
            .with_context(|| format!("creating temp dir {}", temp_dir.display()))?;

        // Resample each unique CSV file
        let mut rng = StdRng::seed_from_u64(sim_seed);
        for (file, kind) in &unique_files {
            resample_csv(
                &config.data_dir.join(file),
                &temp_dir.join(file),
                kind,
                mc_config.block_size,
                mc_config.gbm_vol_scale,
                &mut rng,
            )
            .with_context(|| format!("resampling {}", file))?;
        }

        // Copy manifest.json to temp directory
        std::fs::copy(
            config.data_dir.join("manifest.json"),
            temp_dir.join("manifest.json"),
        )
        .context("copying manifest.json")?;

        // Run backtest on resampled data
        let sim_config = super::BacktestConfig {
            workflow_path: config.workflow_path.clone(),
            data_dir: temp_dir.clone(),
            capital: config.capital,
            slippage_bps: config.slippage_bps,
            seed: sim_seed,
            verbose: false,
            output: None,
            monte_carlo: None,
        };

        match super::run_single_backtest(&sim_config) {
            Ok(result) => sim_results.push(result),
            Err(e) => {
                eprintln!("  Warning: simulation {} failed: {}", i + 1, e);
            }
        }

        // Clean up temp directory
        let _ = std::fs::remove_dir_all(&temp_dir);
        pb.inc(1);
    }
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

    // Extract and sort metrics
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

// ── CSV Resampling ────────────────────────────────────────────────────

/// Resample a single CSV file using block bootstrap + GBM perturbation.
fn resample_csv(
    input_path: &Path,
    output_path: &Path,
    kind: &str,
    block_size: usize,
    gbm_vol_scale: f64,
    rng: &mut impl Rng,
) -> Result<()> {
    let mut reader = csv::Reader::from_path(input_path)
        .with_context(|| format!("opening {}", input_path.display()))?;
    let headers = reader.headers()?.clone();
    let records: Vec<csv::StringRecord> = reader
        .records()
        .collect::<Result<_, _>>()
        .with_context(|| format!("reading {}", input_path.display()))?;

    if records.len() < 2 {
        // Too few rows to resample — just copy
        std::fs::copy(input_path, output_path)?;
        return Ok(());
    }

    // Group into periods (for options: by snapshot; for others: one row per period)
    let periods = group_into_periods(&records, &headers, kind);
    let n_periods = periods.len();

    if n_periods < 2 {
        std::fs::copy(input_path, output_path)?;
        return Ok(());
    }

    // Identify price columns for GBM perturbation
    let price_cols = price_column_indices(&headers, kind);

    // Compute historical volatility for each price column (from original data)
    let sigmas: Vec<f64> = price_cols
        .iter()
        .map(|&col| compute_volatility(&records, &periods, col))
        .collect();

    // Block bootstrap: select periods with replacement
    let bootstrapped_indices = block_bootstrap(n_periods, block_size, rng);

    // Generate GBM scaling factors (one path per price column)
    let gbm_factors: Vec<Vec<f64>> = if gbm_vol_scale > 0.0 && !price_cols.is_empty() {
        sigmas
            .iter()
            .map(|sigma| generate_gbm_factors(n_periods, *sigma * gbm_vol_scale, rng))
            .collect()
    } else {
        price_cols.iter().map(|_| vec![1.0; n_periods]).collect()
    };

    // Collect original timestamps per period (first row of each period)
    let timestamp_col = headers.iter().position(|h| h == "timestamp");
    let snapshot_col = headers.iter().position(|h| h == "snapshot");

    let original_timestamps: Vec<String> = periods
        .iter()
        .map(|period_rows| {
            timestamp_col
                .map(|c| records[period_rows[0]][c].to_string())
                .unwrap_or_default()
        })
        .collect();

    // Build resampled records
    let mut output_records: Vec<csv::StringRecord> = Vec::new();

    for (new_period_idx, &orig_period_idx) in bootstrapped_indices.iter().enumerate() {
        let period_rows = &periods[orig_period_idx];

        for &orig_row_idx in period_rows {
            let mut fields: Vec<String> = (0..records[orig_row_idx].len())
                .map(|i| records[orig_row_idx][i].to_string())
                .collect();

            // Restore the original timestamp for this position in the sequence
            if let Some(ts_col) = timestamp_col {
                if new_period_idx < original_timestamps.len() {
                    fields[ts_col] = original_timestamps[new_period_idx].clone();
                }
            }

            // Update snapshot number for options
            if let Some(snap_col) = snapshot_col {
                fields[snap_col] = (new_period_idx + 1).to_string();
            }

            // Apply GBM perturbation to price columns
            for (price_idx, &col) in price_cols.iter().enumerate() {
                if let Ok(price) = fields[col].parse::<f64>() {
                    if price > 0.0 {
                        let factor = gbm_factors[price_idx][new_period_idx];
                        fields[col] = format!("{}", price * factor);
                    }
                }
            }

            output_records.push(csv::StringRecord::from(fields));
        }
    }

    // Write output CSV
    let mut writer = csv::Writer::from_path(output_path)
        .with_context(|| format!("writing {}", output_path.display()))?;
    writer.write_record(&headers)?;
    for record in &output_records {
        writer.write_record(record)?;
    }
    writer.flush()?;

    Ok(())
}

/// Group records into periods.
/// For options: group by snapshot number (multiple rows per snapshot).
/// For all other CSV types: each row is its own period.
fn group_into_periods(
    records: &[csv::StringRecord],
    headers: &csv::StringRecord,
    kind: &str,
) -> Vec<Vec<usize>> {
    if kind == "options" {
        if let Some(snap_col) = headers.iter().position(|h| h == "snapshot") {
            let mut groups: Vec<Vec<usize>> = Vec::new();
            let mut current_snapshot = String::new();

            for (i, record) in records.iter().enumerate() {
                let snap = record[snap_col].to_string();
                if snap != current_snapshot {
                    groups.push(Vec::new());
                    current_snapshot = snap;
                }
                groups.last_mut().unwrap().push(i);
            }
            return groups;
        }
    }

    // Default: each row is its own period
    (0..records.len()).map(|i| vec![i]).collect()
}

/// Get the column indices for price fields that should receive GBM perturbation.
fn price_column_indices(headers: &csv::StringRecord, kind: &str) -> Vec<usize> {
    let price_names: &[&str] = match kind {
        "perp" => &[
            "mark_price",
            "index_price",
            "bid",
            "ask",
            "mid_price",
            "last_price",
        ],
        "options" => &["spot_price"],
        "lp" => &["price_a", "price_b", "reward_token_price"],
        "pendle" => &["pt_price", "yt_price", "underlying_price"],
        _ => &[], // lending has no price columns
    };

    price_names
        .iter()
        .filter_map(|name| headers.iter().position(|h| h == *name))
        .collect()
}

/// Compute per-period log-return volatility from a price column.
fn compute_volatility(
    records: &[csv::StringRecord],
    periods: &[Vec<usize>],
    col: usize,
) -> f64 {
    // Extract one price per period (first row of each period)
    let prices: Vec<f64> = periods
        .iter()
        .filter_map(|period| {
            period
                .first()
                .and_then(|&idx| records[idx][col].parse::<f64>().ok())
        })
        .filter(|p| *p > 0.0)
        .collect();

    if prices.len() < 3 {
        return 0.01; // minimal volatility
    }

    // Compute log returns
    let log_returns: Vec<f64> = prices.windows(2).map(|w| (w[1] / w[0]).ln()).collect();

    let mean = log_returns.iter().sum::<f64>() / log_returns.len() as f64;
    let variance = log_returns
        .iter()
        .map(|r| (r - mean).powi(2))
        .sum::<f64>()
        / (log_returns.len() - 1) as f64;

    variance.sqrt()
}

/// Block bootstrap: resample period indices using blocks of consecutive periods.
/// Blocks wrap around to preserve the full dataset.
fn block_bootstrap(n_periods: usize, block_size: usize, rng: &mut impl Rng) -> Vec<usize> {
    let bs = block_size.max(1).min(n_periods);
    let n_blocks = (n_periods + bs - 1) / bs;
    let mut indices = Vec::with_capacity(n_blocks * bs);

    for _ in 0..n_blocks {
        let start = rng.random_range(0..n_periods);
        for j in 0..bs {
            indices.push((start + j) % n_periods);
        }
    }

    indices.truncate(n_periods);
    indices
}

/// Generate cumulative GBM scaling factors.
/// S_t = exp(sum_{j=1..t} (-0.5*sigma^2 + sigma*Z_j)) where Z_j ~ N(0,1)
fn generate_gbm_factors(n: usize, sigma: f64, rng: &mut impl Rng) -> Vec<f64> {
    let mut factors = Vec::with_capacity(n);
    let mut cumulative = 0.0_f64;

    for _ in 0..n {
        let z = standard_normal(rng);
        cumulative += -0.5 * sigma * sigma + sigma * z;
        factors.push(cumulative.exp());
    }

    factors
}

/// Box-Muller transform to generate N(0,1) samples.
fn standard_normal(rng: &mut impl Rng) -> f64 {
    let u1: f64 = rng.random_range(0.0001f64..1.0);
    let u2: f64 = rng.random_range(0.0f64..std::f64::consts::TAU);
    (-2.0 * u1.ln()).sqrt() * u2.cos()
}

// ── Percentile Utility ────────────────────────────────────────────────

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
