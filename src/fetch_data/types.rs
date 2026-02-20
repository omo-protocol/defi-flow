use crate::data::csv_types;

// ── Data source enum ────────────────────────────────────────────────

/// Which API / data source to use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataSource {
    HyperliquidPerp,
    HyperliquidSpot,
    Rysk,
    DefiLlamaYield,
    AerodromeSubgraph,
}

impl DataSource {
    pub fn name(&self) -> &'static str {
        match self {
            DataSource::HyperliquidPerp => "hyperliquid",
            DataSource::HyperliquidSpot => "hyperliquid",
            DataSource::Rysk => "rysk",
            DataSource::DefiLlamaYield => "defillama",
            DataSource::AerodromeSubgraph => "aerodrome",
        }
    }
}

// ── Fetch job ───────────────────────────────────────────────────────

/// A single data-fetch job (may serve multiple nodes).
pub struct FetchJob {
    /// Node IDs that will consume this data.
    pub node_ids: Vec<String>,
    /// Data source to query.
    pub source: DataSource,
    /// Source-specific key (coin symbol, pool name, venue+asset, etc.)
    pub key: String,
    /// CSV kind for the manifest.
    pub kind: String,
    /// Output filename.
    pub filename: String,
}

// ── Fetch config ────────────────────────────────────────────────────

/// Configuration for all fetchers.
pub struct FetchConfig {
    pub start_time_ms: u64,
    pub end_time_ms: u64,
    pub interval: String,
}

// ── Fetch result ────────────────────────────────────────────────────

/// Result of a fetch operation — typed by CSV row kind.
pub enum FetchResult {
    Perp(Vec<csv_types::PerpCsvRow>),
    Options(Vec<csv_types::OptionsCsvRow>),
    Lp(Vec<csv_types::LpCsvRow>),
    Lending(Vec<csv_types::LendingCsvRow>),
    Vault(Vec<csv_types::VaultCsvRow>),
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
            FetchResult::Vault(r) => r.len(),
            FetchResult::Pendle(r) => r.len(),
            FetchResult::Price(r) => r.len(),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

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

/// Extract the coin symbol from a pair string like "ETH/USDC" → "ETH".
pub fn coin_from_pair(pair: &str) -> String {
    pair.split('/').next().unwrap_or(pair).to_string()
}

/// Sanitize a string for use as a filename component.
pub fn sanitize(s: &str) -> String {
    s.to_lowercase()
        .replace('/', "_")
        .replace(' ', "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}
