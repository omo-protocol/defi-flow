use anyhow::{Context, Result};

use crate::data::csv_types::LendingCsvRow;
use crate::fetch_data::providers::defillama;
use crate::fetch_data::types::{DataSource, FetchConfig, FetchJob, FetchResult, sanitize};
use crate::model::node::Node;

// ── Plan ────────────────────────────────────────────────────────────

pub fn fetch_plan(node: &Node) -> Option<FetchJob> {
    match node {
        Node::Lending {
            id,
            defillama_slug,
            asset,
            ..
        } => {
            let slug = defillama_slug.as_deref()?;
            let key = format!("{slug}:{asset}");
            let source = DataSource::DefiLlamaYield;
            let filename = format!("{}_{}.csv", source.name(), sanitize(&key));
            Some(FetchJob {
                node_ids: vec![id.clone()],
                source,
                key,
                kind: "lending".to_string(),
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
    if job.source != DataSource::DefiLlamaYield || job.kind != "lending" {
        return None;
    }

    let (venue, asset) = job.key.split_once(':').unwrap_or(("unknown", &job.key));
    Some(
        fetch_lending(client, venue, asset, config)
            .await
            .map(FetchResult::Lending),
    )
}

// ── Internal ────────────────────────────────────────────────────────

/// Fetch lending rate history from DefiLlama yields API.
async fn fetch_lending(
    client: &reqwest::Client,
    venue_slug: &str,
    asset: &str,
    config: &FetchConfig,
) -> Result<Vec<LendingCsvRow>> {
    let pool_id = defillama::find_pool(client, venue_slug, asset, None)
        .await
        .with_context(|| format!("finding DefiLlama pool for {venue_slug}/{asset}"))?;

    let chart = defillama::fetch_chart(client, &pool_id)
        .await
        .with_context(|| format!("fetching chart for pool {pool_id}"))?;

    let start_s = config.start_time_ms / 1000;
    let end_s = config.end_time_ms / 1000;

    let rows: Vec<LendingCsvRow> = chart
        .iter()
        .filter_map(|p| {
            let ts = defillama::parse_timestamp(&p.timestamp)?;
            if ts < start_s || ts > end_s {
                return None;
            }
            Some(LendingCsvRow {
                timestamp: ts,
                supply_apy: p.apy_base.unwrap_or(0.0) / 100.0,
                borrow_apy: p
                    .apy_base_borrow
                    .unwrap_or_else(|| p.apy_base.unwrap_or(0.0) * 1.3)
                    / 100.0,
                utilization: 0.0,
                reward_apy: p.apy_reward.unwrap_or(0.0) / 100.0,
            })
        })
        .collect();

    Ok(rows)
}
