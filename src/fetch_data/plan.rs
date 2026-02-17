use std::collections::HashMap;

use crate::model::node::{LendingVenue, Node};
use crate::model::workflow::Workflow;

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

/// Map a LendingVenue to its DefiLlama project slug.
pub fn lending_venue_slug(venue: &LendingVenue) -> &'static str {
    match venue {
        LendingVenue::Aave => "aave-v3",
        LendingVenue::Lendle => "lendle",
        LendingVenue::Morpho => "morpho-blue",
        LendingVenue::Compound => "compound-v3",
        LendingVenue::InitCapital => "init-capital",
        LendingVenue::HyperLend => "hyperlend-pooled",
    }
}

/// Extract the coin symbol from a pair string like "ETH/USDC" → "ETH".
fn coin_from_pair(pair: &str) -> String {
    pair.split('/').next().unwrap_or(pair).to_string()
}

/// Sanitize a string for use as a filename component.
fn sanitize(s: &str) -> String {
    s.to_lowercase()
        .replace('/', "_")
        .replace(' ', "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Scan a workflow and produce a deduplicated list of fetch jobs.
///
/// Multiple nodes that need the same data (e.g. perp_eth_long and collect_eth_funding)
/// are grouped into a single FetchJob.
pub fn build_plan(workflow: &Workflow) -> Vec<FetchJob> {
    // Group by (source, key) → (node_ids, kind)
    let mut groups: HashMap<(DataSource, String), (Vec<String>, String)> = HashMap::new();

    for node in &workflow.nodes {
        let entry = match node {
            Node::Perp { id, pair, venue, .. } => {
                let coin = match venue {
                    crate::model::node::PerpVenue::Hyena => {
                        format!("hyna:{}", coin_from_pair(pair))
                    }
                    _ => coin_from_pair(pair),
                };
                Some((id.clone(), DataSource::HyperliquidPerp, coin, "perp".to_string()))
            }
            Node::Options { id, asset, .. } => {
                Some((id.clone(), DataSource::Rysk, format!("{asset:?}"), "options".to_string()))
            }
            Node::Spot { id, pair, .. } => {
                let coin = coin_from_pair(pair);
                Some((id.clone(), DataSource::HyperliquidSpot, coin, "spot".to_string()))
            }
            Node::Lp { id, pool, .. } => {
                Some((id.clone(), DataSource::AerodromeSubgraph, pool.clone(), "lp".to_string()))
            }
            Node::Lending { id, venue, asset, .. } => {
                let slug = lending_venue_slug(venue);
                let key = format!("{slug}:{asset}");
                Some((id.clone(), DataSource::DefiLlamaYield, key, "lending".to_string()))
            }
            Node::Pendle { id, market, .. } => {
                let key = format!("pendle:{market}");
                Some((id.clone(), DataSource::DefiLlamaYield, key, "pendle".to_string()))
            }
            // These don't need external data
            Node::Wallet { .. }
            | Node::Swap { .. }
            | Node::Bridge { .. }
            | Node::Optimizer { .. } => None,
        };

        if let Some((node_id, source, key, kind)) = entry {
            let group_key = (source, key);
            groups
                .entry(group_key)
                .or_insert_with(|| (Vec::new(), kind))
                .0
                .push(node_id);
        }
    }

    groups
        .into_iter()
        .map(|((source, key), (node_ids, kind))| {
            let filename = format!("{}_{}.csv", source.name(), sanitize(&key));
            FetchJob {
                node_ids,
                source,
                key,
                kind,
                filename,
            }
        })
        .collect()
}
