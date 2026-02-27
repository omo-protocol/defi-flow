pub mod data;
pub mod fetch;
pub mod morpho;
pub mod simulator;

use std::future::Future;

use anyhow::Result;

use crate::data as crate_data;
use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;

use super::{BuildMode, Venue, VenueCategory};

pub struct VaultCategory;

impl VenueCategory for VaultCategory {
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
            Node::Vault { .. } => match mode {
                BuildMode::Backtest {
                    manifest, data_dir, ..
                } => {
                    let id = node.id();
                    let rows = if let Some(entry) = manifest.get(id) {
                        crate_data::load_csv::<self::data::VaultCsvRow>(data_dir, &entry.file)?
                    } else {
                        vec![self::data::default_vault_row()]
                    };
                    Ok(Some(Box::new(simulator::VaultSimulator::new(rows))))
                }
                BuildMode::Live {
                    config,
                    tokens,
                    contracts,
                } => Ok(Some(Box::new(morpho::MorphoVault::new(
                    config, tokens, contracts, node,
                )?))),
            },
            _ => Ok(None),
        }
    }
}
