use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::data::csv_types::{OptionsCsvRow, PriceCsvRow};
use crate::fetch_data::types::{sanitize, DataSource, FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;

const RYSK_API_URL: &str = "https://v12.rysk.finance/api/inventory";

// ── Rysk API response types ─────────────────────────────────────────

type InventoryResponse = HashMap<String, AssetInventory>;

#[derive(Debug, Deserialize)]
struct AssetInventory {
    combinations: HashMap<String, OptionEntry>,
}

#[derive(Debug, Deserialize)]
struct OptionEntry {
    strike: Option<f64>,
    #[serde(rename = "expiration_timestamp")]
    expiration_timestamp: Option<u64>,
    #[serde(rename = "isPut")]
    is_put: Option<bool>,
    #[serde(rename = "bidIv")]
    bid_iv: Option<f64>,
    #[serde(rename = "askIv")]
    ask_iv: Option<f64>,
    index: Option<f64>, // spot price
}

// ── Plan ────────────────────────────────────────────────────────────

pub fn fetch_plan(node: &Node) -> Option<FetchJob> {
    match node {
        Node::Options { id, asset, .. } => {
            let key = format!("{asset:?}");
            let source = DataSource::Rysk;
            let filename = format!("{}_{}.csv", source.name(), sanitize(&key));
            Some(FetchJob {
                node_ids: vec![id.clone()],
                source,
                key,
                kind: "options".to_string(),
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
    match &job.source {
        DataSource::Rysk => {
            // Fetch Hyperliquid spot prices as oracle for Rysk's synthetic history
            let oracle_prices = fetch_spot_oracle(client, &job.key, config).await.ok();
            Some(
                fetch_options(client, &job.key, config, oracle_prices.as_deref())
                    .await
                    .map(FetchResult::Options),
            )
        }
        _ => None,
    }
}

// ── Internal ────────────────────────────────────────────────────────

/// Fetch spot prices from Hyperliquid to use as oracle for Rysk synthetic data.
async fn fetch_spot_oracle(
    client: &reqwest::Client,
    asset: &str,
    config: &FetchConfig,
) -> Result<Vec<PriceCsvRow>> {
    crate::venues::perps::fetch::fetch_spot(client, asset, config).await
}

/// Fetch options data from Rysk API.
///
/// NOTE: Rysk only provides current inventory (no historical endpoint).
/// This fetches the current snapshot and generates synthetic historical data
/// by shifting premiums with underlying price movements.
async fn fetch_options(
    client: &reqwest::Client,
    asset: &str,
    config: &FetchConfig,
    oracle_prices: Option<&[PriceCsvRow]>,
) -> Result<Vec<OptionsCsvRow>> {
    if oracle_prices.is_some() {
        eprintln!(
            "  NOTE: Rysk provides current inventory only. \
             Using Hyperliquid oracle prices for historical spot simulation."
        );
    } else {
        eprintln!(
            "  NOTE: Rysk provides current inventory only (no historical API). \
             Generating synthetic history from current snapshot + linear drift."
        );
    }

    // Fetch current inventory
    let inventory = fetch_inventory(client)
        .await
        .context("fetching Rysk inventory")?;

    let asset_upper = asset.to_uppercase();
    let asset_data = inventory
        .iter()
        .find(|(k, _)| k.to_uppercase() == asset_upper || k.to_uppercase().contains(&asset_upper));

    let entries: Vec<(&String, &OptionEntry)> = match asset_data {
        Some((_, inv)) => inv.combinations.iter().collect(),
        None => {
            eprintln!("  WARN: No Rysk inventory found for asset '{asset}'. Writing empty CSV.");
            return Ok(Vec::new());
        }
    };

    if entries.is_empty() {
        return Ok(Vec::new());
    }

    // Get spot price from inventory
    let spot_price = entries
        .iter()
        .find_map(|(_, e)| e.index)
        .unwrap_or(1.0);

    let now_s = config.end_time_ms / 1000;
    let start_s = config.start_time_ms / 1000;

    // Generate daily snapshots by scaling the current inventory
    let day_step = 86400u64;

    let mut rows: Vec<OptionsCsvRow> = Vec::new();
    let mut snapshot_id = 1u64;

    let mut ts = start_s;
    while ts <= now_s {
        // Use oracle prices if available, otherwise fall back to linear drift
        let sim_spot = if let Some(prices) = oracle_prices {
            prices
                .iter()
                .min_by_key(|p| (p.timestamp as i64 - ts as i64).unsigned_abs())
                .map(|p| p.price)
                .unwrap_or(spot_price)
        } else {
            let progress = (ts - start_s) as f64 / (now_s - start_s).max(1) as f64;
            let drift = 1.0 - (1.0 - progress) * 0.15;
            spot_price * drift
        };

        for (key, entry) in &entries {
            let strike = entry.strike.unwrap_or(0.0);
            let expiry = entry.expiration_timestamp.unwrap_or(0);
            let is_put = entry.is_put.unwrap_or(false);

            // Skip expired options
            if expiry > 0 && expiry < ts {
                continue;
            }

            let days_to_expiry = if expiry > ts {
                (expiry - ts) as f64 / 86400.0
            } else {
                30.0 // default
            };

            // Scale premium with spot price movement and time decay
            let mid_iv = ((entry.bid_iv.unwrap_or(50.0) + entry.ask_iv.unwrap_or(50.0)) / 2.0)
                / 100.0;
            let time_factor = (days_to_expiry / 365.0).sqrt();
            let premium = sim_spot * mid_iv * time_factor * 0.4;

            let collateral = if is_put { strike } else { sim_spot };
            let apy = if collateral > 0.0 && days_to_expiry > 0.0 {
                (premium / collateral) * (365.0 / days_to_expiry)
            } else {
                0.0
            };

            let moneyness = sim_spot / strike;
            let delta = if is_put {
                Some(if moneyness > 1.0 { -0.2 } else { -0.5 })
            } else {
                Some(if moneyness > 1.0 { 0.8 } else { 0.5 })
            };

            rows.push(OptionsCsvRow {
                snapshot: snapshot_id,
                timestamp: ts,
                spot_price: sim_spot,
                asset: asset.to_string(),
                address: key.to_string(),
                option_type: if is_put {
                    "put".to_string()
                } else {
                    "call".to_string()
                },
                strike,
                expiry,
                days_to_expiry,
                premium,
                apy,
                delta,
            });
        }

        snapshot_id += 1;
        ts += day_step;
    }

    Ok(rows)
}

async fn fetch_inventory(client: &reqwest::Client) -> Result<InventoryResponse> {
    let resp = crate::fetch_data::types::retry(3, || {
        let client = client.clone();
        async move {
            let r = client
                .get(RYSK_API_URL)
                .send()
                .await?
                .error_for_status()?
                .json::<InventoryResponse>()
                .await?;
            Ok(r)
        }
    })
    .await
    .context("fetching Rysk inventory")?;

    Ok(resp)
}
