use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::chain::Chain;

/// A unique identifier for a node within a workflow.
pub type NodeId = String;

// ── Venue / Provider enums ──────────────────────────────────────────

/// Perpetual futures trading venues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PerpVenue {
    Hyperliquid,
    /// Hyena (HIP-3 perps on Hyperliquid).
    Hyena,
}

/// Options trading venues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OptionsVenue {
    /// Rysk options protocol on HyperEVM.
    Rysk,
}

/// Spot / DEX trading venues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SpotVenue {
    Aerodrome,
}

/// Liquidity provision venues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LpVenue {
    /// Aerodrome (Base) — concentrated liquidity via Slipstream.
    Aerodrome,
}

/// Swap aggregator providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SwapProvider {
    LiFi,
}

/// Cross-chain bridge providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BridgeProvider {
    LiFi,
    Stargate,
}

/// Lending protocol venues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LendingVenue {
    /// Aave V3.
    Aave,
    /// Lendle — Aave V2 fork on Mantle.
    Lendle,
    /// Morpho Blue — modular lending with isolated markets.
    Morpho,
    /// Compound V3 (Comet) — single-asset markets.
    Compound,
    /// Init Capital — isolated lending pools (ERC4626 vault-style).
    InitCapital,
    /// HyperLend — Aave V3 fork on HyperEVM.
    HyperLend,
}

/// Lending protocol actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LendingAction {
    /// Supply / deposit assets as collateral or lendable liquidity.
    Supply,
    /// Withdraw previously supplied assets.
    Withdraw,
    /// Borrow assets against supplied collateral.
    Borrow,
    /// Repay outstanding borrows.
    Repay,
    /// Claim protocol reward emissions.
    ClaimRewards,
}

/// Pendle yield tokenization actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PendleAction {
    /// Mint principal token (PT) — lock underlying for fixed yield until maturity.
    MintPt,
    /// Redeem PT back to underlying at or after maturity.
    RedeemPt,
    /// Mint yield token (YT) — receive variable yield stream.
    MintYt,
    /// Redeem YT — exit variable yield position.
    RedeemYt,
    /// Claim accumulated Pendle rewards.
    ClaimRewards,
}

// ── Direction / Side / Action enums ─────────────────────────────────

/// Trade direction for perpetual futures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PerpDirection {
    Long,
    Short,
}

/// Trade side for spot markets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpotSide {
    Buy,
    Sell,
}

/// Action for perpetual futures — venue-specific lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PerpAction {
    /// Open a new position (requires direction + leverage).
    Open,
    /// Close an existing position.
    Close,
    /// Adjust position size or leverage.
    Adjust,
    /// Collect accumulated funding payments (periodic).
    CollectFunding,
}

/// Action for options — Rysk lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionsAction {
    /// Sell a covered call — hold underlying, sell calls against it.
    /// Collateral: underlying asset. Premium received in USD.
    SellCoveredCall,
    /// Sell a cash-secured put — hold USD, sell puts.
    /// Collateral: USD stablecoin. Premium received in USD.
    SellCashSecuredPut,
    /// Buy a call option.
    BuyCall,
    /// Buy a put option.
    BuyPut,
    /// Collect settled premium from expired options (periodic).
    CollectPremium,
    /// Roll expiring positions — close near-expiry + open new at next expiry.
    Roll,
    /// Close/exercise an existing position.
    Close,
}

/// Supported underlying assets for Rysk options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RyskAsset {
    ETH,
    BTC,
    HYPE,
    SOL,
}

/// Action for liquidity provision — venue-specific lifecycle.
/// Aerodrome: add/remove liquidity, gauge staking, reward claiming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LpAction {
    AddLiquidity,
    RemoveLiquidity,
    /// Claim gauge reward emissions (e.g. AERO on Aerodrome).
    ClaimRewards,
    /// Reinvest claimed rewards back into the pool.
    Compound,
    /// Stake LP tokens into gauge for reward emissions.
    StakeGauge,
    /// Unstake LP tokens from gauge.
    UnstakeGauge,
}

// ── Trigger types ───────────────────────────────────────────────────

/// Cron interval for periodic triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CronInterval {
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

/// Trigger that makes a node execute periodically rather than once.
/// When a node has a trigger, it runs on the specified schedule.
/// Its outgoing edges define where periodic outputs (e.g. claimed rewards) flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Run on a fixed cron schedule.
    Cron { interval: CronInterval },
    /// Run when an external event fires (e.g. price threshold, health factor).
    OnEvent { event: String },
}

// ── Optimizer types ─────────────────────────────────────────────────

/// Optimization strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OptimizerStrategy {
    /// Kelly Criterion for optimal bet sizing / capital allocation.
    Kelly,
}

/// Per-venue allocation parameters for the optimizer.
/// The optimizer uses these to compute optimal capital splits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VenueAllocation {
    /// The node ID of the downstream venue to allocate capital to.
    pub target_node: NodeId,
    /// Expected annualized return (e.g. 0.15 = 15%).
    pub expected_return: f64,
    /// Annualized volatility / standard deviation (e.g. 0.30 = 30%).
    pub volatility: f64,
    /// Correlation with a reference asset. Defaults to 0.0 if omitted.
    #[serde(default)]
    pub correlation: f64,
}

// ── The main Node enum ──────────────────────────────────────────────

/// A workflow node. Discriminated by the "type" field in JSON.
/// Each variant carries a unique id and type-specific parameters.
///
/// Venue nodes can optionally have a `trigger` to make them execute
/// periodically (e.g. claim rewards every 24h, rebalance daily).
/// Triggered nodes are allowed to form cycles in the graph since they
/// represent periodic re-entry flows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    /// Wallet node: source or sink for funds on a specific chain.
    Wallet {
        /// Unique identifier for this node.
        id: NodeId,
        /// The chain this wallet resides on.
        chain: Chain,
        /// Wallet address (0x-prefixed hex).
        address: String,
    },
    /// Perpetual futures trading node.
    /// For `open`/`adjust` actions, `direction` and `leverage` are required.
    /// For `close`/`collect_funding`, they can be omitted.
    Perp {
        /// Unique identifier for this node.
        id: NodeId,
        /// Trading venue.
        venue: PerpVenue,
        /// Trading pair, e.g. "ETH/USDC".
        pair: String,
        /// What action to perform on this venue.
        action: PerpAction,
        /// Trade direction (required for open/adjust).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<PerpDirection>,
        /// Leverage multiplier (required for open/adjust, e.g. 5.0 for 5x).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leverage: Option<f64>,
        /// Optional periodic trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Options trading node (Rysk on HyperEVM).
    /// The LLM specifies strategy parameters (delta target, days to expiry, min APY).
    /// The execution layer uses the Rysk RFQ flow to pick actual strike/expiry.
    ///
    /// Covered calls: collateral = underlying asset, premium in USD.
    /// Cash-secured puts: collateral = USD stablecoin, premium in USD.
    Options {
        /// Unique identifier for this node.
        id: NodeId,
        /// Options venue.
        venue: OptionsVenue,
        /// Underlying asset to trade options on.
        asset: RyskAsset,
        /// What action to perform.
        action: OptionsAction,
        /// Target delta for option selection (0.0 - 1.0, e.g. 0.3 = 30-delta).
        /// Used by the selector to pick strikes. Only for sell/buy actions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta_target: Option<f64>,
        /// Target days to expiry (e.g. 30 for monthly).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        days_to_expiry: Option<u32>,
        /// Minimum acceptable APY (e.g. 0.05 = 5%). Skip options below this.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_apy: Option<f64>,
        /// Batch size in tokens for Rysk RFQ (e.g. 10 = 10 token increments).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_size: Option<u32>,
        /// Days before expiry to roll positions (e.g. 3 = roll 3 days early).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roll_days_before: Option<u32>,
        /// Optional periodic trigger (e.g. deploy weekly, collect premium daily, roll).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Spot trading node.
    Spot {
        /// Unique identifier for this node.
        id: NodeId,
        /// DEX venue.
        venue: SpotVenue,
        /// Trading pair, e.g. "ETH/USDC".
        pair: String,
        /// Buy or sell.
        side: SpotSide,
        /// Optional periodic trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Liquidity provision node.
    /// Supports full Aerodrome Slipstream lifecycle: concentrated liquidity with
    /// tick ranges, gauge staking for AERO rewards, and compounding.
    ///
    /// Aerodrome Slipstream uses Uniswap V3-style concentrated liquidity with NFT positions.
    /// The `tick_lower` / `tick_upper` define the price range for the position.
    /// Tighter ranges earn more fees (concentration multiplier) but risk going out of range.
    Lp {
        /// Unique identifier for this node.
        id: NodeId,
        /// LP venue.
        venue: LpVenue,
        /// Pool identifier, e.g. "cbBTC/WETH".
        pool: String,
        /// What action to perform.
        action: LpAction,
        /// Lower tick bound for concentrated liquidity (Aerodrome Slipstream).
        /// Omit for full-range positions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tick_lower: Option<i32>,
        /// Upper tick bound for concentrated liquidity.
        /// Omit for full-range positions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tick_upper: Option<i32>,
        /// Tick spacing of the pool (e.g. 100 for Aerodrome CL100, 200 for CL200).
        /// Used to snap tick bounds to valid values.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tick_spacing: Option<i32>,
        /// Optional periodic trigger (e.g. claim_rewards every day).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Token swap via aggregator node.
    Swap {
        /// Unique identifier for this node.
        id: NodeId,
        /// Aggregator provider.
        provider: SwapProvider,
        /// Source token symbol, e.g. "USDC".
        from_token: String,
        /// Destination token symbol, e.g. "ETH".
        to_token: String,
        /// Optional periodic trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Cross-chain bridge node.
    Bridge {
        /// Unique identifier for this node.
        id: NodeId,
        /// Bridge provider.
        provider: BridgeProvider,
        /// Source chain.
        from_chain: Chain,
        /// Destination chain.
        to_chain: Chain,
        /// Token to bridge, e.g. "USDC".
        token: String,
        /// Optional periodic trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Lending protocol node.
    /// Supply collateral, borrow, repay, withdraw, or claim rewards.
    /// The execution layer handles protocol-specific details (e.g. Morpho market IDs,
    /// HyperLend E-mode, Init Capital vault shares).
    Lending {
        /// Unique identifier for this node.
        id: NodeId,
        /// Lending protocol venue.
        venue: LendingVenue,
        /// Asset token symbol, e.g. "USDC", "WETH", "USDe".
        asset: String,
        /// What action to perform.
        action: LendingAction,
        /// Optional periodic trigger (e.g. claim rewards daily).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Pendle yield tokenization node.
    /// Mint/redeem principal tokens (PT) for fixed yield or yield tokens (YT)
    /// for variable yield. Used in strategies like PT-kHYPE looping.
    /// The execution layer handles Pendle router interactions and market lookups.
    Pendle {
        /// Unique identifier for this node.
        id: NodeId,
        /// Pendle market identifier, e.g. "PT-kHYPE", "PT-stETH", "PT-eETH".
        market: String,
        /// What action to perform.
        action: PendleAction,
        /// Optional periodic trigger (e.g. claim rewards weekly).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Capital allocation optimizer node (e.g. Kelly Criterion).
    /// Receives capital from upstream and distributes it across N downstream
    /// venue nodes using optimal sizing.
    ///
    /// With a `trigger`, the optimizer periodically checks current allocations
    /// against targets and rebalances if drift exceeds `drift_threshold`.
    Optimizer {
        /// Unique identifier for this node.
        id: NodeId,
        /// Optimization strategy to use.
        strategy: OptimizerStrategy,
        /// Fraction of Kelly to apply (0.0 - 1.0). 0.5 = half-Kelly (smoother).
        kelly_fraction: f64,
        /// Optional maximum allocation to any single venue (0.0 - 1.0).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_allocation: Option<f64>,
        /// Minimum allocation drift before triggering a rebalance (0.0 - 1.0).
        /// E.g. 0.05 = rebalance if any venue drifts >5% from target.
        /// Defaults to 0.0 (always rebalance on trigger).
        #[serde(default)]
        drift_threshold: f64,
        /// Per-venue allocation parameters. Each entry maps to a downstream node.
        allocations: Vec<VenueAllocation>,
        /// Periodic trigger for rebalance checks.
        /// When triggered, reads current positions, computes Kelly-optimal allocations,
        /// and rebalances if drift exceeds threshold.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
}

impl Node {
    /// Extract the node ID regardless of variant.
    pub fn id(&self) -> &str {
        match self {
            Node::Wallet { id, .. }
            | Node::Perp { id, .. }
            | Node::Options { id, .. }
            | Node::Spot { id, .. }
            | Node::Lp { id, .. }
            | Node::Swap { id, .. }
            | Node::Bridge { id, .. }
            | Node::Lending { id, .. }
            | Node::Pendle { id, .. }
            | Node::Optimizer { id, .. } => id,
        }
    }

    /// Return a human-readable type name for this node.
    pub fn type_name(&self) -> &'static str {
        match self {
            Node::Wallet { .. } => "wallet",
            Node::Perp { .. } => "perp",
            Node::Options { .. } => "options",
            Node::Spot { .. } => "spot",
            Node::Lp { .. } => "lp",
            Node::Swap { .. } => "swap",
            Node::Bridge { .. } => "bridge",
            Node::Lending { .. } => "lending",
            Node::Pendle { .. } => "pendle",
            Node::Optimizer { .. } => "optimizer",
        }
    }

    /// Whether this node has a periodic trigger.
    pub fn is_triggered(&self) -> bool {
        match self {
            Node::Perp { trigger, .. }
            | Node::Options { trigger, .. }
            | Node::Spot { trigger, .. }
            | Node::Lp { trigger, .. }
            | Node::Swap { trigger, .. }
            | Node::Bridge { trigger, .. }
            | Node::Lending { trigger, .. }
            | Node::Pendle { trigger, .. }
            | Node::Optimizer { trigger, .. } => trigger.is_some(),
            Node::Wallet { .. } => false,
        }
    }

    /// Short label for display (type + key info).
    pub fn label(&self) -> String {
        let trig_suffix = |trigger: &Option<Trigger>| -> &str {
            if trigger.is_some() { " [cron]" } else { "" }
        };

        match self {
            Node::Wallet { chain, .. } => format!("wallet({})", chain),
            Node::Perp {
                venue,
                pair,
                action,
                direction,
                leverage,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                match action {
                    PerpAction::Open | PerpAction::Adjust => {
                        let dir = direction.map(|d| format!("{d:?}")).unwrap_or_default();
                        let lev = leverage.map(|l| format!(" {l}x")).unwrap_or_default();
                        format!("perp({venue:?} {action:?} {dir} {pair}{lev}{t})")
                    }
                    _ => format!("perp({venue:?} {action:?} {pair}{t})"),
                }
            }
            Node::Options {
                venue,
                asset,
                action,
                delta_target,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                let delta = delta_target
                    .map(|d| format!(" {:.0}d", d * 100.0))
                    .unwrap_or_default();
                format!("options({venue:?} {action:?} {asset:?}{delta}{t})")
            }
            Node::Spot {
                venue,
                pair,
                side,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("spot({venue:?} {side:?} {pair}{t})")
            }
            Node::Lp {
                venue,
                pool,
                action,
                tick_lower,
                tick_upper,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                let ticks = match (tick_lower, tick_upper) {
                    (Some(lo), Some(hi)) => format!(" [{lo},{hi}]"),
                    _ => String::new(),
                };
                format!("lp({venue:?} {action:?} {pool}{ticks}{t})")
            }
            Node::Swap {
                provider,
                from_token,
                to_token,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("swap({provider:?} {from_token}->{to_token}{t})")
            }
            Node::Bridge {
                provider,
                from_chain,
                to_chain,
                token,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("bridge({provider:?} {token} {from_chain}->{to_chain}{t})")
            }
            Node::Lending {
                venue,
                asset,
                action,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("lending({venue:?} {action:?} {asset}{t})")
            }
            Node::Pendle {
                market,
                action,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("pendle({action:?} {market}{t})")
            }
            Node::Optimizer {
                strategy,
                kelly_fraction,
                allocations,
                trigger,
                drift_threshold,
                ..
            } => {
                let t = trig_suffix(trigger);
                let drift = if *drift_threshold > 0.0 {
                    format!(" drift>{:.0}%", drift_threshold * 100.0)
                } else {
                    String::new()
                };
                format!(
                    "optimizer({strategy:?} {:.0}% kelly, {} venues{drift}{t})",
                    kelly_fraction * 100.0,
                    allocations.len()
                )
            }
        }
    }
}
