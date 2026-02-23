mod anvil_common;

use alloy::primitives::U256;

use defi_flow::model::chain::Chain;
use defi_flow::model::node::{LendingAction, LendingArchetype, Node};
use defi_flow::venues::lending::aave::AaveLending;
use defi_flow::venues::{ExecutionResult, Venue};

use anvil_common::*;

// ── Constants: Aave V3 on Base ───────────────────────────────────────

const BASE_RPC: &str = "https://mainnet.base.org";
const BASE_CHAIN_ID: u64 = 8453;

const USDC_BASE: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const AAVE_V3_POOL_BASE: &str = "0xA238Dd80C259a72e81d7e4664a9801593F98d1c5";

// Large USDC holder on Base
const USDC_WHALE: &str = "0xF977814e90dA44bFA03b6295A0616a897441aceC";

// ── Helpers ──────────────────────────────────────────────────────────

fn lending_manifests() -> (
    defi_flow::venues::evm::TokenManifest,
    defi_flow::venues::evm::ContractManifest,
) {
    let tokens = token_manifest(&[("USDC", "base", USDC_BASE)]);
    let contracts = contract_manifest(&[("aave_v3_pool", "base", AAVE_V3_POOL_BASE)]);
    (tokens, contracts)
}

fn make_lending_node(chain: &Chain, action: LendingAction) -> Node {
    Node::Lending {
        id: format!("test_{:?}", action).to_lowercase(),
        archetype: LendingArchetype::AaveV3,
        chain: chain.clone(),
        pool_address: "aave_v3_pool".to_string(),
        asset: "USDC".to_string(),
        action,
        rewards_controller: None,
        defillama_slug: None,
        trigger: None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_lending_supply_withdraw() {
    // 1. Fork Base
    let ctx = spawn_fork(BASE_RPC, BASE_CHAIN_ID);
    let chain = Chain::custom("base", BASE_CHAIN_ID, &ctx.rpc_url);

    // 2. Fund test wallet with USDC
    let usdc: alloy::primitives::Address = USDC_BASE.parse().unwrap();
    let whale: alloy::primitives::Address = USDC_WHALE.parse().unwrap();
    let amount_units = U256::from(1_000_000_000u64); // 1000 USDC (6 decimals)

    fund_eth(&ctx.rpc_url, ctx.wallet_address, U256::from(10u128 * 10u128.pow(18))).await;
    fund_erc20(&ctx.rpc_url, usdc, whale, ctx.wallet_address, amount_units).await;

    // Verify funding
    let balance = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(balance >= amount_units, "USDC funding failed: balance={balance}");
    println!("  Funded {balance} USDC to test wallet");

    // 3. Create venue
    let config = make_config(&ctx);
    let (tokens, contracts) = lending_manifests();
    let mut lending = AaveLending::new(&config, &tokens, &contracts).unwrap();

    // 4. Supply 100 USDC
    let supply_node = make_lending_node(&chain, LendingAction::Supply);
    let result = lending.execute(&supply_node, 100.0).await.unwrap();
    match &result {
        ExecutionResult::PositionUpdate { consumed, output } => {
            assert_eq!(*consumed, 100.0);
            assert!(output.is_none());
            println!("  SUPPLY OK: consumed={consumed}");
        }
        other => panic!("Expected PositionUpdate, got {other:?}"),
    }

    // 5. Check USDC balance decreased
    let balance_after_supply = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(
        balance_after_supply < balance,
        "USDC should have decreased after supply"
    );
    println!("  Balance after supply: {balance_after_supply} (was {balance})");

    // 6. Withdraw 100 USDC
    let withdraw_node = make_lending_node(&chain, LendingAction::Withdraw);
    let result = lending.execute(&withdraw_node, 100.0).await.unwrap();
    match &result {
        ExecutionResult::TokenOutput { token, amount } => {
            assert_eq!(token, "USDC");
            assert_eq!(*amount, 100.0);
            println!("  WITHDRAW OK: token={token}, amount={amount}");
        }
        other => panic!("Expected TokenOutput, got {other:?}"),
    }

    // 7. Check USDC balance restored
    let balance_after_withdraw = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(
        balance_after_withdraw > balance_after_supply,
        "USDC should have increased after withdraw"
    );
    println!("  Balance after withdraw: {balance_after_withdraw}");
    println!("  test_lending_supply_withdraw PASSED");
}

#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_lending_borrow_repay() {
    // 1. Fork Base
    let ctx = spawn_fork(BASE_RPC, BASE_CHAIN_ID);
    let chain = Chain::custom("base", BASE_CHAIN_ID, &ctx.rpc_url);

    // 2. Fund test wallet with USDC
    let usdc: alloy::primitives::Address = USDC_BASE.parse().unwrap();
    let whale: alloy::primitives::Address = USDC_WHALE.parse().unwrap();
    let amount_units = U256::from(10_000_000_000u64); // 10000 USDC

    fund_eth(&ctx.rpc_url, ctx.wallet_address, U256::from(10u128 * 10u128.pow(18))).await;
    fund_erc20(&ctx.rpc_url, usdc, whale, ctx.wallet_address, amount_units).await;

    // 3. Create venue
    let config = make_config(&ctx);
    let (tokens, contracts) = lending_manifests();
    let mut lending = AaveLending::new(&config, &tokens, &contracts).unwrap();

    // 4. Supply 5000 USDC as collateral
    let supply_node = make_lending_node(&chain, LendingAction::Supply);
    let result = lending.execute(&supply_node, 5000.0).await.unwrap();
    assert!(matches!(result, ExecutionResult::PositionUpdate { .. }));
    println!("  SUPPLY (collateral) OK");

    // 5. Borrow 100 USDC
    let borrow_node = make_lending_node(&chain, LendingAction::Borrow);
    let result = lending.execute(&borrow_node, 100.0).await.unwrap();
    match &result {
        ExecutionResult::TokenOutput { token, amount } => {
            assert_eq!(token, "USDC");
            assert_eq!(*amount, 100.0);
            println!("  BORROW OK: token={token}, amount={amount}");
        }
        other => panic!("Expected TokenOutput for borrow, got {other:?}"),
    }

    // 6. Repay 100 USDC
    let repay_node = make_lending_node(&chain, LendingAction::Repay);
    let result = lending.execute(&repay_node, 100.0).await.unwrap();
    match &result {
        ExecutionResult::PositionUpdate { consumed, .. } => {
            assert_eq!(*consumed, 100.0);
            println!("  REPAY OK: consumed={consumed}");
        }
        other => panic!("Expected PositionUpdate for repay, got {other:?}"),
    }

    println!("  test_lending_borrow_repay PASSED");
}

/// Dry-run preflight should catch when pool_address doesn't support the specified asset.
/// Here we use the USDC token address as the pool — it has code but no getReserveData.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_lending_dryrun_wrong_pool() {
    let ctx = spawn_fork(BASE_RPC, BASE_CHAIN_ID);
    let chain = Chain::custom("base", BASE_CHAIN_ID, &ctx.rpc_url);

    // Point "bad_pool" at the USDC token contract (not an Aave pool!)
    let tokens = token_manifest(&[("USDC", "base", USDC_BASE)]);
    let contracts = contract_manifest(&[("bad_pool", "base", USDC_BASE)]);

    let mut config = make_config(&ctx);
    config.dry_run = true;

    let mut lending = AaveLending::new(&config, &tokens, &contracts).unwrap();
    let node = Node::Lending {
        id: "test_wrong_pool".to_string(),
        archetype: LendingArchetype::AaveV3,
        chain: chain.clone(),
        pool_address: "bad_pool".to_string(),
        asset: "USDC".to_string(),
        action: LendingAction::Supply,
        rewards_controller: None,
        defillama_slug: None,
        trigger: None,
    };

    let result = lending.execute(&node, 100.0).await;
    assert!(result.is_err(), "Preflight should catch wrong pool address");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("does not support") || err.contains("getReserveData"),
        "Error should mention getReserveData failure, got: {err}"
    );
    println!("  test_lending_dryrun_wrong_pool PASSED: {err}");
}

/// Dry-run preflight should succeed when pool_address is correct.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_lending_dryrun_correct_pool() {
    let ctx = spawn_fork(BASE_RPC, BASE_CHAIN_ID);
    let chain = Chain::custom("base", BASE_CHAIN_ID, &ctx.rpc_url);

    let (tokens, contracts) = lending_manifests();
    let mut config = make_config(&ctx);
    config.dry_run = true;

    let mut lending = AaveLending::new(&config, &tokens, &contracts).unwrap();
    let node = make_lending_node(&chain, LendingAction::Supply);

    let result = lending.execute(&node, 100.0).await;
    assert!(result.is_ok(), "Preflight should pass for correct pool: {:?}", result.err());
    match result.unwrap() {
        ExecutionResult::PositionUpdate { consumed, .. } => {
            assert_eq!(consumed, 100.0);
            println!("  test_lending_dryrun_correct_pool PASSED: dry-run supply OK");
        }
        other => panic!("Expected PositionUpdate, got {other:?}"),
    }
}
