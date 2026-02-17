pub mod lifi;
pub mod simulator;

use std::future::Future;

use anyhow::Result;

use crate::fetch_data::types::{FetchConfig, FetchJob, FetchResult};
use crate::model::node::Node;

use super::{BuildMode, Venue, VenueCategory};

pub struct MovementCategory;

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
            Node::Swap { .. } => match mode {
                BuildMode::Backtest { slippage_bps, .. } => {
                    Ok(Some(Box::new(simulator::SwapSimulator::new(*slippage_bps, 30.0))))
                }
                BuildMode::Live { config } => {
                    Ok(Some(Box::new(lifi::LiFiMovement::new(config)?)))
                }
            },
            Node::Bridge { .. } => match mode {
                BuildMode::Backtest { .. } => {
                    Ok(Some(Box::new(simulator::BridgeSimulator::new(10.0))))
                }
                BuildMode::Live { config } => {
                    Ok(Some(Box::new(lifi::LiFiMovement::new(config)?)))
                }
            },
            _ => Ok(None),
        }
    }
}
