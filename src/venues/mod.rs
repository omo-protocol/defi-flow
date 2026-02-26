pub mod evm;
pub mod lending;
pub mod lp;
pub mod movement;
pub mod options;
pub mod perps;
pub mod primitives;
pub mod vault;
pub mod yield_tokens;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::data::ManifestEntry;
use crate::fetch_data::types::{DataSource, FetchConfig, FetchJob, FetchResult};
use crate::model::node::{Node, NodeId};
use crate::model::workflow::Workflow;
use crate::run::config::RuntimeConfig;

// ── Execution result ────────────────────────────────────────────────

/// Result of executing a node action.
#[derive(Debug)]
pub enum ExecutionResult {
    /// Produced tokens (swap output, claimed rewards, withdrawal, etc.)
    TokenOutput { token: String, amount: f64 },
    /// Position opened/modified. Consumed input, optionally produced output (e.g. premium).
    PositionUpdate {
        consumed: f64,
        output: Option<(String, f64)>,
    },
    /// No output (e.g. stake gauge, adjust leverage).
    Noop,
}

/// Risk parameters for smooth Kelly optimization.
/// Each venue can report its catastrophic loss probability, severity, and rebalance cost.
#[derive(Debug, Clone, Default)]
pub struct RiskParams {
    /// Annualized probability of catastrophic loss (liquidation, full IL, etc.)
    pub p_loss: f64,
    /// Fraction of allocated capital lost in that event (0.0 – 1.0)
    pub loss_severity: f64,
    /// Transaction/rebalance cost as fraction per rebalance
    pub rebalance_cost: f64,
}

/// Metrics reported by a venue at finalization.
#[derive(Debug, Default, Clone)]
pub struct SimMetrics {
    pub funding_pnl: f64,
    pub rewards_pnl: f64,
    pub premium_pnl: f64,
    pub lp_fees: f64,
    pub lending_interest: f64,
    pub swap_costs: f64,
    pub liquidations: u32,
}

// ── Unified Venue trait ─────────────────────────────────────────────

/// Unified venue trait. Every simulator and live executor implements this.
///
/// Simulator implementations use trivially-async methods (no actual I/O).
/// Live executor implementations do real network I/O (API calls, on-chain txns).
#[async_trait]
pub trait Venue: Send + Sync {
    /// Execute the node's action given available input amount.
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult>;

    /// Current total value of positions held at this venue (USD terms).
    async fn total_value(&self) -> Result<f64>;

    /// Advance internal state by one tick.
    /// `now` is the unix timestamp, `dt_secs` is seconds since the previous tick.
    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()>;

    /// Unwind (reduce) positions by the given fraction (0.0–1.0).
    /// Returns the USD value of capital freed.
    ///
    /// Used by:
    /// - **Optimizer rebalancing**: unwind over-allocated venues to redistribute capital.
    /// - **Reserve management**: unwind pro-rata from all venues to replenish vault reserve.
    async fn unwind(&mut self, fraction: f64) -> Result<f64>;

    /// Report accumulated metrics. Default: all zeros.
    fn metrics(&self) -> SimMetrics {
        SimMetrics::default()
    }

    /// Compute annualized alpha stats from historical data up to the current cursor.
    /// Returns `(expected_return, volatility)` — the yield component only,
    /// not directional exposure. Used by the adaptive Kelly optimizer.
    ///
    /// - Perp: funding rate income (shorts receive when positive)
    /// - Spot: (0, 0) — no inherent yield, directional only
    /// - Lending: supply APY + reward APY
    /// - Vault: vault APY + reward APY
    fn alpha_stats(&self) -> Option<(f64, f64)> {
        None
    }

    /// Risk parameters for smooth Kelly. Returns None if the venue has no
    /// meaningful risk model (falls back to classic Kelly).
    fn risk_params(&self) -> Option<RiskParams> {
        None
    }

    /// Current margin ratio (equity / notional) for leveraged positions.
    /// Returns `None` for venues without margin (lending, spot, etc.).
    /// The optimizer uses this to detect perps approaching liquidation
    /// and pull capital from other venues to add margin.
    fn margin_ratio(&self) -> Option<f64> {
        None
    }

    /// Add margin to the venue (top up a perp position).
    /// Default no-op for venues that don't use margin.
    fn add_margin(&mut self, _amount: f64) {}
}

// ── Build mode ──────────────────────────────────────────────────────

/// How to construct venues — backtest (data-driven) or live (on-chain).
pub enum BuildMode<'a> {
    Backtest {
        manifest: &'a HashMap<NodeId, ManifestEntry>,
        data_dir: &'a Path,
        slippage_bps: f64,
        seed: u64,
    },
    Live {
        config: &'a RuntimeConfig,
        tokens: &'a evm::TokenManifest,
        contracts: &'a evm::ContractManifest,
    },
}

// ── VenueCategory trait ─────────────────────────────────────────────

/// Every venue category must implement this trait, bundling build + fetch.
///
/// This provides compile-time enforcement that every venue integration
/// has a data fetcher. Categories that don't need historical data
/// (movement, primitives) return `None` from `fetch_plan` and `fetch`.
pub trait VenueCategory {
    /// What data does this node need? Returns `None` if this category
    /// doesn't handle this node type.
    fn fetch_plan(node: &Node) -> Option<FetchJob>;

    /// Fetch data for a job. Returns `None` if this category doesn't
    /// handle this job's data source.
    fn fetch(
        client: &reqwest::Client,
        job: &FetchJob,
        config: &FetchConfig,
    ) -> impl std::future::Future<Output = Option<Result<FetchResult>>> + Send;

    /// Build a venue instance for a node. Returns `None` if this
    /// category doesn't handle this node type.
    fn build(node: &Node, mode: &BuildMode) -> Result<Option<Box<dyn Venue>>>;
}

// ── Category registration ───────────────────────────────────────────

/// Generates `build_all`, `fetch_plan_all`, and `dispatch_fetch` from
/// a single list of VenueCategory types. Adding a type here without
/// implementing VenueCategory causes a compile error.
macro_rules! register_venue_categories {
    ($($Cat:ty),* $(,)?) => {
        /// Build venues for all nodes in the workflow.
        /// Each node gets one venue instance. Optimizer nodes are skipped.
        pub fn build_all(
            workflow: &Workflow,
            mode: &BuildMode,
        ) -> Result<HashMap<NodeId, Box<dyn Venue>>> {
            let mut venues: HashMap<NodeId, Box<dyn Venue>> = HashMap::new();

            for node in &workflow.nodes {
                if matches!(node, Node::Optimizer { .. }) {
                    continue;
                }

                let id = node.id().to_string();
                let mut found = false;

                $(
                    if !found {
                        if let Some(venue) = <$Cat as VenueCategory>::build(node, mode)? {
                            venues.insert(id.clone(), venue);
                            found = true;
                        }
                    }
                )*

                if !found {
                    bail!(
                        "No venue builder matched node '{}' (type: {})",
                        id,
                        node.type_name()
                    );
                }
            }

            Ok(venues)
        }

        /// Scan a workflow and produce a deduplicated list of fetch jobs.
        /// Each category contributes via its `fetch_plan()`.
        pub fn fetch_plan_all(workflow: &Workflow) -> Vec<FetchJob> {
            let mut groups: HashMap<(DataSource, String), (Vec<String>, String, String)> =
                HashMap::new();

            for node in &workflow.nodes {
                let mut handled = false;

                $(
                    if !handled {
                        if let Some(job) = <$Cat as VenueCategory>::fetch_plan(node) {
                            let group_key = (job.source, job.key);
                            let entry = groups
                                .entry(group_key)
                                .or_insert_with(|| (Vec::new(), job.kind, job.filename));
                            entry.0.extend(job.node_ids);
                            handled = true;
                        }
                    }
                )*

                let _ = handled;
            }

            groups
                .into_iter()
                .map(|((source, key), (node_ids, kind, filename))| FetchJob {
                    node_ids,
                    source,
                    key,
                    kind,
                    filename,
                })
                .collect()
        }

        /// Dispatch a single fetch job to the appropriate venue category.
        pub async fn dispatch_fetch(
            client: &reqwest::Client,
            job: &FetchJob,
            config: &FetchConfig,
        ) -> Result<FetchResult> {
            $(
                if let Some(result) = <$Cat as VenueCategory>::fetch(client, job, config).await {
                    return result;
                }
            )*

            bail!(
                "No fetch handler matched job: source={:?}, key={}, kind={}",
                job.source,
                job.key,
                job.kind
            )
        }
    };
}

register_venue_categories!(
    perps::PerpsCategory,
    options::OptionsCategory,
    lending::LendingCategory,
    vault::VaultCategory,
    lp::LpCategory,
    movement::MovementCategory,
    yield_tokens::YieldTokensCategory,
    primitives::PrimitivesCategory,
);
