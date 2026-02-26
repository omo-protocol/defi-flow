use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use defi_flow::engine::Engine;
use defi_flow::model::amount::Amount;
use defi_flow::model::chain::Chain;
use defi_flow::model::edge::Edge;
use defi_flow::model::node::{CronInterval, Node, OptimizerStrategy, Trigger, VenueAllocation};
use defi_flow::model::reserve::ReserveConfig;
use defi_flow::model::workflow::Workflow;
use defi_flow::validate;
use defi_flow::venues::{ExecutionResult, Venue};

// ── Mock venue ──────────────────────────────────────────────────────

/// A mock venue with controllable value and unwind behavior.
struct MockVenue {
    value: f64,
}

#[async_trait]
impl Venue for MockVenue {
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
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let f = fraction.clamp(0.0, 1.0);
        let freed = self.value * f;
        self.value -= freed;
        Ok(freed)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn minimal_workflow() -> Workflow {
    Workflow {
        name: "test".into(),
        description: None,
        tokens: None,
        contracts: None,
        reserve: None,
        nodes: vec![Node::Wallet {
            id: "wallet".into(),
            chain: Chain::hyperevm(),
            token: "USDC".into(),
            address: "0x0000000000000000000000000000000000000000".into(),
        }],
        edges: vec![],
    }
}

fn workflow_with_reserve(rc: ReserveConfig) -> Workflow {
    let mut wf = minimal_workflow();
    wf.reserve = Some(rc);
    wf
}

fn valid_reserve_config() -> ReserveConfig {
    ReserveConfig {
        vault_address: "morpho_usdc_vault".into(),
        vault_chain: Chain::base(),
        vault_token: "USDC".into(),
        target_ratio: 0.20,
        trigger_threshold: 0.05,
        min_unwind: 100.0,
    }
}

/// Build a workflow + manifests that pass validation.
fn valid_workflow_with_reserve() -> Workflow {
    let mut wf = workflow_with_reserve(valid_reserve_config());
    wf.tokens = Some(HashMap::from([(
        "USDC".into(),
        HashMap::from([
            (
                "base".into(),
                "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
            ),
            (
                "hyperevm".into(),
                "0x2222222222222222222222222222222222222222".into(),
            ),
        ]),
    )]));
    wf.contracts = Some(HashMap::from([(
        "morpho_usdc_vault".into(),
        HashMap::from([(
            "base".into(),
            "0x616a4E1db48e22028C643323ef2bE4c1f5a3a3E7".into(),
        )]),
    )]));
    wf
}

// ── Validation tests ────────────────────────────────────────────────

#[test]
fn test_reserve_validation_passes_with_valid_config() {
    let wf = valid_workflow_with_reserve();
    let result = validate::validate(&wf);
    assert!(result.is_ok(), "Expected valid, got: {:?}", result);
}

#[test]
fn test_reserve_validation_missing_vault() {
    // vault_address not in contracts manifest
    let mut wf = valid_workflow_with_reserve();
    wf.contracts = Some(HashMap::new()); // empty manifest

    let errors = validate::validate(&wf).unwrap_err();
    let has_vault_err = errors.iter().any(|e| {
        let msg = e.to_string();
        msg.contains("morpho_usdc_vault") && msg.contains("not found in contracts manifest")
    });
    assert!(
        has_vault_err,
        "Expected vault manifest error, got: {:?}",
        errors
    );
}

#[test]
fn test_reserve_validation_missing_token() {
    // vault_token not in tokens manifest
    let mut wf = valid_workflow_with_reserve();
    wf.tokens = Some(HashMap::new()); // empty manifest

    let errors = validate::validate(&wf).unwrap_err();
    let has_token_err = errors.iter().any(|e| {
        let msg = e.to_string();
        msg.contains("USDC") && msg.contains("not found in tokens manifest")
    });
    assert!(
        has_token_err,
        "Expected token manifest error, got: {:?}",
        errors
    );
}

#[test]
fn test_reserve_validation_bad_thresholds() {
    // trigger_threshold >= target_ratio
    let mut rc = valid_reserve_config();
    rc.trigger_threshold = 0.30;
    rc.target_ratio = 0.20;

    let mut wf = valid_workflow_with_reserve();
    wf.reserve = Some(rc);

    let errors = validate::validate(&wf).unwrap_err();
    let has_threshold_err = errors.iter().any(|e| {
        let msg = e.to_string();
        msg.contains("trigger_threshold") && msg.contains("must be less than")
    });
    assert!(
        has_threshold_err,
        "Expected threshold ordering error, got: {:?}",
        errors
    );
}

#[test]
fn test_reserve_validation_zero_target() {
    let mut rc = valid_reserve_config();
    rc.target_ratio = 0.0;

    let mut wf = valid_workflow_with_reserve();
    wf.reserve = Some(rc);

    let errors = validate::validate(&wf).unwrap_err();
    let has_invalid = errors.iter().any(|e| {
        let msg = e.to_string();
        msg.contains("target_ratio") && msg.contains("invalid value")
    });
    assert!(
        has_invalid,
        "Expected invalid target_ratio error, got: {:?}",
        errors
    );
}

#[test]
fn test_reserve_validation_no_rpc() {
    // vault_chain without rpc_url
    let mut rc = valid_reserve_config();
    rc.vault_chain = Chain::named("base"); // no rpc_url

    let mut wf = valid_workflow_with_reserve();
    wf.reserve = Some(rc);

    let errors = validate::validate(&wf).unwrap_err();
    let has_rpc_err = errors.iter().any(|e| {
        let msg = e.to_string();
        msg.contains("no rpc_url")
    });
    assert!(
        has_rpc_err,
        "Expected missing rpc_url error, got: {:?}",
        errors
    );
}

#[test]
fn test_no_reserve_config_passes() {
    // Workflow without reserve config should validate fine
    let wf = minimal_workflow();
    let result = validate::validate(&wf);
    assert!(result.is_ok(), "Expected valid, got: {:?}", result);
}

// ── Engine unwind tests ─────────────────────────────────────────────

fn build_engine_with_mock_venues(values: Vec<(&str, f64)>) -> Engine {
    let node_ids: Vec<String> = values.iter().map(|(id, _)| id.to_string()).collect();

    let nodes: Vec<Node> = node_ids
        .iter()
        .map(|id| Node::Wallet {
            id: id.clone(),
            chain: Chain::hyperevm(),
            token: "USDC".into(),
            address: "0x0000000000000000000000000000000000000000".into(),
        })
        .collect();

    // Create edges so topo sort sees them connected
    let edges: Vec<Edge> = if node_ids.len() > 1 {
        node_ids
            .windows(2)
            .map(|w| Edge {
                from_node: w[0].clone(),
                to_node: w[1].clone(),
                token: "USDC".into(),
                amount: Amount::All,
            })
            .collect()
    } else {
        vec![]
    };

    let workflow = Workflow {
        name: "test_unwind".into(),
        description: None,
        tokens: None,
        contracts: None,
        reserve: None,
        nodes,
        edges,
    };

    let mut venues: HashMap<String, Box<dyn Venue>> = HashMap::new();
    for (id, value) in values {
        venues.insert(id.to_string(), Box::new(MockVenue { value }));
    }

    Engine::new(workflow, venues)
}

#[tokio::test]
async fn test_unwind_single_venue() {
    let mut engine = build_engine_with_mock_venues(vec![("v1", 1000.0)]);

    let venue = engine.venues.get_mut("v1").unwrap();
    let freed = venue.unwind(0.5).await.unwrap();

    assert!((freed - 500.0).abs() < 0.01, "Expected ~500, got {}", freed);

    let remaining = venue.total_value().await.unwrap();
    assert!(
        (remaining - 500.0).abs() < 0.01,
        "Expected ~500 remaining, got {}",
        remaining
    );
}

#[tokio::test]
async fn test_unwind_full_liquidation() {
    let mut engine = build_engine_with_mock_venues(vec![("v1", 1000.0)]);

    let venue = engine.venues.get_mut("v1").unwrap();
    let freed = venue.unwind(1.0).await.unwrap();

    assert!(
        (freed - 1000.0).abs() < 0.01,
        "Expected ~1000, got {}",
        freed
    );

    let remaining = venue.total_value().await.unwrap();
    assert!(remaining < 0.01, "Expected ~0 remaining, got {}", remaining);
}

#[tokio::test]
async fn test_unwind_zero_fraction() {
    let mut engine = build_engine_with_mock_venues(vec![("v1", 1000.0)]);

    let venue = engine.venues.get_mut("v1").unwrap();
    let freed = venue.unwind(0.0).await.unwrap();

    assert!(freed < 0.01, "Expected ~0 freed, got {}", freed);

    let remaining = venue.total_value().await.unwrap();
    assert!(
        (remaining - 1000.0).abs() < 0.01,
        "Expected ~1000 remaining, got {}",
        remaining
    );
}

#[tokio::test]
async fn test_pro_rata_unwind_all_venues() {
    let mut engine = build_engine_with_mock_venues(vec![("v1", 600.0), ("v2", 400.0)]);

    // Simulate a 50% pro-rata unwind across all venues
    let fraction = 0.5;
    let mut total_freed = 0.0;

    let venue_ids: Vec<String> = engine.venues.keys().cloned().collect();
    for id in &venue_ids {
        let venue = engine.venues.get_mut(id.as_str()).unwrap();
        total_freed += venue.unwind(fraction).await.unwrap();
    }

    assert!(
        (total_freed - 500.0).abs() < 0.01,
        "Expected ~500 total freed, got {}",
        total_freed
    );

    // Check individual remaining values
    let v1_remaining = engine
        .venues
        .get("v1")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v2_remaining = engine
        .venues
        .get("v2")
        .unwrap()
        .total_value()
        .await
        .unwrap();

    assert!(
        (v1_remaining - 300.0).abs() < 0.01,
        "v1 expected ~300, got {}",
        v1_remaining
    );
    assert!(
        (v2_remaining - 200.0).abs() < 0.01,
        "v2 expected ~200, got {}",
        v2_remaining
    );
}

#[tokio::test]
async fn test_tvl_after_unwind() {
    let mut engine = build_engine_with_mock_venues(vec![("v1", 600.0), ("v2", 400.0)]);

    let tvl_before = engine.total_tvl().await;
    assert!(
        (tvl_before - 1000.0).abs() < 0.01,
        "Expected TVL ~1000, got {}",
        tvl_before
    );

    // Unwind 30% from all
    let venue_ids: Vec<String> = engine.venues.keys().cloned().collect();
    for id in &venue_ids {
        let venue = engine.venues.get_mut(id.as_str()).unwrap();
        venue.unwind(0.3).await.unwrap();
    }

    let tvl_after = engine.total_tvl().await;
    assert!(
        (tvl_after - 700.0).abs() < 0.01,
        "Expected TVL ~700 after 30% unwind, got {}",
        tvl_after
    );
}

// ── Dry-run integration tests ────────────────────────────────────────

/// Build an engine with an optimizer node + downstream venue nodes.
/// The optimizer has edges to each venue and uses static return/vol
/// so Kelly allocations are deterministic.
fn build_optimizer_engine(
    venue_values: Vec<(&str, f64)>,
    allocations: Vec<VenueAllocation>,
    optimizer_cash: f64,
) -> Engine {
    let venue_ids: Vec<String> = venue_values.iter().map(|(id, _)| id.to_string()).collect();

    let mut nodes: Vec<Node> = vec![
        Node::Wallet {
            id: "wallet".into(),
            chain: Chain::hyperevm(),
            token: "USDC".into(),
            address: "0x0000000000000000000000000000000000000000".into(),
        },
        Node::Optimizer {
            id: "optimizer".into(),
            strategy: OptimizerStrategy::Kelly,
            kelly_fraction: 1.0,
            max_allocation: None,
            drift_threshold: 0.0,
            allocations,
            trigger: Some(Trigger::Cron {
                interval: CronInterval::Daily,
            }),
        },
    ];

    // Add venue nodes (using Wallet as stand-in since MockVenue handles execution)
    for vid in &venue_ids {
        nodes.push(Node::Wallet {
            id: vid.clone(),
            chain: Chain::hyperevm(),
            token: "USDC".into(),
            address: "0x0000000000000000000000000000000000000000".into(),
        });
    }

    // Edges: wallet → optimizer, optimizer → each venue
    let mut edges = vec![Edge {
        from_node: "wallet".into(),
        to_node: "optimizer".into(),
        token: "USDC".into(),
        amount: Amount::All,
    }];
    for vid in &venue_ids {
        edges.push(Edge {
            from_node: "optimizer".into(),
            to_node: vid.clone(),
            token: "USDC".into(),
            amount: Amount::All,
        });
    }

    let workflow = Workflow {
        name: "test_optimizer".into(),
        description: None,
        tokens: None,
        contracts: None,
        reserve: None,
        nodes,
        edges,
    };

    let mut venues: HashMap<String, Box<dyn Venue>> = HashMap::new();
    for (id, value) in venue_values {
        venues.insert(id.to_string(), Box::new(MockVenue { value }));
    }

    let mut engine = Engine::new(workflow, venues);

    // Seed optimizer with cash
    if optimizer_cash > 0.0 {
        engine.balances.add("optimizer", "USDC", optimizer_cash);
    }

    engine
}

/// Test the reserve unwind flow: when venues are over-allocated and we need to
/// free capital pro-rata, verify that freed amounts are correct and TVL is preserved.
#[tokio::test]
async fn test_dry_run_reserve_flow() {
    let mut engine =
        build_engine_with_mock_venues(vec![("v1", 600.0), ("v2", 300.0), ("v3", 100.0)]);

    let tvl_before = engine.total_tvl().await;
    assert!((tvl_before - 1000.0).abs() < 0.01);

    // Simulate reserve deficit: need to free $200 from venues
    let deficit = 200.0;
    let mut total_venue_value = 0.0;
    let venue_ids: Vec<String> = engine.venues.keys().cloned().collect();
    for id in &venue_ids {
        total_venue_value += engine
            .venues
            .get(id.as_str())
            .unwrap()
            .total_value()
            .await
            .unwrap();
    }

    let unwind_fraction = deficit / total_venue_value;
    assert!(
        (unwind_fraction - 0.2).abs() < 0.01,
        "Expected 20% unwind fraction, got {}",
        unwind_fraction
    );

    // Execute pro-rata unwind
    let mut total_freed = 0.0;
    for id in &venue_ids {
        let venue = engine.venues.get_mut(id.as_str()).unwrap();
        let freed = venue.unwind(unwind_fraction).await.unwrap();
        total_freed += freed;
    }

    // Verify total freed matches deficit
    assert!(
        (total_freed - deficit).abs() < 1.0,
        "Expected ~${} freed, got ${}",
        deficit,
        total_freed
    );

    // Verify each venue shrank by ~20%
    let v1 = engine
        .venues
        .get("v1")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v2 = engine
        .venues
        .get("v2")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v3 = engine
        .venues
        .get("v3")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    assert!((v1 - 480.0).abs() < 1.0, "v1 expected ~480, got {}", v1);
    assert!((v2 - 240.0).abs() < 1.0, "v2 expected ~240, got {}", v2);
    assert!((v3 - 80.0).abs() < 1.0, "v3 expected ~80, got {}", v3);

    // TVL should decrease by exactly the freed amount (capital leaves venues)
    let tvl_after = engine.total_tvl().await;
    assert!(
        (tvl_after - (tvl_before - total_freed)).abs() < 1.0,
        "TVL should be {} but got {}",
        tvl_before - total_freed,
        tvl_after
    );
}

/// Test additive rebalance: optimizer has cash, venues are under-allocated.
/// Capital should flow from optimizer to venues proportionally.
#[tokio::test]
async fn test_dry_run_additive_rebalance() {
    // Two venues with static 50/50 allocation (equal return/vol → equal Kelly fractions)
    let allocations = vec![
        VenueAllocation {
            target_node: Some("v1".into()),
            target_nodes: vec![],
            expected_return: Some(0.10),
            volatility: Some(0.20),
            correlation: 0.0,
        },
        VenueAllocation {
            target_node: Some("v2".into()),
            target_nodes: vec![],
            expected_return: Some(0.10),
            volatility: Some(0.20),
            correlation: 0.0,
        },
    ];

    // Venues start empty, optimizer has $1000 cash
    let mut engine = build_optimizer_engine(vec![("v1", 0.0), ("v2", 0.0)], allocations, 1000.0);

    let tvl_before = engine.total_tvl().await;
    assert!(
        (tvl_before - 1000.0).abs() < 0.01,
        "Expected TVL ~1000, got {}",
        tvl_before
    );

    // Execute the optimizer node (deploy phase)
    engine.execute_node("optimizer").await.unwrap();

    // Both venues should receive capital (exact split depends on Kelly)
    let v1_value = engine
        .venues
        .get("v1")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v2_value = engine
        .venues
        .get("v2")
        .unwrap()
        .total_value()
        .await
        .unwrap();

    // With equal return/vol, both should get the same allocation
    assert!(
        (v1_value - v2_value).abs() < 1.0,
        "Expected equal allocation: v1=${}, v2=${}",
        v1_value,
        v2_value
    );

    // Total deployed should be > 0 (optimizer saw positive expected return)
    let total_deployed = v1_value + v2_value;
    assert!(
        total_deployed > 0.0,
        "Expected some capital deployed, got {}",
        total_deployed
    );

    // TVL preserved: venue values + remaining optimizer balance == original
    let tvl_after = engine.total_tvl().await;
    assert!(
        (tvl_after - tvl_before).abs() < 1.0,
        "TVL not preserved: before={}, after={}",
        tvl_before,
        tvl_after
    );
}

/// Test subtractive rebalance: venues are over-allocated, optimizer should unwind
/// from the over-allocated venue and deploy to the under-allocated one.
#[tokio::test]
async fn test_dry_run_subtractive_rebalance() {
    // v1 has 80% expected return → should get more allocation
    // v2 has 10% expected return → should get less
    let allocations = vec![
        VenueAllocation {
            target_node: Some("v1".into()),
            target_nodes: vec![],
            expected_return: Some(0.80),
            volatility: Some(0.20),
            correlation: 0.0,
        },
        VenueAllocation {
            target_node: Some("v2".into()),
            target_nodes: vec![],
            expected_return: Some(0.02),
            volatility: Some(0.20),
            correlation: 0.0,
        },
    ];

    // Start with equal venue values — v2 is over-allocated relative to Kelly target
    let mut engine = build_optimizer_engine(vec![("v1", 500.0), ("v2", 500.0)], allocations, 0.0);

    let tvl_before = engine.total_tvl().await;
    let v1_before = engine
        .venues
        .get("v1")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v2_before = engine
        .venues
        .get("v2")
        .unwrap()
        .total_value()
        .await
        .unwrap();

    // Execute optimizer — should unwind from v2 (over-allocated) and deploy to v1
    engine.execute_node("optimizer").await.unwrap();

    let v1_after = engine
        .venues
        .get("v1")
        .unwrap()
        .total_value()
        .await
        .unwrap();
    let v2_after = engine
        .venues
        .get("v2")
        .unwrap()
        .total_value()
        .await
        .unwrap();

    // v1 (high return) should have grown or stayed the same
    assert!(
        v1_after >= v1_before - 1.0,
        "v1 should not decrease: before={}, after={}",
        v1_before,
        v1_after
    );

    // v2 (low return) should have shrunk
    assert!(
        v2_after < v2_before,
        "v2 should decrease: before={}, after={}",
        v2_before,
        v2_after
    );

    // TVL preserved (capital just reshuffled)
    let tvl_after = engine.total_tvl().await;
    assert!(
        (tvl_after - tvl_before).abs() < 1.0,
        "TVL not preserved: before={}, after={}",
        tvl_before,
        tvl_after
    );
}

/// Verify the fundamental invariant: for any unwind fraction,
/// freed + remaining_value == original_value (no leakage, no double-counting).
#[tokio::test]
async fn test_unwind_preserves_tvl_invariant() {
    let fractions = [0.01, 0.1, 0.25, 0.5, 0.75, 0.99, 1.0];

    for frac in fractions {
        let mut engine = build_engine_with_mock_venues(vec![("v", 1000.0)]);

        let original = engine.venues.get("v").unwrap().total_value().await.unwrap();
        let freed = engine
            .venues
            .get_mut("v")
            .unwrap()
            .unwind(frac)
            .await
            .unwrap();
        let remaining = engine.venues.get("v").unwrap().total_value().await.unwrap();

        assert!(
            (freed + remaining - original).abs() < 0.01,
            "Invariant violated at frac={}: freed={} + remaining={} != original={}",
            frac,
            freed,
            remaining,
            original,
        );
    }
}
