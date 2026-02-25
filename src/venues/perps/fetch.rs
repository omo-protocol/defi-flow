use std::collections::HashMap;

use anyhow::{Context, Result};

use crate::data::csv_types::{PerpCsvRow, PriceCsvRow};
use crate::fetch_data::providers::hyperliquid;
use crate::fetch_data::types::{
    coin_from_pair, sanitize, DataSource, FetchConfig, FetchJob, FetchResult,
};
use crate::model::node::Node;

// ── Plan ────────────────────────────────────────────────────────────

/// Determine what data a perp/spot node needs.
pub fn fetch_plan(node: &Node) -> Option<FetchJob> {
    match node {
        Node::Perp { id, pair, venue, .. } => {
            let coin = match venue {
                crate::model::node::PerpVenue::Hyena => {
                    format!("hyna:{}", coin_from_pair(pair))
                }
                _ => coin_from_pair(pair),
            };
            let source = DataSource::HyperliquidPerp;
            let filename = format!("{}_{}.csv", source.name(), sanitize(&coin));
            Some(FetchJob {
                node_ids: vec![id.clone()],
                source,
                key: coin,
                kind: "perp".to_string(),
                filename,
            })
        }
        Node::Spot { id, pair, .. } => {
            let coin = coin_from_pair(pair);
            let source = DataSource::HyperliquidSpot;
            let filename = format!("{}_{}.csv", source.name(), sanitize(&coin));
            Some(FetchJob {
                node_ids: vec![id.clone()],
                source,
                key: coin,
                kind: "spot".to_string(),
                filename,
            })
        }
        _ => None,
    }
}

// ── Fetch ───────────────────────────────────────────────────────────

/// Dispatch a fetch for perp/spot data sources.
pub async fn fetch(
    client: &reqwest::Client,
    job: &FetchJob,
    config: &FetchConfig,
) -> Option<Result<FetchResult>> {
    match &job.source {
        DataSource::HyperliquidPerp => {
            Some(fetch_perp(client, &job.key, config).await.map(FetchResult::Perp))
        }
        DataSource::HyperliquidSpot => {
            Some(fetch_spot(client, &job.key, config).await.map(FetchResult::Price))
        }
        _ => None,
    }
}

// ── Internal ────────────────────────────────────────────────────────

/// Fetch perp candle + funding data from Hyperliquid API.
async fn fetch_perp(
    client: &reqwest::Client,
    coin: &str,
    config: &FetchConfig,
) -> Result<Vec<PerpCsvRow>> {
    let candles = hyperliquid::fetch_candles(client, coin, config)
        .await
        .with_context(|| format!("fetching candles for {coin}"))?;
    let funding = hyperliquid::fetch_funding(client, coin, config)
        .await
        .with_context(|| format!("fetching funding for {coin}"))?;

    // Build a lookup: timestamp_ms → funding_rate
    let mut funding_map: HashMap<u64, f64> = HashMap::new();
    for f in &funding {
        let rate: f64 = f.funding_rate.parse().unwrap_or(0.0);
        funding_map.insert(f.time, rate);
    }

    let rows: Vec<PerpCsvRow> = candles
        .iter()
        .map(|c| {
            let close: f64 = c.close.parse().unwrap_or(0.0);
            let high: f64 = c.high.parse().unwrap_or(close);
            let low: f64 = c.low.parse().unwrap_or(close);
            let volume: f64 = c.volume.parse().unwrap_or(0.0);
            let timestamp_s = c.open_time / 1000;

            let funding_rate =
                hyperliquid::find_nearest_funding(&funding_map, c.open_time, c.close_time);

            // Annualize: hourly rate * 24 * 365 (Hyperliquid settles funding every hour)
            let funding_apy = funding_rate * 24.0 * 365.0;

            // Synthetic bid/ask spread (~5bps from close)
            let spread = close * 0.0005;

            PerpCsvRow {
                symbol: coin.to_string(),
                mark_price: close,
                index_price: close,
                funding_rate,
                open_interest: 0.0,
                volume_24h: volume,
                bid: close - spread,
                ask: close + spread,
                mid_price: close,
                last_price: close,
                premium: (high - low) / close,
                basis: 0.0,
                timestamp: timestamp_s,
                funding_apy,
                rewards_apy: 0.0,
            }
        })
        .collect();

    Ok(rows)
}

/// Fetch spot price candles from Hyperliquid API.
pub async fn fetch_spot(
    client: &reqwest::Client,
    coin: &str,
    config: &FetchConfig,
) -> Result<Vec<PriceCsvRow>> {
    let candles = hyperliquid::fetch_candles(client, coin, config)
        .await
        .with_context(|| format!("fetching spot candles for {coin}"))?;

    let rows: Vec<PriceCsvRow> = candles
        .iter()
        .map(|c| {
            let close: f64 = c.close.parse().unwrap_or(0.0);
            let spread = close * 0.0005;
            PriceCsvRow {
                timestamp: c.open_time / 1000,
                price: close,
                bid: close - spread,
                ask: close + spread,
            }
        })
        .collect();

    Ok(rows)
}
