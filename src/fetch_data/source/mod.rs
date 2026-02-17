pub mod aerodrome;
pub mod defillama;
pub mod hyperliquid;
pub mod rysk;

use crate::data::csv_types;

/// Configuration for all fetchers.
pub struct FetchConfig {
    pub start_time_ms: u64,
    pub end_time_ms: u64,
    pub interval: String,
}

/// Result of a fetch operation — typed by CSV row kind.
pub enum FetchResult {
    Perp(Vec<csv_types::PerpCsvRow>),
    Options(Vec<csv_types::OptionsCsvRow>),
    Lp(Vec<csv_types::LpCsvRow>),
    Lending(Vec<csv_types::LendingCsvRow>),
    Pendle(Vec<csv_types::PendleCsvRow>),
    Price(Vec<csv_types::PriceCsvRow>),
}

impl FetchResult {
    pub fn row_count(&self) -> usize {
        match self {
            FetchResult::Perp(r) => r.len(),
            FetchResult::Options(r) => r.len(),
            FetchResult::Lp(r) => r.len(),
            FetchResult::Lending(r) => r.len(),
            FetchResult::Pendle(r) => r.len(),
            FetchResult::Price(r) => r.len(),
        }
    }
}

/// Retry an async operation with exponential backoff.
pub async fn retry<T, F, Fut>(max_retries: u32, f: F) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut last_err = None;
    for attempt in 0..=max_retries {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = Some(e);
                if attempt < max_retries {
                    let delay = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(last_err.unwrap())
}
