use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::data::csv_types::{LendingCsvRow, PendleCsvRow};

use super::FetchConfig;

const POOLS_URL: &str = "https://yields.llama.fi/pools";
const CHART_URL: &str = "https://yields.llama.fi/chart";

// ── API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PoolsResponse {
    data: Vec<Pool>,
}

#[derive(Debug, Deserialize)]
struct Pool {
    pool: String,
    chain: Option<String>,
    project: String,
    symbol: String,
    #[serde(rename = "tvlUsd")]
    tvl_usd: Option<f64>,
    apy: Option<f64>,
    #[serde(rename = "apyBase")]
    apy_base: Option<f64>,
    #[serde(rename = "apyReward")]
    apy_reward: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    data: Vec<ChartPoint>,
}

#[derive(Debug, Deserialize)]
struct ChartPoint {
    timestamp: String, // ISO date string like "2024-01-01T00:00:00.000Z"
    apy: Option<f64>,
    #[serde(rename = "apyBase")]
    apy_base: Option<f64>,
    #[serde(rename = "apyReward")]
    apy_reward: Option<f64>,
    #[serde(rename = "apyBaseBorrow")]
    apy_base_borrow: Option<f64>,
    #[serde(rename = "tvlUsd")]
    tvl_usd: Option<f64>,
}

// ── Public API ───────────────────────────────────────────────────────

/// Fetch lending rate history from DefiLlama yields API.
///
/// `venue_slug` is the DefiLlama project name (e.g. "hyperlend", "aave-v3").
/// `asset` is the token symbol (e.g. "USDC").
pub async fn fetch_lending(
    client: &reqwest::Client,
    venue_slug: &str,
    asset: &str,
    config: &FetchConfig,
) -> Result<Vec<LendingCsvRow>> {
    let pool_id = find_pool(client, venue_slug, asset, None).await
        .with_context(|| format!("finding DefiLlama pool for {venue_slug}/{asset}"))?;

    let chart = fetch_chart(client, &pool_id).await
        .with_context(|| format!("fetching chart for pool {pool_id}"))?;

    let start_s = config.start_time_ms / 1000;
    let end_s = config.end_time_ms / 1000;

    let rows: Vec<LendingCsvRow> = chart
        .iter()
        .filter_map(|p| {
            let ts = parse_timestamp(&p.timestamp)?;
            if ts < start_s || ts > end_s {
                return None;
            }
            Some(LendingCsvRow {
                timestamp: ts,
                supply_apy: p.apy_base.unwrap_or(0.0) / 100.0,
                borrow_apy: p.apy_base_borrow.unwrap_or_else(|| {
                    // Estimate borrow rate as ~1.3x supply rate if not available
                    p.apy_base.unwrap_or(0.0) * 1.3
                }) / 100.0,
                utilization: 0.0, // Not directly in DefiLlama response
                reward_apy: p.apy_reward.unwrap_or(0.0) / 100.0,
            })
        })
        .collect();

    Ok(rows)
}

/// Fetch Pendle market data from DefiLlama yields API.
///
/// `market` is the Pendle market identifier (e.g. "PT-kHYPE").
pub async fn fetch_pendle(
    client: &reqwest::Client,
    market: &str,
    config: &FetchConfig,
) -> Result<Vec<PendleCsvRow>> {
    // Search for the Pendle pool on DefiLlama
    let pool_id = find_pool(client, "pendle", market, None).await
        .with_context(|| format!("finding DefiLlama pool for pendle/{market}"))?;

    let chart = fetch_chart(client, &pool_id).await
        .with_context(|| format!("fetching chart for Pendle pool {pool_id}"))?;

    let start_s = config.start_time_ms / 1000;
    let end_s = config.end_time_ms / 1000;

    // Estimate maturity from market name (default: 6 months from now)
    let maturity = config.end_time_ms / 1000 + 180 * 86400;

    let rows: Vec<PendleCsvRow> = chart
        .iter()
        .filter_map(|p| {
            let ts = parse_timestamp(&p.timestamp)?;
            if ts < start_s || ts > end_s {
                return None;
            }

            let implied_apy = p.apy.unwrap_or(0.0) / 100.0;
            let time_to_maturity_years = (maturity - ts) as f64 / (365.25 * 86400.0);

            // PT price ≈ 1 / (1 + implied_apy * time_to_maturity)
            let pt_price = if time_to_maturity_years > 0.0 {
                1.0 / (1.0 + implied_apy * time_to_maturity_years)
            } else {
                1.0
            };
            let yt_price = (1.0 - pt_price).max(0.0);

            Some(PendleCsvRow {
                timestamp: ts,
                pt_price,
                yt_price,
                implied_apy,
                underlying_price: 1.0, // Normalized; real price needs separate lookup
                maturity,
            })
        })
        .collect();

    Ok(rows)
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Find a DefiLlama pool ID by project slug and asset symbol.
async fn find_pool(
    client: &reqwest::Client,
    project: &str,
    asset: &str,
    chain: Option<&str>,
) -> Result<String> {
    let resp = super::retry(3, || {
        let client = client.clone();
        async move {
            let r = client
                .get(POOLS_URL)
                .send()
                .await?
                .error_for_status()?
                .json::<PoolsResponse>()
                .await?;
            Ok(r)
        }
    })
    .await
    .context("fetching DefiLlama pools")?;

    let asset_upper = asset.to_uppercase();
    let project_lower = project.to_lowercase();

    // Strip common prefixes for fuzzy matching (PT-kHYPE → KHYPE, YT-kHYPE → KHYPE)
    let stripped_asset = asset_upper
        .strip_prefix("PT-")
        .or_else(|| asset_upper.strip_prefix("YT-"))
        .unwrap_or(&asset_upper)
        .to_uppercase();

    // Find best matching pool
    let mut candidates: Vec<&Pool> = resp
        .data
        .iter()
        .filter(|p| {
            let proj = p.project.to_lowercase();
            let proj_match = proj == project_lower || proj.starts_with(&project_lower);
            let sym = p.symbol.to_uppercase();
            let symbol_match =
                sym.contains(&asset_upper) || sym.contains(&stripped_asset);
            let chain_match = chain
                .map(|c| {
                    p.chain
                        .as_ref()
                        .is_some_and(|pc| pc.to_lowercase() == c.to_lowercase())
                })
                .unwrap_or(true);
            proj_match && symbol_match && chain_match
        })
        .collect();

    // Sort by TVL descending to pick the biggest pool
    candidates.sort_by(|a, b| {
        b.tvl_usd
            .unwrap_or(0.0)
            .partial_cmp(&a.tvl_usd.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some(pool) = candidates.first() {
        Ok(pool.pool.clone())
    } else {
        bail!(
            "No DefiLlama pool found for project={project}, asset={asset}. \
             Try checking available pools at https://yields.llama.fi/pools"
        )
    }
}

/// Fetch the historical chart for a given pool ID.
async fn fetch_chart(client: &reqwest::Client, pool_id: &str) -> Result<Vec<ChartPoint>> {
    let url = format!("{CHART_URL}/{pool_id}");

    let resp = super::retry(3, || {
        let client = client.clone();
        let url = url.clone();
        async move {
            let r = client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json::<ChartResponse>()
                .await?;
            Ok(r)
        }
    })
    .await
    .context("fetching DefiLlama chart")?;

    Ok(resp.data)
}

/// Parse a DefiLlama timestamp string to unix seconds.
fn parse_timestamp(ts: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .or_else(|| {
            // Try ISO format without timezone
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
                .ok()
                .map(|ndt| ndt.and_utc().fixed_offset())
        })
        .map(|dt| dt.timestamp() as u64)
}
