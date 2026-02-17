use anyhow::{Context, Result};

use crate::data::csv_types::PendleCsvRow;
use crate::fetch_data::providers::defillama;
use crate::fetch_data::types::{sanitize, DataSource, FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;

// ── Plan ────────────────────────────────────────────────────────────

pub fn fetch_plan(node: &Node) -> Option<FetchJob> {
    match node {
        Node::Pendle { id, market, .. } => {
            let key = format!("pendle:{market}");
            let source = DataSource::DefiLlamaYield;
            let filename = format!("{}_{}.csv", source.name(), sanitize(&key));
            Some(FetchJob {
                node_ids: vec![id.clone()],
                source,
                key,
                kind: "pendle".to_string(),
                filename,
            })
        }
        _ => None,
    }
}

// ── Fetch ───────────────────────────────────────────────────────────

pub async fn fetch(
    client: &reqwest::Client,
    job: &FetchJob,
    config: &FetchConfig,
) -> Option<Result<FetchResult>> {
    if job.source != DataSource::DefiLlamaYield || job.kind != "pendle" {
        return None;
    }

    let market = job.key.strip_prefix("pendle:").unwrap_or(&job.key);
    Some(fetch_pendle(client, market, config).await.map(FetchResult::Pendle))
}

// ── Internal ────────────────────────────────────────────────────────

/// Fetch Pendle market data from DefiLlama yields API.
async fn fetch_pendle(
    client: &reqwest::Client,
    market: &str,
    config: &FetchConfig,
) -> Result<Vec<PendleCsvRow>> {
    let pool_id = defillama::find_pool(client, "pendle", market, None)
        .await
        .with_context(|| format!("finding DefiLlama pool for pendle/{market}"))?;

    let chart = defillama::fetch_chart(client, &pool_id)
        .await
        .with_context(|| format!("fetching chart for Pendle pool {pool_id}"))?;

    let start_s = config.start_time_ms / 1000;
    let end_s = config.end_time_ms / 1000;

    // Estimate maturity from market name (default: 6 months from now)
    let maturity = config.end_time_ms / 1000 + 180 * 86400;

    let rows: Vec<PendleCsvRow> = chart
        .iter()
        .filter_map(|p| {
            let ts = defillama::parse_timestamp(&p.timestamp)?;
            if ts < start_s || ts > end_s {
                return None;
            }

            let implied_apy = p.apy.unwrap_or(0.0) / 100.0;
            let time_to_maturity_years = (maturity - ts) as f64 / (365.25 * 86400.0);

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
                underlying_price: 1.0,
                maturity,
            })
        })
        .collect();

    Ok(rows)
}
