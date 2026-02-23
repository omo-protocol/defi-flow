pub mod aerodrome;
pub mod data;
pub mod fetch;
pub mod simulator;

use std::future::Future;

use anyhow::Result;

use crate::data as crate_data;
use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::chain::Chain;
use crate::model::node::Node;

use super::{BuildMode, Venue, VenueCategory};

pub struct LpCategory;

impl VenueCategory for LpCategory {
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
            Node::Lp { .. } => match mode {
                BuildMode::Backtest {
                    manifest,
                    data_dir,
                    ..
                } => {
                    let id = node.id();
                    let rows = if let Some(entry) = manifest.get(id) {
                        crate_data::load_csv::<self::data::LpCsvRow>(data_dir, &entry.file)?
                    } else {
                        vec![self::data::default_lp_row()]
                    };
                    Ok(Some(Box::new(simulator::LpSimulator::new(rows))))
                }
                BuildMode::Live { config, tokens, contracts } => {
                    let chain = node.chain().unwrap_or_else(Chain::base);
                    Ok(Some(Box::new(aerodrome::AerodromeLp::new(config, tokens, contracts, chain)?)))
                }
            },
            _ => Ok(None),
        }
    }
}
