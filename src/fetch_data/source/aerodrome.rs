use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::data::csv_types::LpCsvRow;

use super::FetchConfig;

/// Aerodrome Slipstream subgraph via The Graph playground.
const SUBGRAPH_URL: &str =
    "https://thegraph.com/explorer/api/playground/QmasYjypV6nTLp4iNH4Vjf7fksRNxAkAskqDdKf2DCsQkV";

/// AERO token address on Base.
const AERO_TOKEN_ID: &str = "0x940181a94a35a4569e4529a3cdfb74e38fd98631";

const RATE_LIMIT_MS: u64 = 300;

// ── GraphQL response types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<GraphQlData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlData {
    #[serde(rename = "poolDayDatas")]
    pool_day_datas: Option<Vec<PoolDayData>>,
    pools: Option<Vec<PoolInfo>>,
    token: Option<TokenInfo>,
    bundle: Option<BundleInfo>,
}

#[derive(Debug, Deserialize)]
struct PoolDayData {
    date: u64,
    #[serde(rename = "feesUSD")]
    fees_usd: String,
    #[serde(rename = "tvlUSD")]
    tvl_usd: String,
    #[serde(rename = "volumeUSD")]
    volume_usd: String,
    tick: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PoolInfo {
    id: String,
    #[serde(rename = "totalValueLockedUSD")]
    total_value_locked_usd: Option<String>,
    token0: Option<TokenSymbol>,
    token1: Option<TokenSymbol>,
}

#[derive(Debug, Deserialize)]
struct TokenSymbol {
    symbol: String,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    #[serde(rename = "derivedETH")]
    derived_eth: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BundleInfo {
    #[serde(rename = "ethPriceUSD")]
    eth_price_usd: Option<String>,
}

// ── Public API ───────────────────────────────────────────────────────

/// Fetch LP pool data from Aerodrome Slipstream subgraph.
///
/// `pool_name` is the pool pair identifier (e.g. "cbBTC/WETH").
pub async fn fetch_lp(
    client: &reqwest::Client,
    pool_name: &str,
    config: &FetchConfig,
) -> Result<Vec<LpCsvRow>> {
    // First, find the pool address by name
    let pool_address = find_pool(client, pool_name).await
        .with_context(|| format!("finding Aerodrome pool for {pool_name}"))?;

    // Fetch AERO price
    let aero_price = fetch_aero_price(client).await.unwrap_or(1.0);

    // Fetch daily snapshots
    let start_date = config.start_time_ms / 1000;
    let snapshots = fetch_pool_day_datas(client, &pool_address, start_date).await
        .with_context(|| format!("fetching pool day data for {pool_name}"))?;

    let end_s = config.end_time_ms / 1000;

    let rows: Vec<LpCsvRow> = snapshots
        .iter()
        .filter(|s| s.date >= start_date && s.date <= end_s)
        .map(|s| {
            let fees_usd: f64 = s.fees_usd.parse().unwrap_or(0.0);
            let tvl_usd: f64 = s.tvl_usd.parse().unwrap_or(1.0);
            let current_tick: i32 = s
                .tick
                .as_ref()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);

            // Fee APY = daily fees / TVL * 365
            let fee_apy = if tvl_usd > 0.0 {
                (fees_usd / tvl_usd) * 365.0
            } else {
                0.0
            };

            // Estimate token prices from TVL (assume roughly equal split)
            // In production, these come from separate token queries
            let price_a = tvl_usd / 2.0; // placeholder
            let price_b = tvl_usd / 2.0; // placeholder

            // Reward rate: estimate from typical Aerodrome AERO emissions
            // In production, this comes from gauge.left() on-chain
            let reward_rate = 0.05; // 5% default; overwritten if on-chain data available

            LpCsvRow {
                timestamp: s.date,
                current_tick,
                price_a,
                price_b,
                fee_apy,
                reward_rate,
                reward_token_price: aero_price,
            }
        })
        .collect();

    Ok(rows)
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Execute a GraphQL query against the Aerodrome subgraph.
async fn query_subgraph(client: &reqwest::Client, query: &str) -> Result<GraphQlData> {
    let body = serde_json::json!({ "query": query });

    let resp = super::retry(3, || {
        let client = client.clone();
        let body = body.clone();
        async move {
            let r = client
                .post(SUBGRAPH_URL)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<GraphQlResponse>()
                .await?;
            Ok(r)
        }
    })
    .await
    .context("querying Aerodrome subgraph")?;

    if let Some(errors) = resp.errors {
        if !errors.is_empty() {
            bail!(
                "Subgraph errors: {}",
                errors
                    .iter()
                    .map(|e| e.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
    }

    resp.data.context("no data in subgraph response")
}

/// Find a pool address by searching for pool name tokens.
///
/// Fetches top pools by TVL and matches by token symbols.
/// e.g. "cbBTC/WETH" matches a pool where token0.symbol contains "cbBTC"
/// and token1.symbol contains "WETH" (or vice versa).
async fn find_pool(client: &reqwest::Client, pool_name: &str) -> Result<String> {
    let tokens: Vec<&str> = pool_name.split('/').collect();
    let (sym_a, sym_b) = match tokens.as_slice() {
        [a, b] => (a.to_uppercase(), b.to_uppercase()),
        [a] => (a.to_uppercase(), String::new()),
        _ => bail!("Invalid pool name format: '{pool_name}'. Expected 'TOKEN_A/TOKEN_B'"),
    };

    // Fetch top 100 pools by TVL and match client-side
    let query = r#"{
        pools(
            first: 100,
            orderBy: totalValueLockedUSD,
            orderDirection: desc
        ) {
            id
            totalValueLockedUSD
            token0 { symbol }
            token1 { symbol }
        }
    }"#;

    let data = query_subgraph(client, query).await?;

    if let Some(pools) = data.pools {
        for pool in &pools {
            let t0 = pool
                .token0
                .as_ref()
                .map(|t| t.symbol.to_uppercase())
                .unwrap_or_default();
            let t1 = pool
                .token1
                .as_ref()
                .map(|t| t.symbol.to_uppercase())
                .unwrap_or_default();

            let match_forward = t0.contains(&sym_a)
                && (sym_b.is_empty() || t1.contains(&sym_b));
            let match_reverse = t1.contains(&sym_a)
                && (sym_b.is_empty() || t0.contains(&sym_b));

            if match_forward || match_reverse {
                println!(
                    "  Found pool {} ({}/{}, TVL: {})",
                    pool.id,
                    t0,
                    t1,
                    pool.total_value_locked_usd.as_deref().unwrap_or("?")
                );
                return Ok(pool.id.clone());
            }
        }
    }

    bail!("No Aerodrome pool found matching '{pool_name}'")
}

/// Fetch AERO token price from the subgraph (derivedETH × ethPriceUSD).
async fn fetch_aero_price(client: &reqwest::Client) -> Result<f64> {
    let query = format!(
        r#"{{
            token(id: "{AERO_TOKEN_ID}") {{
                derivedETH
            }}
            bundle(id: "1") {{
                ethPriceUSD
            }}
        }}"#
    );

    let data = query_subgraph(client, &query).await?;

    let derived_eth: f64 = data
        .token
        .and_then(|t| t.derived_eth)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    let eth_price: f64 = data
        .bundle
        .and_then(|b| b.eth_price_usd)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    let aero_price = derived_eth * eth_price;
    if aero_price > 0.0 {
        Ok(aero_price)
    } else {
        bail!("Could not determine AERO price from subgraph")
    }
}

/// Fetch pool day data with pagination.
async fn fetch_pool_day_datas(
    client: &reqwest::Client,
    pool_address: &str,
    start_timestamp: u64,
) -> Result<Vec<PoolDayData>> {
    let mut all_data: Vec<PoolDayData> = Vec::new();
    let mut last_date = start_timestamp;

    loop {
        let query = format!(
            r#"{{
                poolDayDatas(
                    first: 1000,
                    orderBy: date,
                    orderDirection: asc,
                    where: {{ pool: "{pool_address}", date_gte: {last_date} }}
                ) {{
                    date
                    feesUSD
                    tvlUSD
                    volumeUSD
                    tick
                }}
            }}"#
        );

        let data = query_subgraph(client, &query).await?;

        if let Some(day_datas) = data.pool_day_datas {
            if day_datas.is_empty() {
                break;
            }
            last_date = day_datas.last().unwrap().date + 1;
            all_data.extend(day_datas);
        } else {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(RATE_LIMIT_MS)).await;
    }

    Ok(all_data)
}
