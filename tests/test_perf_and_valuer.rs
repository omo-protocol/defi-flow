use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use defi_flow::engine::Engine;
use defi_flow::model::amount::Amount;
use defi_flow::model::chain::Chain;
use defi_flow::model::edge::Edge;
use defi_flow::model::node::*;
use defi_flow::model::valuer::ValuerConfig;
use defi_flow::model::workflow::Workflow;
use defi_flow::run::state::RunState;
use defi_flow::run::valuer;
use defi_flow::validate;
use defi_flow::venues::{ExecutionResult, SimMetrics, Venue};

// ── Mock venues ─────────────────────────────────────────────────────

/// Mock venue with controllable metrics for testing performance tracking.
struct MockPerpVenue {
    value: f64,
    funding: f64,
    swap_cost: f64,
}

impl MockPerpVenue {
    fn new(value: f64) -> Self {
        Self {
            value,
            funding: 0.0,
            swap_cost: 0.0,
        }
    }

    fn with_metrics(value: f64, funding: f64, swap_cost: f64) -> Self {
        Self {
            value,
            funding,
            swap_cost,
        }
    }
}

#[async_trait]
impl Venue for MockPerpVenue {
    async fn execute(&mut self, _node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        self.value += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.value)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        // Simulate funding accrual
        self.value += self.funding;
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let f = fraction.clamp(0.0, 1.0);
        let freed = self.value * f;
        self.value -= freed;
        Ok(freed)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            funding_pnl: self.funding,
            swap_costs: self.swap_cost,
            ..Default::default()
        }
    }
}

/// Mock lending venue.
struct MockLendingVenue {
    value: f64,
    interest: f64,
    rewards: f64,
}

impl MockLendingVenue {
    fn new(value: f64) -> Self {
        Self {
            value,
            interest: 0.0,
            rewards: 0.0,
        }
    }

    fn with_metrics(value: f64, interest: f64, rewards: f64) -> Self {
        Self {
            value,
            interest,
            rewards,
        }
    }
}

#[async_trait]
impl Venue for MockLendingVenue {
    async fn execute(&mut self, _node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        self.value += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.value)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        self.value += self.interest;
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let f = fraction.clamp(0.0, 1.0);
        let freed = self.value * f;
        self.value -= freed;
        Ok(freed)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lending_interest: self.interest,
            rewards_pnl: self.rewards,
            ..Default::default()
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn simple_lending_workflow() -> Workflow {
    Workflow {
        name: "Test Lending".into(),
        description: None,
        tokens: None,
        contracts: None,
        reserve: None,
        valuer: None,
        nodes: vec![
            Node::Wallet {
                id: "wallet".into(),
                chain: Chain::hyperevm(),
                token: "USDC".into(),
                address: "0x1234567890123456789012345678901234567890".into(),
            },
            Node::Lending {
                id: "lend_usdc".into(),
                archetype: LendingArchetype::AaveV3,
                chain: Chain::hyperevm(),
                pool_address: "pool".into(),
                asset: "USDC".into(),
                action: LendingAction::Supply,
                rewards_controller: None,
                defillama_slug: None,
                trigger: None,
            },
        ],
        edges: vec![Edge {
            from_node: "wallet".into(),
            to_node: "lend_usdc".into(),
            token: "USDC".into(),
            amount: Amount::All,
        }],
    }
}

fn dn_workflow() -> Workflow {
    Workflow {
        name: "Test DN".into(),
        description: None,
        tokens: None,
        contracts: None,
        reserve: None,
        valuer: None,
        nodes: vec![
            Node::Wallet {
                id: "wallet".into(),
                chain: Chain::hyperevm(),
                token: "USDC".into(),
                address: "0x1234567890123456789012345678901234567890".into(),
            },
            Node::Optimizer {
                id: "kelly".into(),
                strategy: OptimizerStrategy::Kelly,
                kelly_fraction: 0.5,
                max_allocation: None,
                drift_threshold: 0.05,
                allocations: vec![
                    VenueAllocation {
                        target_nodes: vec!["buy_eth".into(), "short_eth".into()],
                        target_node: None,
                        expected_return: None,
                        volatility: None,
                        correlation: 0.0,
                    },
                    VenueAllocation {
                        target_nodes: vec![],
                        target_node: Some("lend_usdc".into()),
                        expected_return: None,
                        volatility: None,
                        correlation: 0.0,
                    },
                ],
                trigger: Some(Trigger::Cron {
                    interval: CronInterval::Weekly,
                }),
            },
            Node::Spot {
                id: "buy_eth".into(),
                venue: SpotVenue::Hyperliquid,
                pair: "ETH/USDC".into(),
                side: SpotSide::Buy,
                trigger: None,
            },
            Node::Perp {
                id: "short_eth".into(),
                venue: PerpVenue::Hyperliquid,
                pair: "ETH/USDC".into(),
                action: PerpAction::Open,
                direction: Some(PerpDirection::Short),
                leverage: Some(1.0),
                margin_token: None,
                trigger: None,
            },
            Node::Lending {
                id: "lend_usdc".into(),
                archetype: LendingArchetype::AaveV3,
                chain: Chain::hyperevm(),
                pool_address: "pool".into(),
                asset: "USDC".into(),
                action: LendingAction::Supply,
                rewards_controller: None,
                defillama_slug: None,
                trigger: None,
            },
        ],
        edges: vec![
            Edge {
                from_node: "wallet".into(),
                to_node: "kelly".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly".into(),
                to_node: "buy_eth".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly".into(),
                to_node: "short_eth".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
            Edge {
                from_node: "kelly".into(),
                to_node: "lend_usdc".into(),
                token: "USDC".into(),
                amount: Amount::All,
            },
        ],
    }
}

fn sync_balances(engine: &Engine, state: &mut RunState) {
    state.balances.clear();
    for node in &engine.workflow.nodes {
        let id = node.id().to_string();
        let total = engine.balances.node_total(&id);
        if total > 0.0 {
            state.balances.insert(id, total);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// RunState Serialization Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_state_roundtrip_with_perf_fields() {
    let state = RunState {
        deploy_completed: true,
        last_tick: 1000,
        balances: {
            let mut m = HashMap::new();
            m.insert("lend_usdc".into(), 50000.0);
            m
        },
        reserve_actions: vec![],
        initial_capital: 48000.0,
        peak_tvl: 52000.0,
        cumulative_funding: 150.0,
        cumulative_interest: 320.5,
        cumulative_rewards: 42.0,
        cumulative_costs: 12.3,
    };

    let json = serde_json::to_string_pretty(&state).unwrap();
    let loaded: RunState = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded.initial_capital, 48000.0);
    assert_eq!(loaded.peak_tvl, 52000.0);
    assert_eq!(loaded.cumulative_funding, 150.0);
    assert_eq!(loaded.cumulative_interest, 320.5);
    assert_eq!(loaded.cumulative_rewards, 42.0);
    assert_eq!(loaded.cumulative_costs, 12.3);
}

#[test]
fn test_state_backward_compat_old_format() {
    // Old state files don't have performance fields — serde(default) fills them with 0.0
    let old_json = r#"{
        "deploy_completed": true,
        "last_tick": 500,
        "balances": { "lend_usdc": 10000.0 }
    }"#;

    let state: RunState = serde_json::from_str(old_json).unwrap();
    assert!(state.deploy_completed);
    assert_eq!(state.last_tick, 500);
    assert_eq!(state.initial_capital, 0.0);
    assert_eq!(state.peak_tvl, 0.0);
    assert_eq!(state.cumulative_funding, 0.0);
    assert_eq!(state.cumulative_interest, 0.0);
    assert_eq!(state.cumulative_rewards, 0.0);
    assert_eq!(state.cumulative_costs, 0.0);
}

#[test]
fn test_state_file_persistence() {
    let path = std::env::temp_dir().join("defi_flow_test_state.json");

    let state = RunState {
        deploy_completed: true,
        last_tick: 1234,
        initial_capital: 100_000.0,
        peak_tvl: 105_000.0,
        cumulative_funding: 500.0,
        cumulative_interest: 800.0,
        cumulative_rewards: 100.0,
        cumulative_costs: 50.0,
        ..Default::default()
    };

    state.save(&path).unwrap();
    let loaded = RunState::load_or_new(&path).unwrap();

    // Clean up
    let _ = std::fs::remove_file(&path);

    assert_eq!(loaded.initial_capital, 100_000.0);
    assert_eq!(loaded.peak_tvl, 105_000.0);
    assert_eq!(loaded.cumulative_funding, 500.0);
    assert_eq!(loaded.cumulative_costs, 50.0);
}

// ═══════════════════════════════════════════════════════════════════
// Performance Tracking — Initial Capital
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_initial_capital_set_after_deploy() {
    let wf = simple_lending_workflow();
    let venues: HashMap<String, Box<dyn Venue>> = {
        let mut m = HashMap::new();
        m.insert(
            "lend_usdc".to_string(),
            Box::new(MockLendingVenue::new(0.0)) as Box<dyn Venue>,
        );
        m
    };
    let mut engine = Engine::new(wf, venues);
    let mut state = RunState::default();

    // After deploy: 50000 USDC is now in the lending venue
    engine.balances.add("lend_usdc", "USDC", 50000.0);
    sync_balances(&engine, &mut state);

    // Record initial capital
    state.initial_capital = state.balances.values().sum();
    state.peak_tvl = state.initial_capital;

    assert_eq!(state.initial_capital, 50000.0);
    assert_eq!(state.peak_tvl, 50000.0);
}

#[tokio::test]
async fn test_initial_capital_backfill_old_state() {
    // Simulates loading an old state file where deploy_completed=true but
    // initial_capital=0.0 (pre-perf-tracking state)
    let mut state = RunState {
        deploy_completed: true,
        last_tick: 100,
        balances: {
            let mut m = HashMap::new();
            m.insert("lend_usdc".into(), 45000.0);
            m.insert("short_eth".into(), 5000.0);
            m
        },
        initial_capital: 0.0, // Not yet tracked
        ..Default::default()
    };

    // Backfill logic (same as run_async):
    if state.initial_capital == 0.0 {
        state.initial_capital = state.balances.values().sum();
        state.peak_tvl = state.initial_capital;
    }

    assert_eq!(state.initial_capital, 50000.0);
    assert_eq!(state.peak_tvl, 50000.0);
}

// ═══════════════════════════════════════════════════════════════════
// Performance Tracking — TVL & Drawdown (Not double-counting)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_peak_tvl_updates_correctly() {
    let mut state = RunState {
        initial_capital: 100_000.0,
        peak_tvl: 100_000.0,
        ..Default::default()
    };

    // Tick 1: TVL rises to 105k
    let tvl = 105_000.0;
    if tvl > state.peak_tvl {
        state.peak_tvl = tvl;
    }
    assert_eq!(state.peak_tvl, 105_000.0);

    // Tick 2: TVL drops to 98k — peak should NOT drop
    let tvl = 98_000.0;
    if tvl > state.peak_tvl {
        state.peak_tvl = tvl;
    }
    assert_eq!(state.peak_tvl, 105_000.0);

    // Tick 3: TVL rises to 110k — peak updates
    let tvl = 110_000.0;
    if tvl > state.peak_tvl {
        state.peak_tvl = tvl;
    }
    assert_eq!(state.peak_tvl, 110_000.0);
}

#[tokio::test]
async fn test_tvl_not_double_counted_with_balances() {
    // Engine.total_tvl() sums venue values + idle balances.
    // Ensure moving capital between venues/balances doesn't inflate TVL.
    let wf = simple_lending_workflow();
    let venues: HashMap<String, Box<dyn Venue>> = {
        let mut m = HashMap::new();
        m.insert(
            "lend_usdc".to_string(),
            Box::new(MockLendingVenue::new(30000.0)) as Box<dyn Venue>,
        );
        m
    };
    let mut engine = Engine::new(wf, venues);

    // 20k in wallet balance + 30k in lending venue = 50k total
    engine.balances.add("wallet", "USDC", 20000.0);
    let tvl = engine.total_tvl().await;
    assert!((tvl - 50000.0).abs() < 0.01, "TVL should be 50000, got {tvl}");

    // Withdraw from venue to balance: TVL unchanged
    if let Some(venue) = engine.venues.get_mut("lend_usdc") {
        let freed = venue.unwind(0.5).await.unwrap();
        engine.balances.add("wallet", "USDC", freed);
    }
    let tvl_after = engine.total_tvl().await;
    assert!(
        (tvl_after - 50000.0).abs() < 0.01,
        "TVL after unwind should still be ~50000, got {tvl_after}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Performance Tracking — Withdrawals Not Treated as Losses
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_withdrawal_is_not_a_loss() {
    // If a user withdraws 10k from a 50k position, the PnL should be:
    // (remaining_tvl + withdrawn) - initial_capital, not (remaining_tvl - initial_capital)
    //
    // In our model, initial_capital is only set once at deploy. If the vault
    // withdraws funds (reserve management), the freed capital goes to the vault
    // and is still value — it just left the strategy.
    //
    // The key insight: initial_capital stays fixed. TVL tracks what's still in
    // the strategy. PnL = (TVL + cumulative_income) - initial_capital + reserves freed.
    let mut state = RunState {
        initial_capital: 50000.0,
        peak_tvl: 50000.0,
        cumulative_interest: 200.0,
        ..Default::default()
    };

    // Simulate reserve unwind: 10k freed and transferred to vault
    state.balances.insert("lend_usdc".into(), 40000.0);

    let tvl: f64 = state.balances.values().sum();
    assert_eq!(tvl, 40000.0);

    // The PnL should account for the freed capital going to vault
    // Reserve actions track what was freed
    let freed_to_vault = 10000.0;
    let total_value = tvl + freed_to_vault;
    let pnl = total_value - state.initial_capital + state.cumulative_interest;
    assert!(
        (pnl - 200.0).abs() < 0.01,
        "PnL should be $200 (interest only), got {pnl}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Performance Tracking — Metrics from Multiple Venues
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_collect_metrics_aggregates_across_venues() {
    let wf = dn_workflow();
    let venues: HashMap<String, Box<dyn Venue>> = {
        let mut m = HashMap::new();
        m.insert(
            "short_eth".to_string(),
            Box::new(MockPerpVenue::with_metrics(25000.0, 100.0, 5.0)) as Box<dyn Venue>,
        );
        m.insert(
            "buy_eth".to_string(),
            Box::new(MockPerpVenue::with_metrics(25000.0, 0.0, 3.0)) as Box<dyn Venue>,
        );
        m.insert(
            "lend_usdc".to_string(),
            Box::new(MockLendingVenue::with_metrics(10000.0, 50.0, 20.0)) as Box<dyn Venue>,
        );
        m
    };
    let engine = Engine::new(wf, venues);

    let metrics = engine.collect_metrics();
    assert!(
        (metrics.funding_pnl - 100.0).abs() < 0.01,
        "Funding should be 100, got {}",
        metrics.funding_pnl
    );
    assert!(
        (metrics.lending_interest - 50.0).abs() < 0.01,
        "Interest should be 50, got {}",
        metrics.lending_interest
    );
    assert!(
        (metrics.rewards_pnl - 20.0).abs() < 0.01,
        "Rewards should be 20, got {}",
        metrics.rewards_pnl
    );
    assert!(
        (metrics.swap_costs - 8.0).abs() < 0.01,
        "Costs should be 8, got {}",
        metrics.swap_costs
    );
}

#[tokio::test]
async fn test_state_stores_cumulative_metrics() {
    let mut state = RunState {
        initial_capital: 100_000.0,
        peak_tvl: 100_000.0,
        ..Default::default()
    };

    // Simulate collecting metrics at tick
    let metrics = SimMetrics {
        funding_pnl: 500.0,
        lending_interest: 300.0,
        rewards_pnl: 100.0,
        swap_costs: 25.0,
        ..Default::default()
    };

    state.cumulative_funding = metrics.funding_pnl;
    state.cumulative_interest = metrics.lending_interest;
    state.cumulative_rewards = metrics.rewards_pnl;
    state.cumulative_costs = metrics.swap_costs;

    assert_eq!(state.cumulative_funding, 500.0);
    assert_eq!(state.cumulative_interest, 300.0);
    assert_eq!(state.cumulative_rewards, 100.0);
    assert_eq!(state.cumulative_costs, 25.0);

    // Roundtrip through serialization
    let json = serde_json::to_string(&state).unwrap();
    let loaded: RunState = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.cumulative_funding, 500.0);
    assert_eq!(loaded.cumulative_interest, 300.0);
}

// ═══════════════════════════════════════════════════════════════════
// Valuer — Config Parsing
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_valuer_config_parsing_full() {
    let json = r#"{
        "contract": "valuer_contract",
        "strategy_id": "lending",
        "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
        "confidence": 90,
        "underlying_decimals": 6,
        "push_interval": 3600,
        "ttl": 7200
    }"#;

    let config: ValuerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.contract, "valuer_contract");
    assert_eq!(config.strategy_id, "lending");
    assert_eq!(config.chain.name, "hyperevm");
    assert_eq!(config.chain.chain_id, Some(999));
    assert_eq!(config.confidence, 90);
    assert_eq!(config.underlying_decimals, 6);
    assert_eq!(config.push_interval, 3600);
    assert_eq!(config.ttl, 7200);
}

#[test]
fn test_valuer_config_defaults() {
    // Minimal config — defaults should fill in
    let json = r#"{
        "contract": "valuer_contract",
        "strategy_id": "test",
        "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" }
    }"#;

    let config: ValuerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.confidence, 90);
    assert_eq!(config.underlying_decimals, 6);
    assert_eq!(config.push_interval, 3600);
    assert_eq!(config.ttl, 7200);
}

#[test]
fn test_valuer_config_custom_decimals() {
    // ETH-based vault: 18 decimals
    let json = r#"{
        "contract": "valuer_contract",
        "strategy_id": "eth_vault",
        "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
        "underlying_decimals": 18
    }"#;

    let config: ValuerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.underlying_decimals, 18);
}

#[test]
fn test_valuer_config_in_workflow() {
    // Full workflow with valuer section
    let json = r#"{
        "name": "Test Strategy",
        "tokens": { "USDC": { "hyperevm": "0xb88339CB7199b77E23DB6E890353E22632Ba630f" } },
        "contracts": {
            "valuer_contract": { "hyperevm": "0x1234567890123456789012345678901234567890" }
        },
        "valuer": {
            "contract": "valuer_contract",
            "strategy_id": "test",
            "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
            "confidence": 95,
            "push_interval": 1800
        },
        "nodes": [
            {
                "type": "wallet",
                "id": "wallet",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
                "token": "USDC",
                "address": "0x1234567890123456789012345678901234567890"
            }
        ],
        "edges": []
    }"#;

    let wf: Workflow = serde_json::from_str(json).unwrap();
    assert!(wf.valuer.is_some());
    let vc = wf.valuer.unwrap();
    assert_eq!(vc.strategy_id, "test");
    assert_eq!(vc.confidence, 95);
    assert_eq!(vc.push_interval, 1800);
}

#[test]
fn test_workflow_without_valuer() {
    let json = r#"{
        "name": "No Valuer Strategy",
        "nodes": [
            {
                "type": "wallet",
                "id": "w",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
                "token": "USDC",
                "address": "0x1234567890123456789012345678901234567890"
            }
        ],
        "edges": []
    }"#;

    let wf: Workflow = serde_json::from_str(json).unwrap();
    assert!(wf.valuer.is_none());
}

// ═══════════════════════════════════════════════════════════════════
// Valuer — Strategy ID Computation
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_strategy_id_deterministic() {
    let id1 = valuer::strategy_id_from_text("lending");
    let id2 = valuer::strategy_id_from_text("lending");
    assert_eq!(id1, id2, "Same text should produce same strategy ID");
}

#[test]
fn test_strategy_id_differs_for_different_text() {
    let id_lending = valuer::strategy_id_from_text("lending");
    let id_dn = valuer::strategy_id_from_text("delta_neutral_basic");
    let id_pt = valuer::strategy_id_from_text("pt_yield");

    assert_ne!(id_lending, id_dn);
    assert_ne!(id_lending, id_pt);
    assert_ne!(id_dn, id_pt);
}

#[test]
fn test_strategy_id_is_keccak256() {
    // Verify against known keccak256 value
    use alloy::primitives::keccak256;
    let expected = keccak256(b"lending");
    let actual = valuer::strategy_id_from_text("lending");
    assert_eq!(actual, expected);
}

// ═══════════════════════════════════════════════════════════════════
// Valuer — TVL Scaling
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_tvl_to_uint256_usdc_6_decimals() {
    use alloy::primitives::U256;

    // $50,000 USDC (6 decimals) → 50_000_000_000
    let val = valuer::tvl_to_uint256(50000.0, 6);
    assert_eq!(val, U256::from(50_000_000_000u64));
}

#[test]
fn test_tvl_to_uint256_eth_18_decimals() {
    use alloy::primitives::U256;

    // 1.5 ETH (18 decimals) → 1_500_000_000_000_000_000
    let val = valuer::tvl_to_uint256(1.5, 18);
    assert_eq!(val, U256::from(1_500_000_000_000_000_000u128));
}

#[test]
fn test_tvl_to_uint256_zero() {
    use alloy::primitives::U256;

    let val = valuer::tvl_to_uint256(0.0, 6);
    assert_eq!(val, U256::ZERO);
}

#[test]
fn test_tvl_to_uint256_negative_clamped() {
    use alloy::primitives::U256;

    // Negative TVL (shouldn't happen but ensure no underflow)
    let val = valuer::tvl_to_uint256(-1000.0, 6);
    assert_eq!(val, U256::ZERO);
}

#[test]
fn test_tvl_to_uint256_fractional_cents() {
    use alloy::primitives::U256;

    // $100.123456 → 100_123_456 (6 decimals)
    let val = valuer::tvl_to_uint256(100.123456, 6);
    assert_eq!(val, U256::from(100_123_456u64));
}

// ═══════════════════════════════════════════════════════════════════
// Valuer — Throttle State
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_valuer_state_default() {
    let vs = valuer::ValuerState::default();
    assert_eq!(vs.last_push, 0);
}

// ═══════════════════════════════════════════════════════════════════
// $DERIVED Wallet Address Validation
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_normal_address_passes_validation() {
    let json = r#"{
        "name": "Normal Wallet Test",
        "nodes": [
            {
                "type": "wallet",
                "id": "wallet",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
                "token": "USDC",
                "address": "0x1234567890123456789012345678901234567890"
            },
            {
                "type": "lending",
                "id": "lend",
                "archetype": "aave_v3",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
                "pool_address": "pool",
                "asset": "USDC",
                "action": "supply"
            }
        ],
        "edges": [
            { "from_node": "wallet", "to_node": "lend", "token": "USDC", "amount": { "type": "all" } }
        ]
    }"#;

    let wf: Workflow = serde_json::from_str(json).unwrap();
    let result = validate::validate(&wf);
    assert!(result.is_ok());
}

#[test]
fn test_empty_address_fails_validation() {
    let json = r#"{
        "name": "Empty Wallet Test",
        "nodes": [
            {
                "type": "wallet",
                "id": "wallet",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
                "token": "USDC",
                "address": ""
            }
        ],
        "edges": []
    }"#;

    let wf: Workflow = serde_json::from_str(json).unwrap();
    let result = validate::validate(&wf);
    assert!(result.is_err(), "Empty address should fail validation");
}

#[test]
fn test_invalid_address_fails_validation() {
    let json = r#"{
        "name": "Invalid Wallet Test",
        "nodes": [
            {
                "type": "wallet",
                "id": "wallet",
                "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.example.com" },
                "token": "USDC",
                "address": "not_a_valid_address"
            }
        ],
        "edges": []
    }"#;

    let wf: Workflow = serde_json::from_str(json).unwrap();
    let result = validate::validate(&wf);
    assert!(result.is_err(), "Invalid address should fail validation");
}

// ═══════════════════════════════════════════════════════════════════
// Vault Strategy JSON Parsing
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_parse_lending_strategy() {
    let json = include_str!("../experiment/vault-strategies/lending.json");
    let wf: Workflow = serde_json::from_str(json).unwrap();
    assert_eq!(wf.name, "USDC Lending");
    assert!(wf.valuer.is_some());
    assert!(wf.reserve.is_some());
    assert_eq!(wf.valuer.as_ref().unwrap().strategy_id, "lending");
    assert_eq!(wf.reserve.as_ref().unwrap().vault_token, "USDC");

    // Wallet should use placeholder (injected at container startup)
    if let Node::Wallet { address, .. } = &wf.nodes[0] {
        assert_eq!(address, "0x_REPLACE_WALLET");
    } else {
        panic!("First node should be a wallet");
    }
}

#[test]
fn test_parse_dn_strategy() {
    let json = include_str!("../experiment/vault-strategies/delta_neutral_basic.json");
    let wf: Workflow = serde_json::from_str(json).unwrap();
    assert_eq!(wf.name, "Basic Delta Neutral");
    assert!(wf.valuer.is_some());
    assert!(wf.reserve.is_some());
    assert_eq!(
        wf.valuer.as_ref().unwrap().strategy_id,
        "delta_neutral_basic"
    );
    assert_eq!(wf.nodes.len(), 5); // wallet, optimizer, spot, perp, lending
}

#[test]
fn test_parse_pt_strategy() {
    let json = include_str!("../experiment/vault-strategies/pt_yield.json");
    let wf: Workflow = serde_json::from_str(json).unwrap();
    assert_eq!(wf.name, "PT Fixed Yield");
    assert!(wf.valuer.is_some());
    assert!(wf.reserve.is_some());
    assert_eq!(wf.valuer.as_ref().unwrap().strategy_id, "pt_yield");
}

#[test]
fn test_all_vault_strategies_have_same_vault_config() {
    // All strategies should reference the same Morpho vault
    let lending: Workflow =
        serde_json::from_str(include_str!("../experiment/vault-strategies/lending.json")).unwrap();
    let dn: Workflow = serde_json::from_str(include_str!(
        "../experiment/vault-strategies/delta_neutral_basic.json"
    ))
    .unwrap();
    let pt: Workflow =
        serde_json::from_str(include_str!("../experiment/vault-strategies/pt_yield.json")).unwrap();

    for wf in [&lending, &dn, &pt] {
        let rc = wf.reserve.as_ref().expect("should have reserve config");
        assert_eq!(rc.vault_address, "morpho_usdc_vault");
        assert_eq!(rc.vault_token, "USDC");
        assert_eq!(rc.vault_chain.name, "hyperevm");

        let vc = wf.valuer.as_ref().expect("should have valuer config");
        assert_eq!(vc.contract, "valuer_contract");
        assert_eq!(vc.chain.name, "hyperevm");
        assert_eq!(vc.underlying_decimals, 6);

        // Contracts manifest should have both placeholders
        let contracts = wf.contracts.as_ref().expect("should have contracts");
        assert!(contracts.contains_key("morpho_usdc_vault"));
        assert!(contracts.contains_key("valuer_contract"));
    }
}

#[test]
fn test_strategy_ids_are_unique() {
    let lending: Workflow =
        serde_json::from_str(include_str!("../experiment/vault-strategies/lending.json")).unwrap();
    let dn: Workflow = serde_json::from_str(include_str!(
        "../experiment/vault-strategies/delta_neutral_basic.json"
    ))
    .unwrap();
    let pt: Workflow =
        serde_json::from_str(include_str!("../experiment/vault-strategies/pt_yield.json")).unwrap();

    let ids: Vec<&str> = vec![
        lending.valuer.as_ref().unwrap().strategy_id.as_str(),
        dn.valuer.as_ref().unwrap().strategy_id.as_str(),
        pt.valuer.as_ref().unwrap().strategy_id.as_str(),
    ];

    // All unique
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "Strategy IDs must be unique");
}

// ═══════════════════════════════════════════════════════════════════
// Complex Scenario: DN with Funding + Lending
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_dn_strategy_perf_tracking_multileg() {
    // Delta-neutral: spot+short perp (hedged pair) + lending
    // funding income should accumulate, lending interest should accumulate,
    // and the two should be independently tracked without conflation.

    let wf = dn_workflow();
    let venues: HashMap<String, Box<dyn Venue>> = {
        let mut m = HashMap::new();
        m.insert(
            "buy_eth".to_string(),
            Box::new(MockPerpVenue::new(25000.0)) as Box<dyn Venue>,
        );
        m.insert(
            "short_eth".to_string(),
            Box::new(MockPerpVenue::with_metrics(25000.0, 50.0, 2.0)) as Box<dyn Venue>,
        );
        m.insert(
            "lend_usdc".to_string(),
            Box::new(MockLendingVenue::with_metrics(50000.0, 30.0, 10.0)) as Box<dyn Venue>,
        );
        m
    };
    let mut engine = Engine::new(wf, venues);
    let mut state = RunState {
        deploy_completed: true,
        initial_capital: 100_000.0,
        peak_tvl: 100_000.0,
        ..Default::default()
    };

    // Tick 1
    engine.tick_venues(1000, 3600.0).await.unwrap();
    let tvl = engine.total_tvl().await;
    if tvl > state.peak_tvl {
        state.peak_tvl = tvl;
    }
    let metrics = engine.collect_metrics();
    state.cumulative_funding = metrics.funding_pnl;
    state.cumulative_interest = metrics.lending_interest;
    state.cumulative_rewards = metrics.rewards_pnl;
    state.cumulative_costs = metrics.swap_costs;

    // Verify: funding comes only from perp, interest only from lending
    assert!(
        (state.cumulative_funding - 50.0).abs() < 0.01,
        "Funding should be 50 (from short_eth), got {}",
        state.cumulative_funding
    );
    assert!(
        (state.cumulative_interest - 30.0).abs() < 0.01,
        "Interest should be 30 (from lend_usdc), got {}",
        state.cumulative_interest
    );
    assert!(
        (state.cumulative_rewards - 10.0).abs() < 0.01,
        "Rewards should be 10, got {}",
        state.cumulative_rewards
    );
    assert!(
        (state.cumulative_costs - 2.0).abs() < 0.01,
        "Costs should be 2 (from short_eth swap), got {}",
        state.cumulative_costs
    );

    // TVL should have increased by the tick accruals (50 funding + 30 interest)
    // buy_eth: 25000, short_eth: 25050, lend_usdc: 50030
    assert!(
        tvl > 100_000.0,
        "TVL should be > 100k after accruals, got {tvl}"
    );
}
