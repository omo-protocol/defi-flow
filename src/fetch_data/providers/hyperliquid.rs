use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;

use super::super::types::FetchConfig;

const API_URL: &str = "https://api.hyperliquid.xyz/info";
const MAX_CANDLES_PER_REQUEST: usize = 500;
const RATE_LIMIT_MS: u64 = 200;

// ── API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CandleRow {
    #[serde(rename = "t")]
    pub open_time: u64, // ms
    #[serde(rename = "T")]
    pub close_time: u64, // ms
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "n")]
    pub _num_trades: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FundingEntry {
    pub coin: String,
    #[serde(rename = "fundingRate")]
    pub funding_rate: String,
    pub time: u64, // ms
}

// ── Public API ───────────────────────────────────────────────────────

/// Fetch candles with pagination (max 500 per request).
pub async fn fetch_candles(
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

        let resp = super::super::types::retry(3, || {
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
pub async fn fetch_funding(
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

        let resp = super::super::types::retry(3, || {
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
pub fn find_nearest_funding(
    funding_map: &HashMap<u64, f64>,
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
