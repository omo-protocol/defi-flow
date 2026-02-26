pub mod hyperliquid_native;
pub mod lifi;
pub mod simulator;

use std::future::Future;

use anyhow::Result;

use crate::data;
use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;
use crate::venues::perps::data::PerpCsvRow;

use super::{BuildMode, Venue, VenueCategory};

pub struct MovementCategory;

/// Stablecoins that don't need a price feed (valued at 1:1 USD).
const STABLECOINS: &[&str] = &["USDC", "USDT", "USDe", "sUSDe", "DAI", "FRAX"];

impl VenueCategory for MovementCategory {
    /// Swaps and bridges don't need historical data — slippage/fees are simulated.
    fn fetch_plan(_node: &Node) -> Option<FetchJob> {
        None
    }

    fn fetch(
        _client: &reqwest::Client,
        _job: &FetchJob,
        _config: &FetchConfig,
    ) -> impl Future<Output = Option<Result<FetchResult>>> + Send {
        async { None }
    }

    fn build(node: &Node, mode: &BuildMode) -> Result<Option<Box<dyn Venue>>> {
        match node {
            Node::Movement {
                movement_type,
                provider,
                to_token,
                ..
            } => match mode {
                BuildMode::Backtest {
                    slippage_bps,
                    manifest,
                    data_dir,
                    ..
                } => match movement_type {
                    crate::model::node::MovementType::Swap
                    | crate::model::node::MovementType::SwapBridge => {
                        let mut sim = simulator::SwapSimulator::new(*slippage_bps, 30.0);

                        // If output token is non-stablecoin, try to find a price feed
                        // from a perp node so the swap can track spot value.
                        let is_stable =
                            STABLECOINS.iter().any(|s| s.eq_ignore_ascii_case(to_token));

                        if !is_stable {
                            if let Some(price_data) =
                                find_perp_price_feed(manifest, data_dir, to_token)
                            {
                                sim = sim.with_price_feed(price_data);
                            }
                        }

                        Ok(Some(Box::new(sim)))
                    }
                    crate::model::node::MovementType::Bridge => {
                        Ok(Some(Box::new(simulator::BridgeSimulator::new(10.0))))
                    }
                },
                BuildMode::Live { config, tokens, .. } => {
                    use crate::model::node::MovementProvider;
                    match provider {
                        MovementProvider::LiFi => {
                            Ok(Some(Box::new(lifi::LiFiMovement::new(config, tokens)?)))
                        }
                        MovementProvider::HyperliquidNative => Ok(Some(Box::new(
                            hyperliquid_native::HyperliquidNativeMovement::new(config)?,
                        ))),
                    }
                }
            },
            _ => Ok(None),
        }
    }
}

/// Look through the manifest for a perp data file whose symbol matches `token`.
fn find_perp_price_feed(
    manifest: &std::collections::HashMap<String, data::ManifestEntry>,
    data_dir: &std::path::Path,
    token: &str,
) -> Option<Vec<PerpCsvRow>> {
    for entry in manifest.values() {
        if entry.kind == "perp" {
            // Try loading and check if the symbol matches
            if let Ok(rows) = data::load_csv::<PerpCsvRow>(data_dir, &entry.file) {
                if let Some(first) = rows.first() {
                    if first.symbol.eq_ignore_ascii_case(token) {
                        return Some(rows);
                    }
                }
            }
        }
    }
    None
}
