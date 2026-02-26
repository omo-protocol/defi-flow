use anyhow::{Context, Result, bail};
use serde::Deserialize;

const POOLS_URL: &str = "https://yields.llama.fi/pools";
const CHART_URL: &str = "https://yields.llama.fi/chart";

// ── API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PoolsResponse {
    data: Vec<Pool>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct ChartPoint {
    pub timestamp: String, // ISO date string like "2024-01-01T00:00:00.000Z"
    pub apy: Option<f64>,
    #[serde(rename = "apyBase")]
    pub apy_base: Option<f64>,
    #[serde(rename = "apyReward")]
    pub apy_reward: Option<f64>,
    #[serde(rename = "apyBaseBorrow")]
    pub apy_base_borrow: Option<f64>,
    #[serde(rename = "tvlUsd")]
    pub tvl_usd: Option<f64>,
}

// ── Public API ───────────────────────────────────────────────────────

/// Find a DefiLlama pool ID by project slug and asset symbol.
pub async fn find_pool(
    client: &reqwest::Client,
    project: &str,
    asset: &str,
    chain: Option<&str>,
) -> Result<String> {
    let resp = super::super::types::retry(3, || {
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
            let symbol_match = sym.contains(&asset_upper) || sym.contains(&stripped_asset);
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
pub async fn fetch_chart(client: &reqwest::Client, pool_id: &str) -> Result<Vec<ChartPoint>> {
    let url = format!("{CHART_URL}/{pool_id}");

    let resp = super::super::types::retry(3, || {
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
pub fn parse_timestamp(ts: &str) -> Option<u64> {
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
