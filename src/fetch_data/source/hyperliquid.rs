use anyhow::{Context, Result};
use serde::Deserialize;

use crate::data::csv_types::{PerpCsvRow, PriceCsvRow};

use super::FetchConfig;

const API_URL: &str = "https://api.hyperliquid.xyz/info";
const MAX_CANDLES_PER_REQUEST: usize = 500;
const RATE_LIMIT_MS: u64 = 200;

// ── API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CandleRow {
    #[serde(rename = "t")]
    open_time: u64, // ms
    #[serde(rename = "T")]
    close_time: u64, // ms
    #[serde(rename = "o")]
    open: String,
    #[serde(rename = "h")]
    high: String,
    #[serde(rename = "l")]
    low: String,
    #[serde(rename = "c")]
    close: String,
    #[serde(rename = "v")]
    volume: String,
    #[serde(rename = "n")]
    _num_trades: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FundingEntry {
    coin: String,
    #[serde(rename = "fundingRate")]
    funding_rate: String,
    time: u64, // ms
}

// ── Public API ───────────────────────────────────────────────────────

/// Fetch perp candle + funding data from Hyperliquid API.
///
/// Produces one PerpCsvRow per candle period, merging with the nearest funding entry.
pub async fn fetch_perp(
    client: &reqwest::Client,
    coin: &str,
    config: &FetchConfig,
) -> Result<Vec<PerpCsvRow>> {
    let candles = fetch_candles(client, coin, config).await
        .with_context(|| format!("fetching candles for {coin}"))?;
    let funding = fetch_funding(client, coin, config).await
        .with_context(|| format!("fetching funding for {coin}"))?;

    // Build a lookup: timestamp_ms → funding_rate
    let mut funding_map: std::collections::HashMap<u64, f64> = std::collections::HashMap::new();
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

            // Find nearest funding entry (within the candle's time range)
            let funding_rate = find_nearest_funding(&funding_map, c.open_time, c.close_time);

            // Annualize: 8h rate * 3 * 365 (signed — positive = longs pay shorts)
            let funding_apy = funding_rate * 3.0 * 365.0;

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
    let candles = fetch_candles(client, coin, config).await
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

// ── Internal helpers ─────────────────────────────────────────────────

/// Fetch candles with pagination (max 500 per request).
async fn fetch_candles(
    client: &reqwest::Client,
    coin: &str,
    config: &FetchConfig,
) -> Result<Vec<CandleRow>> {
    let mut all_candles: Vec<CandleRow> = Vec::new();
    let mut start = config.start_time_ms;

    loop {
        let body = serde_json::json!({
            "type": "candleSnapshot",
            "req": {
                "coin": coin,
                "interval": &config.interval,
                "startTime": start,
                "endTime": config.end_time_ms,
            }
        });

        let resp = super::retry(3, || {
            let client = client.clone();
            let body = body.clone();
            async move {
                let r = client
                    .post(API_URL)
                    .json(&body)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Vec<CandleRow>>()
                    .await?;
                Ok(r)
            }
        })
        .await?;

        if resp.is_empty() {
            break;
        }

        let last_close_time = resp.last().unwrap().close_time;
        all_candles.extend(resp);

        // If we got fewer than max, we've reached the end
        if all_candles.len() % MAX_CANDLES_PER_REQUEST != 0 || last_close_time >= config.end_time_ms
        {
            break;
        }

        // Advance start past the last candle
        start = last_close_time + 1;
        tokio::time::sleep(std::time::Duration::from_millis(RATE_LIMIT_MS)).await;
    }

    Ok(all_candles)
}

/// Fetch funding history with pagination.
async fn fetch_funding(
    client: &reqwest::Client,
    coin: &str,
    config: &FetchConfig,
) -> Result<Vec<FundingEntry>> {
    let mut all_entries: Vec<FundingEntry> = Vec::new();
    let mut start = config.start_time_ms;

    loop {
        let body = serde_json::json!({
            "type": "fundingHistory",
            "coin": coin,
            "startTime": start,
        });

        let resp = super::retry(3, || {
            let client = client.clone();
            let body = body.clone();
            async move {
                let r = client
                    .post(API_URL)
                    .json(&body)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Vec<FundingEntry>>()
                    .await?;
                Ok(r)
            }
        })
        .await?;

        if resp.is_empty() {
            break;
        }

        let last_time = resp.last().unwrap().time;
        all_entries.extend(resp);

        if last_time >= config.end_time_ms {
            break;
        }

        start = last_time + 1;
        tokio::time::sleep(std::time::Duration::from_millis(RATE_LIMIT_MS)).await;
    }

    // Filter to only entries within our time range
    all_entries.retain(|e| e.time >= config.start_time_ms && e.time <= config.end_time_ms);

    Ok(all_entries)
}

/// Find the funding rate nearest to a candle's time range.
fn find_nearest_funding(
    funding_map: &std::collections::HashMap<u64, f64>,
    open_time: u64,
    close_time: u64,
) -> f64 {
    // Look for any entry within the candle period
    let mut best_rate = 0.0;
    let mut best_dist = u64::MAX;

    let mid = (open_time + close_time) / 2;

    for (&ts, &rate) in funding_map {
        if ts >= open_time && ts <= close_time {
            return rate;
        }
        let dist = if ts > mid { ts - mid } else { mid - ts };
        if dist < best_dist {
            best_dist = dist;
            best_rate = rate;
        }
    }

    // Only use if within 2x the candle duration
    let candle_duration = close_time - open_time;
    if best_dist <= candle_duration * 2 {
        best_rate
    } else {
        0.0
    }
}
