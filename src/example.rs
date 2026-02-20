use crate::model::amount::Amount;
use crate::model::chain::Chain;

use crate::model::edge::Edge;
use crate::model::node::*;
use crate::model::workflow::Workflow;

/// Print an example workflow JSON to stdout.
pub fn run() -> anyhow::Result<()> {
    let workflow = Workflow {
        name: "Kelly-Optimized Multi-Venue with Auto-Compound".to_string(),
        description: Some(
            "Bridge USDe from Mantle to HyperCore via Stargate, swap to USDC via LiFi, \
             then Kelly-optimize across: Hyperliquid ETH long perp, Hyena BTC short hedge, \
             Aerodrome cbBTC/WETH LP on Base, Rysk HYPE covered calls on HyperEVM, \
             HyperLend USDC supply, and Pendle PT-kHYPE fixed yield. \
             Periodic: rebalance daily (5% drift threshold), collect funding daily, \
             claim AERO rewards daily and compound back, sell covered calls weekly, \
             collect premium daily, claim lending rewards weekly."
                .to_string(),
        ),
        nodes: vec![
            // ── Deploy phase ────────────────────────────────────
            Node::Wallet {
                id: "wallet_src".into(),
                chain: Chain::mantle(),
                address: "0xYourWalletAddress".into(),
            },
            Node::Bridge {
                id: "bridge_hyper".into(),
                provider: BridgeProvider::Stargate,
                from_chain: Chain::mantle(),
                to_chain: Chain::hyperliquid(),
                token: "USDe".into(),
                trigger: None,
            },
            Node::Swap {
                id: "swap_usdc".into(),
                provider: SwapProvider::LiFi,
                from_token: "USDe".into(),
                to_token: "USDC".into(),
                chain: None,
                trigger: None,
            },
            Node::Optimizer {
                id: "kelly_opt".into(),
                strategy: OptimizerStrategy::Kelly,
                kelly_fraction: 0.5,
                max_allocation: Some(0.40),
                drift_threshold: 0.05,
                allocations: vec![
                    VenueAllocation {
                        target_node: "perp_eth_long".into(),
                        expected_return: 0.25,
                        volatility: 0.45,
                        correlation: 0.0,
                    },
                    VenueAllocation {
                        target_node: "swap_usde_hyena".into(),
                        expected_return: 0.10,
                        volatility: 0.35,
                        correlation: -0.2,
                    },
                    VenueAllocation {
                        target_node: "lp_aero".into(),
                        expected_return: 0.12,
                        volatility: 0.20,
                        correlation: 0.3,
                    },
                    VenueAllocation {
                        target_node: "options_hype_cc".into(),
                        expected_return: 0.15,
                        volatility: 0.50,
                        correlation: 0.1,
                    },
                    VenueAllocation {
                        target_node: "lend_usdc".into(),
                        expected_return: 0.06,
                        volatility: 0.02,
                        correlation: 0.0,
                    },
                    VenueAllocation {
                        target_node: "pendle_pt_khype".into(),
                        expected_return: 0.18,
                        volatility: 0.25,
                        correlation: 0.15,
                    },
                ],
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Daily,
                }),
            },
            Node::Perp {
                id: "perp_eth_long".into(),
                venue: PerpVenue::Hyperliquid,
                pair: "ETH/USDC".into(),
                action: PerpAction::Open,
                direction: Some(PerpDirection::Long),
                leverage: Some(3.0),
                margin_token: None,
                trigger: None,
            },
            // ── Swap USDC→USDe for Hyena margin ──────────────────
            Node::Swap {
                id: "swap_usde_hyena".into(),
                provider: SwapProvider::LiFi,
                from_token: "USDC".into(),
                to_token: "USDe".into(),
                chain: None,
                trigger: None,
            },
            Node::Perp {
                id: "perp_btc_short".into(),
                venue: PerpVenue::Hyena,
                pair: "BTC/USDe".into(),
                action: PerpAction::Open,
                direction: Some(PerpDirection::Short),
                leverage: Some(2.0),
                margin_token: None, // defaults to USDe for Hyena
                trigger: None,
            },
            Node::Lp {
                id: "lp_aero".into(),
                venue: LpVenue::Aerodrome,
                pool: "cbBTC/WETH".into(),
                action: LpAction::AddLiquidity,
                // Concentrated liquidity: ±5% around current price (CL100 pool)
                tick_lower: Some(-500),
                tick_upper: Some(500),
                tick_spacing: Some(100),
                trigger: None,
            },
            Node::Options {
                id: "options_hype_cc".into(),
                venue: OptionsVenue::Rysk,
                asset: RyskAsset::HYPE,
                action: OptionsAction::SellCoveredCall,
                delta_target: Some(0.3),
                days_to_expiry: Some(30),
                min_apy: Some(0.05),
                batch_size: Some(10),
                roll_days_before: Some(3),
                trigger: None,
            },
            // ── Deploy: lending supply on HyperLend ───────────────
            Node::Lending {
                id: "lend_usdc".into(),
                archetype: LendingArchetype::AaveV3,
                chain: Chain::hyperevm(),
                pool_address: "0xC0EE4e7e60D0A1F9a9AfaE0706D1b5C5A7f5B9b4".into(),
                asset: "USDC".into(),
                action: LendingAction::Supply,
                rewards_controller: Some("0x54586bE62E3c3580375aE3723C145253060Ca0C2".into()),
                defillama_slug: Some("hyperlend-pooled".into()),
                trigger: None,
            },
            // ── Deploy: Pendle PT-kHYPE for fixed yield ───────────
            Node::Pendle {
                id: "pendle_pt_khype".into(),
                market: "PT-kHYPE".into(),
                action: PendleAction::MintPt,
                trigger: None,
            },
            // ── Periodic: collect perp funding daily ────────────
            Node::Perp {
                id: "collect_eth_funding".into(),
                venue: PerpVenue::Hyperliquid,
                pair: "ETH/USDC".into(),
                action: PerpAction::CollectFunding,
                direction: None,
                leverage: None,
                margin_token: None,
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Daily,
                }),
            },
            // ── Periodic: claim AERO rewards & compound ─────────
            Node::Lp {
                id: "claim_aero".into(),
                venue: LpVenue::Aerodrome,
                pool: "cbBTC/WETH".into(),
                action: LpAction::ClaimRewards,
                tick_lower: None,
                tick_upper: None,
                tick_spacing: None,
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Daily,
                }),
            },
            Node::Swap {
                id: "swap_aero_usdc".into(),
                provider: SwapProvider::LiFi,
                from_token: "AERO".into(),
                to_token: "USDC".into(),
                chain: None,
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Daily,
                }),
            },
            // ── Periodic: collect Rysk premium daily ────────────
            Node::Options {
                id: "collect_premium".into(),
                venue: OptionsVenue::Rysk,
                asset: RyskAsset::HYPE,
                action: OptionsAction::CollectPremium,
                delta_target: None,
                days_to_expiry: None,
                min_apy: None,
                batch_size: None,
                roll_days_before: None,
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Daily,
                }),
            },
            // ── Periodic: roll expiring options ─────────────────
            Node::Options {
                id: "roll_options".into(),
                venue: OptionsVenue::Rysk,
                asset: RyskAsset::HYPE,
                action: OptionsAction::Roll,
                delta_target: Some(0.3),
                days_to_expiry: Some(30),
                min_apy: Some(0.05),
                batch_size: Some(10),
                roll_days_before: Some(3),
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Weekly,
                }),
            },
            // ── Periodic: claim HyperLend rewards weekly ─────────
            Node::Lending {
                id: "claim_lend_rewards".into(),
                archetype: LendingArchetype::AaveV3,
                chain: Chain::hyperevm(),
                pool_address: "0xC0EE4e7e60D0A1F9a9AfaE0706D1b5C5A7f5B9b4".into(),
                asset: "USDC".into(),
                action: LendingAction::ClaimRewards,
                rewards_controller: Some("0x54586bE62E3c3580375aE3723C145253060Ca0C2".into()),
                defillama_slug: Some("hyperlend-pooled".into()),
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Weekly,
                }),
            },
        ],
        edges: vec![
            // ── Deploy edges (acyclic) ──────────────────────────
            Edge {
                from_node: "wallet_src".into(),
                to_node: "bridge_hyper".into(),
                token: "USDe".into(),
                amount: Amount::Fixed {
                    value: "50000".into(),
                },
            },
            Edge {
                from_node: "bridge_hyper".into(),
                to_node: "swap_usdc".into(),
                token: "USDe".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "swap_usdc".into(),
                to_node: "kelly_opt".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "perp_eth_long".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "swap_usde_hyena".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "swap_usde_hyena".into(),
                to_node: "perp_btc_short".into(),
                token: "USDe".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "lp_aero".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "options_hype_cc".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "lend_usdc".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly_opt".into(),
                to_node: "pendle_pt_khype".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            // ── Periodic edges (cycles allowed) ─────────────────
            // Collected funding -> back to optimizer
            Edge {
                from_node: "collect_eth_funding".into(),
                to_node: "kelly_opt".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            // Claimed AERO rewards -> swap -> back to optimizer
            Edge {
                from_node: "claim_aero".into(),
                to_node: "swap_aero_usdc".into(),
                token: "AERO".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "swap_aero_usdc".into(),
                to_node: "kelly_opt".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            // Collected premium -> back to optimizer
            Edge {
                from_node: "collect_premium".into(),
                to_node: "kelly_opt".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            // Claimed lending rewards -> back to optimizer
            Edge {
                from_node: "claim_lend_rewards".into(),
                to_node: "kelly_opt".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
        ],
    };

    let json = serde_json::to_string_pretty(&workflow)?;
    println!("{json}");
    Ok(())
}
