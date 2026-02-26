use crate::model::amount::Amount;
use crate::model::chain::Chain;

use crate::model::edge::Edge;
use crate::model::node::*;
use crate::model::workflow::Workflow;

/// Print an example workflow JSON to stdout.
pub fn run() -> anyhow::Result<()> {
    let workflow = Workflow {
        name: "Kelly-Optimized Multi-Venue with Auto-Compound".to_string(),
        tokens: None,
        reserve: None,
        contracts: Some({
            let mut c = std::collections::HashMap::new();
            // Lending contracts
            c.insert("hyperlend_pool".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0xC0EE4e7e60D0A1F9a9AfaE0706D1b5C5A7f5B9b4".to_string());
                m
            });
            c.insert("hyperlend_rewards".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0x54586bE62E3c3580375aE3723C145253060Ca0C2".to_string());
                m
            });
            // Pendle
            c.insert("pendle_router".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0x00000000005BBB0EF59571E58418F9a4357b68A0".to_string());
                m
            });
            c.insert("pendle_pt_khype_market".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0x0000000000000000000000000000000000000001".to_string());
                m
            });
            c.insert("pendle_pt_khype_sy".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0x0000000000000000000000000000000000000002".to_string());
                m
            });
            c.insert("pendle_pt_khype_yt".to_string(), {
                let mut m = std::collections::HashMap::new();
                m.insert("hyperevm".to_string(), "0x0000000000000000000000000000000000000003".to_string());
                m
            });
            c
        }),
        description: Some(
            "Bridge USDe from Mantle to HyperCore via Stargate, swap to USDC via LiFi, \
             then Kelly-optimize across: Hyperliquid ETH long perp, Hyena BTC short hedge, \
             Rysk HYPE covered calls on HyperEVM, \
             HyperLend USDC supply, and Pendle PT-kHYPE fixed yield. \
             Periodic: rebalance daily (5% drift threshold), collect funding daily, \
             sell covered calls weekly, \
             collect premium daily, claim lending rewards weekly."
                .to_string(),
        ),
        nodes: vec![
            // ── Deploy phase ────────────────────────────────────
            Node::Wallet {
                id: "wallet_src".into(),
                chain: Chain::mantle(),
                token: "USDe".into(),
                address: "0xYourWalletAddress".into(),
            },
            Node::Movement {
                id: "bridge_hyper".into(),
                movement_type: MovementType::Bridge,
                provider: MovementProvider::LiFi,
                from_token: "USDe".into(),
                to_token: "USDe".into(),
                from_chain: Some(Chain::mantle()),
                to_chain: Some(Chain::hyperliquid()),
                trigger: None,
            },
            Node::Movement {
                id: "swap_usdc".into(),
                movement_type: MovementType::Swap,
                provider: MovementProvider::LiFi,
                from_token: "USDe".into(),
                to_token: "USDC".into(),
                from_chain: None,
                to_chain: None,
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
                        target_nodes: vec![],
                        target_node: Some("perp_eth_long".into()),
                        expected_return: None,
                        volatility: None,
                        correlation: 0.0,
                    },
                    VenueAllocation {
                        target_nodes: vec![],
                        target_node: Some("swap_usde_hyena".into()),
                        expected_return: None,
                        volatility: None,
                        correlation: -0.2,
                    },
                    VenueAllocation {
                        target_nodes: vec![],
                        target_node: Some("options_hype_cc".into()),
                        expected_return: None,
                        volatility: None,
                        correlation: 0.1,
                    },
                    VenueAllocation {
                        target_nodes: vec![],
                        target_node: Some("lend_usdc".into()),
                        expected_return: None,
                        volatility: None,
                        correlation: 0.0,
                    },
                    VenueAllocation {
                        target_nodes: vec![],
                        target_node: Some("pendle_pt_khype".into()),
                        expected_return: None,
                        volatility: None,
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
            Node::Movement {
                id: "swap_usde_hyena".into(),
                movement_type: MovementType::Swap,
                provider: MovementProvider::LiFi,
                from_token: "USDC".into(),
                to_token: "USDe".into(),
                from_chain: None,
                to_chain: None,
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
                pool_address: "hyperlend_pool".into(),
                asset: "USDC".into(),
                action: LendingAction::Supply,
                rewards_controller: Some("hyperlend_rewards".into()),
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
                pool_address: "hyperlend_pool".into(),
                asset: "USDC".into(),
                action: LendingAction::ClaimRewards,
                rewards_controller: Some("hyperlend_rewards".into()),
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
