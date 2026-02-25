pub mod data;
pub mod fetch;
pub mod hyperliquid;
pub mod simulator;

use std::future::Future;

use anyhow::{bail, Result};

use crate::data as crate_data;
use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::{Node, PerpVenue};

use super::{BuildMode, Venue, VenueCategory};

pub struct PerpsCategory;

impl VenueCategory for PerpsCategory {
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
            Node::Perp { venue, .. } => match mode {
                BuildMode::Backtest {
                    manifest,
                    data_dir,
                    slippage_bps,
                    seed,
                    ..
                } => {
                    let id = node.id();
                    let rows = match manifest.get(id) {
                        Some(entry) => {
                            crate_data::load_csv::<self::data::PerpCsvRow>(data_dir, &entry.file)?
                        }
                        None => bail!(
                            "Node '{}' has no manifest entry. Run `defi-flow fetch-data` \
                             or add it to manifest.json (kind: \"perp\")",
                            id
                        ),
                    };
                    Ok(Some(Box::new(simulator::PerpSimulator::new(
                        rows,
                        *slippage_bps,
                        *seed,
                    ))))
                }
                BuildMode::Live { config, .. } => match venue {
                    PerpVenue::Hyperliquid | PerpVenue::Hyena => {
                        Ok(Some(Box::new(hyperliquid::HyperliquidPerp::new(config)?)))
                    }
                },
            },
            Node::Spot { .. } => match mode {
                BuildMode::Backtest {
                    manifest,
                    data_dir,
                    slippage_bps,
                    ..
                } => {
                    let id = node.id();
                    let rows = match manifest.get(id) {
                        Some(entry) => {
                            crate_data::load_csv::<self::data::PriceCsvRow>(data_dir, &entry.file)?
                        }
                        None => bail!(
                            "Node '{}' has no manifest entry. Run `defi-flow fetch-data` \
                             or add it to manifest.json (kind: \"spot\")",
                            id
                        ),
                    };
                    Ok(Some(Box::new(simulator::SpotSimulator::new(
                        rows,
                        *slippage_bps,
                    ))))
                }
                BuildMode::Live { config, .. } => {
                    Ok(Some(Box::new(hyperliquid::HyperliquidPerp::new(config)?)))
                }
            },
            _ => Ok(None),
        }
    }
}
