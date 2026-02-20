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

/// Movement type — what kind of token transfer operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MovementType {
    /// Same-chain token conversion (e.g. AERO → USDC on Base).
    Swap,
    /// Cross-chain same-token transfer (e.g. USDC Base → USDC HyperEVM).
    Bridge,
    /// Atomic cross-chain swap + bridge (e.g. AERO on Base → USDC on HyperEVM).
    SwapBridge,
}

/// Movement provider — which aggregator/protocol to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum MovementProvider {
    /// LiFi — supports swap, bridge, and swap+bridge atomically.
    LiFi,
    /// Stargate — bridge only.
    Stargate,
}

/// Vault protocol interface archetypes.
/// Determines which ABI / contract interaction pattern to use for live execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VaultArchetype {
    /// Morpho Vaults V2 (ERC4626-style).
    MorphoV2,
}

/// Vault protocol actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VaultAction {
    /// Deposit assets into the vault.
    Deposit,
    /// Withdraw assets from the vault.
    Withdraw,
    /// Claim reward emissions.
    ClaimRewards,
}

/// Lending protocol interface archetypes.
/// Determines which ABI / contract interaction pattern to use for live execution.
/// Aave forks (HyperLend, Lendle, Seamless, Granary, etc.) all use `AaveV3` or `AaveV2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LendingArchetype {
    /// Aave V3 Pool ABI (also used by HyperLend, Granary, Spark, Seamless, etc.)
    AaveV3,
    /// Aave V2 Pool ABI (Lendle, Geist, etc.)
    AaveV2,
    /// Morpho Blue.
    Morpho,
    /// Compound V3 (Comet).
    CompoundV3,
    /// Init Capital (ERC4626 vault-style).
    InitCapital,
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
        /// Token symbol (e.g. "USDC", "USDT0", "ETH").
        /// Must have a matching entry in the workflow `tokens` manifest.
        token: String,
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
        /// Margin / collateral token. Defaults to venue's native token
        /// (USDC for Hyperliquid, USDe for Hyena) if omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        margin_token: Option<String>,
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
    /// Token movement node — swap, bridge, or atomic swap+bridge.
    /// Unifies same-chain swaps, cross-chain bridges, and atomic cross-chain swaps.
    ///
    /// - `swap`: same-chain token conversion (e.g. AERO → USDC on Base).
    /// - `bridge`: cross-chain same-token transfer (e.g. USDC Base → USDC HyperEVM).
    /// - `swap_bridge`: atomic cross-chain swap+bridge via LiFi.
    Movement {
        /// Unique identifier for this node.
        id: NodeId,
        /// What kind of movement operation.
        movement_type: MovementType,
        /// Which aggregator / protocol to use.
        provider: MovementProvider,
        /// Source token symbol, e.g. "USDC".
        from_token: String,
        /// Destination token symbol. Same as `from_token` for bridge-only.
        to_token: String,
        /// Source chain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_chain: Option<Chain>,
        /// Destination chain. Required for bridge and swap_bridge.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_chain: Option<Chain>,
        /// Optional periodic trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Lending protocol node.
    /// Supply collateral, borrow, repay, withdraw, or claim rewards.
    /// The archetype determines which ABI to use; addresses are provided per-deployment.
    /// Any Aave fork works with `aave_v3` archetype + its pool address.
    Lending {
        /// Unique identifier for this node.
        id: NodeId,
        /// Which protocol interface to use (determines the ABI).
        archetype: LendingArchetype,
        /// The chain this lending deployment is on.
        chain: Chain,
        /// Contract manifest key for the pool (e.g. `hyperlend_pool`).
        /// Must have a matching entry in the workflow `contracts` manifest for this chain.
        pool_address: String,
        /// Asset token symbol, e.g. "USDC", "WETH", "USDe".
        asset: String,
        /// What action to perform.
        action: LendingAction,
        /// Contract manifest key for rewards controller (e.g. `hyperlend_rewards`).
        /// Must have a matching entry in the workflow `contracts` manifest for this chain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rewards_controller: Option<String>,
        /// DefiLlama project slug for fetch-data (e.g. "hyperlend-pooled", "aave-v3").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        defillama_slug: Option<String>,
        /// Optional periodic trigger (e.g. claim rewards daily).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<Trigger>,
    },
    /// Vault deposit node (e.g. Morpho Vaults V2).
    /// Deposit into yield-bearing vaults, withdraw, or claim rewards.
    /// The archetype determines which ABI to use; addresses are provided per-deployment.
    Vault {
        /// Unique identifier for this node.
        id: NodeId,
        /// Which vault interface to use (determines the ABI).
        archetype: VaultArchetype,
        /// The chain this vault deployment is on.
        chain: Chain,
        /// Contract manifest key for the vault (e.g. `morpho_usdc_vault`).
        /// Must have a matching entry in the workflow `contracts` manifest for this chain.
        vault_address: String,
        /// Asset token symbol, e.g. "USDC", "WETH".
        asset: String,
        /// What action to perform.
        action: VaultAction,
        /// DefiLlama project slug for fetch-data (e.g. "morpho-vaults-v2").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        defillama_slug: Option<String>,
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
            | Node::Movement { id, .. }
            | Node::Lending { id, .. }
            | Node::Vault { id, .. }
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
            Node::Movement { .. } => "movement",
            Node::Lending { .. } => "lending",
            Node::Vault { .. } => "vault",
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
            | Node::Movement { trigger, .. }
            | Node::Lending { trigger, .. }
            | Node::Vault { trigger, .. }
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
            Node::Wallet { token, chain, .. } => format!("wallet({}@{})", token, chain),
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
            Node::Movement {
                movement_type,
                provider,
                from_token,
                to_token,
                from_chain,
                to_chain,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                match movement_type {
                    MovementType::Swap => {
                        format!("movement(swap {provider:?} {from_token}->{to_token}{t})")
                    }
                    MovementType::Bridge => {
                        let fc = from_chain.as_ref().map(|c| c.to_string()).unwrap_or_default();
                        let tc = to_chain.as_ref().map(|c| c.to_string()).unwrap_or_default();
                        format!("movement(bridge {provider:?} {from_token} {fc}->{tc}{t})")
                    }
                    MovementType::SwapBridge => {
                        let fc = from_chain.as_ref().map(|c| c.to_string()).unwrap_or_default();
                        let tc = to_chain.as_ref().map(|c| c.to_string()).unwrap_or_default();
                        format!("movement(swap+bridge {provider:?} {from_token}->{to_token} {fc}->{tc}{t})")
                    }
                }
            }
            Node::Lending {
                archetype,
                chain,
                asset,
                action,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("lending({archetype:?} {action:?} {asset} on {chain}{t})")
            }
            Node::Vault {
                archetype,
                chain,
                asset,
                action,
                trigger,
                ..
            } => {
                let t = trig_suffix(trigger);
                format!("vault({archetype:?} {action:?} {asset} on {chain}{t})")
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

    /// The chain this node's output is on. `None` for chain-agnostic nodes (Optimizer, Swap without chain).
    /// For cross-chain Swaps (with `to_chain`), this returns `to_chain` (like Bridge).
    pub fn chain(&self) -> Option<Chain> {
        match self {
            Node::Wallet { chain, .. } => Some(chain.clone()),
            Node::Movement { to_chain, from_chain, .. } => {
                // Output chain: prefer to_chain, fallback to from_chain
                to_chain.clone().or_else(|| from_chain.clone())
            }
            Node::Perp { .. } => Some(Chain::hyperevm()),
            Node::Options { .. } => Some(Chain::hyperevm()),
            Node::Spot { .. } => Some(Chain::base()),
            Node::Lp { .. } => Some(Chain::base()),
            Node::Lending { chain, .. } => Some(chain.clone()),
            Node::Vault { chain, .. } => Some(chain.clone()),
            Node::Pendle { .. } => Some(Chain::hyperevm()),
            Node::Optimizer { .. } => None,
        }
    }

    /// The chain this node expects on its input side.
    /// For Bridge and cross-chain Swap nodes, this is the source chain.
    pub fn input_chain(&self) -> Option<Chain> {
        match self {
            Node::Movement { from_chain, .. } => from_chain.clone(),
            other => other.chain(),
        }
    }

    /// The effective margin / collateral token for Perp nodes.
    /// Returns `None` for non-Perp nodes.
    pub fn margin_token(&self) -> Option<&str> {
        match self {
            Node::Perp {
                venue,
                margin_token,
                ..
            } => Some(
                margin_token
                    .as_deref()
                    .unwrap_or(perp_venue_default_margin(venue)),
            ),
            _ => None,
        }
    }
}

/// Map a perp venue to its default margin / collateral token.
pub fn perp_venue_default_margin(venue: &PerpVenue) -> &'static str {
    match venue {
        PerpVenue::Hyperliquid => "USDC",
        PerpVenue::Hyena => "USDe",
    }
}

// ── Token flow for validation ──────────────────────────────────────

/// Describes a token on a specific chain, used for flow validation.
#[derive(Debug, Clone)]
pub struct TokenFlow {
    /// Token symbol (e.g. "USDC", "AERO", "ETH").
    pub token: String,
    /// Chain this token is on. `None` for chain-agnostic contexts.
    pub chain: Option<Chain>,
}

impl Node {
    /// What token (and on what chain) this node produces after execution.
    /// Returns `None` for position-update actions (no token leaves the venue)
    /// and for passthrough nodes (Wallet, Optimizer) where the edge token is used.
    pub fn output_token(&self) -> Option<TokenFlow> {
        match self {
            Node::Movement {
                to_token,
                to_chain,
                from_chain,
                ..
            } => Some(TokenFlow {
                token: to_token.clone(),
                chain: to_chain.clone().or_else(|| from_chain.clone()),
            }),
            Node::Perp {
                action,
                venue,
                margin_token,
                ..
            } => match action {
                PerpAction::Close | PerpAction::CollectFunding => {
                    let tok = margin_token
                        .as_deref()
                        .unwrap_or(perp_venue_default_margin(venue));
                    Some(TokenFlow {
                        token: tok.to_string(),
                        chain: Some(Chain::hyperevm()),
                    })
                }
                _ => None,
            },
            Node::Options { action, .. } => match action {
                OptionsAction::SellCoveredCall
                | OptionsAction::SellCashSecuredPut
                | OptionsAction::CollectPremium
                | OptionsAction::Close => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::hyperevm()),
                }),
                _ => None,
            },
            Node::Spot { pair, side, .. } => {
                let parts: Vec<&str> = pair.split('/').collect();
                if parts.len() == 2 {
                    let tok = match side {
                        SpotSide::Buy => parts[0],
                        SpotSide::Sell => parts[1],
                    };
                    Some(TokenFlow {
                        token: tok.to_string(),
                        chain: Some(Chain::base()),
                    })
                } else {
                    None
                }
            }
            Node::Lp { action, .. } => match action {
                LpAction::ClaimRewards => Some(TokenFlow {
                    token: "AERO".to_string(),
                    chain: Some(Chain::base()),
                }),
                LpAction::RemoveLiquidity => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::base()),
                }),
                _ => None,
            },
            Node::Lending {
                action,
                asset,
                chain,
                ..
            } => match action {
                LendingAction::Withdraw | LendingAction::Borrow => Some(TokenFlow {
                    token: asset.clone(),
                    chain: Some(chain.clone()),
                }),
                LendingAction::ClaimRewards => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(chain.clone()),
                }),
                _ => None,
            },
            Node::Vault {
                action,
                asset,
                chain,
                ..
            } => match action {
                VaultAction::Withdraw => Some(TokenFlow {
                    token: asset.clone(),
                    chain: Some(chain.clone()),
                }),
                VaultAction::ClaimRewards => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(chain.clone()),
                }),
                _ => None,
            },
            Node::Pendle { action, .. } => match action {
                PendleAction::RedeemPt
                | PendleAction::RedeemYt
                | PendleAction::ClaimRewards => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::hyperevm()),
                }),
                _ => None,
            },
            Node::Wallet { token, chain, .. } => Some(TokenFlow {
                token: token.clone(),
                chain: Some(chain.clone()),
            }),
            Node::Optimizer { .. } => None,
        }
    }

    /// What token (and on what chain) this node expects as input.
    /// Returns `None` for nodes that accept any token contextually
    /// or for actions that don't consume inbound tokens (Close, ClaimRewards, etc.).
    pub fn expected_input_token(&self) -> Option<TokenFlow> {
        match self {
            Node::Movement { from_token, from_chain, .. } => Some(TokenFlow {
                token: from_token.clone(),
                chain: from_chain.clone(),
            }),
            Node::Perp {
                venue,
                margin_token,
                action,
                ..
            } => match venue {
                // Hyperliquid USDC lives on HyperCore — skip token validation.
                PerpVenue::Hyperliquid => None,
                _ => match action {
                    PerpAction::Open | PerpAction::Adjust => {
                        let tok = margin_token
                            .as_deref()
                            .unwrap_or(perp_venue_default_margin(venue));
                        Some(TokenFlow {
                            token: tok.to_string(),
                            chain: Some(Chain::hyperevm()),
                        })
                    }
                    _ => None,
                },
            },
            Node::Lp { action, .. } => match action {
                LpAction::AddLiquidity => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::base()),
                }),
                _ => None,
            },
            Node::Lending {
                action,
                asset,
                chain,
                ..
            } => match action {
                LendingAction::Supply | LendingAction::Repay => Some(TokenFlow {
                    token: asset.clone(),
                    chain: Some(chain.clone()),
                }),
                _ => None,
            },
            Node::Vault {
                action,
                asset,
                chain,
                ..
            } => match action {
                VaultAction::Deposit => Some(TokenFlow {
                    token: asset.clone(),
                    chain: Some(chain.clone()),
                }),
                _ => None,
            },
            Node::Options { action, .. } => match action {
                OptionsAction::SellCashSecuredPut
                | OptionsAction::BuyCall
                | OptionsAction::BuyPut => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::hyperevm()),
                }),
                _ => None,
            },
            Node::Pendle { action, .. } => match action {
                PendleAction::MintPt | PendleAction::MintYt => Some(TokenFlow {
                    token: "USDC".to_string(),
                    chain: Some(Chain::hyperevm()),
                }),
                _ => None,
            },
            Node::Wallet { token, chain, .. } => Some(TokenFlow {
                token: token.clone(),
                chain: Some(chain.clone()),
            }),
            Node::Spot { .. } | Node::Optimizer { .. } => None,
        }
    }
}

