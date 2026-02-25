pub mod aave;
pub mod data;
pub mod fetch;
pub mod simulator;

use std::future::Future;

use anyhow::{bail, Result};

use crate::data as crate_data;
use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;
use crate::venues::perps::data::PriceCsvRow;

use super::{BuildMode, Venue, VenueCategory};

/// Stablecoins that don't need a price feed (assumed pegged ~$1).
const STABLECOINS: &[&str] = &["USDC", "USDT", "DAI", "FRAX", "LUSD", "USDX", "GHO", "crvUSD", "PYUSD"];

pub struct LendingCategory;

impl VenueCategory for LendingCategory {
    fn fetch_plan(node: &Node) -> Option<FetchJob> {
        fetch::fetch_plan(node)
    }

    fn fetch(
        client: &reqwest::Client,
        job: &FetchJob,
        config: &FetchConfig,
    ) -> impl Future<Output = Option<Result<FetchResult>>> + Send {
        fetch::fetch(client, job, config)
    }

    fn build(node: &Node, mode: &BuildMode) -> Result<Option<Box<dyn Venue>>> {
        match node {
            Node::Lending { asset, .. } => match mode {
                BuildMode::Backtest {
                    manifest,
                    data_dir,
                    ..
                } => {
                    let id = node.id();
                    let rows = match manifest.get(id) {
                        Some(entry) => {
                            crate_data::load_csv::<self::data::LendingCsvRow>(data_dir, &entry.file)?
                        }
                        None => bail!(
                            "Node '{}' has no manifest entry. Run `defi-flow fetch-data` \
                             or add it to manifest.json (kind: \"lending\")",
                            id
                        ),
                    };

                    let mut sim = simulator::LendingSimulator::new(rows);

                    // For non-stablecoin assets, find a spot price feed in the manifest
                    // so the simulator can mark-to-market correctly.
                    if !STABLECOINS.iter().any(|s| s.eq_ignore_ascii_case(asset)) {
                        let price_feed = find_spot_price_feed(manifest, data_dir);
                        if let Some(prices) = price_feed {
                            sim = sim.with_price_feed(prices);
                        } else {
                            eprintln!(
                                "warning: lending node '{}' holds non-stablecoin '{}' but no \
                                 spot price feed found in manifest. Values won't mark-to-market.",
                                id, asset
                            );
                        }
                    }

                    Ok(Some(Box::new(sim)))
                }
                BuildMode::Live { config, tokens, contracts } => {
                    Ok(Some(Box::new(aave::AaveLending::new(config, tokens, contracts)?)))
                }
            },
            _ => Ok(None),
        }
    }
}

/// Search the manifest for a "spot" kind entry and load its PriceCsvRow data.
fn find_spot_price_feed(
    manifest: &std::collections::HashMap<String, crate::data::ManifestEntry>,
    data_dir: &std::path::Path,
) -> Option<Vec<PriceCsvRow>> {
    for entry in manifest.values() {
        if entry.kind == "spot" {
            if let Ok(rows) = crate_data::load_csv::<PriceCsvRow>(data_dir, &entry.file) {
                if !rows.is_empty() {
                    return Some(rows);
                }
            }
        }
    }
    None
}
