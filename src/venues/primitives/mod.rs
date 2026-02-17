pub mod wallet;

use std::future::Future;

use anyhow::Result;

use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;

use super::{BuildMode, Venue, VenueCategory};

pub struct PrimitivesCategory;

impl VenueCategory for PrimitivesCategory {
    /// Wallet doesn't need historical data.
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

    fn build(node: &Node, _mode: &BuildMode) -> Result<Option<Box<dyn Venue>>> {
        match node {
            Node::Wallet { .. } => Ok(Some(Box::new(wallet::WalletVenue::new()))),
            _ => Ok(None),
        }
    }
}
