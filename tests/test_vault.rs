mod anvil_common;

use alloy::primitives::U256;

use defi_flow::model::chain::Chain;
use defi_flow::model::node::{Node, VaultAction, VaultArchetype};
use defi_flow::venues::vault::morpho::MorphoVault;
use defi_flow::venues::{ExecutionResult, Venue};

use anvil_common::*;

// ── Constants ────────────────────────────────────────────────────────

const ETHEREUM_RPC: &str = "https://ethereum-rpc.publicnode.com";
const ETHEREUM_CHAIN_ID: u64 = 1;

const USDC_ETHEREUM: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const STEAKHOUSE_USDC_VAULT: &str = "0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB";

// Binance 14 — large USDC holder on Ethereum mainnet
const USDC_WHALE: &str = "0x28C6c06298d514Db089934071355E5743bf21d60";

// ── Helpers ──────────────────────────────────────────────────────────

fn vault_manifests() -> (
    defi_flow::venues::evm::TokenManifest,
    defi_flow::venues::evm::ContractManifest,
) {
    let tokens = token_manifest(&[("USDC", "ethereum", USDC_ETHEREUM)]);
    let contracts = contract_manifest(&[(
        "steakhouse_usdc",
        "ethereum",
        STEAKHOUSE_USDC_VAULT,
    )]);
    (tokens, contracts)
}

fn make_vault_node(chain: &Chain, action: VaultAction) -> Node {
    Node::Vault {
        id: format!("test_{:?}", action).to_lowercase(),
        archetype: VaultArchetype::MorphoV2,
        chain: chain.clone(),
        vault_address: "steakhouse_usdc".to_string(),
        asset: "USDC".to_string(),
        action,
        defillama_slug: None,
        trigger: None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_vault_deposit_withdraw() {
    // 1. Fork Ethereum mainnet
    let ctx = spawn_fork(ETHEREUM_RPC, ETHEREUM_CHAIN_ID);
    let chain = Chain::custom("ethereum", ETHEREUM_CHAIN_ID, &ctx.rpc_url);

    // 2. Fund test wallet with USDC
    let usdc: alloy::primitives::Address = USDC_ETHEREUM.parse().unwrap();
    let whale: alloy::primitives::Address = USDC_WHALE.parse().unwrap();
    let amount_units = U256::from(10_000_000_000u64); // 10,000 USDC (6 decimals)

    fund_eth(&ctx.rpc_url, ctx.wallet_address, U256::from(10u128 * 10u128.pow(18))).await;
    fund_erc20(&ctx.rpc_url, usdc, whale, ctx.wallet_address, amount_units).await;

    // Verify funding
    let balance = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(balance >= amount_units, "USDC funding failed: balance={balance}");
    println!("  Funded {balance} USDC to test wallet");

    // 3. Create venue
    let config = make_config(&ctx);
    let (tokens, contracts) = vault_manifests();
    let mut vault = MorphoVault::new(&config, &tokens, &contracts).unwrap();

    // 4. Deposit 1000 USDC
    let deposit_node = make_vault_node(&chain, VaultAction::Deposit);
    let result = vault.execute(&deposit_node, 1000.0).await.unwrap();
    match &result {
        ExecutionResult::PositionUpdate { consumed, output } => {
            assert_eq!(*consumed, 1000.0);
            assert!(output.is_none());
            println!("  DEPOSIT OK: consumed={consumed}");
        }
        other => panic!("Expected PositionUpdate, got {other:?}"),
    }

    // 5. Check USDC balance decreased
    let balance_after_deposit = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(
        balance_after_deposit < balance,
        "USDC should have decreased after deposit"
    );
    println!("  Balance after deposit: {balance_after_deposit}");

    // 6. Check vault share balance > 0
    let vault_addr: alloy::primitives::Address = STEAKHOUSE_USDC_VAULT.parse().unwrap();
    let shares = balance_of(&ctx.rpc_url, vault_addr, ctx.wallet_address).await;
    assert!(shares > U256::ZERO, "Should have vault shares after deposit");
    println!("  Vault shares: {shares}");

    // 7. Withdraw 1000 USDC
    let withdraw_node = make_vault_node(&chain, VaultAction::Withdraw);
    let result = vault.execute(&withdraw_node, 1000.0).await.unwrap();
    match &result {
        ExecutionResult::TokenOutput { token, amount } => {
            assert_eq!(token, "USDC");
            assert_eq!(*amount, 1000.0);
            println!("  WITHDRAW OK: token={token}, amount={amount}");
        }
        other => panic!("Expected TokenOutput, got {other:?}"),
    }

    // 8. Check USDC balance restored
    let balance_after_withdraw = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    assert!(
        balance_after_withdraw > balance_after_deposit,
        "USDC should have increased after withdraw"
    );
    println!("  Balance after withdraw: {balance_after_withdraw}");
    println!("  test_vault_deposit_withdraw PASSED");
}

/// Dry-run preflight should catch when vault_address points to the wrong contract.
/// Here we use the USDC token address as the vault — it has code but no asset() method.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_vault_dryrun_wrong_address() {
    let ctx = spawn_fork(ETHEREUM_RPC, ETHEREUM_CHAIN_ID);
    let chain = Chain::custom("ethereum", ETHEREUM_CHAIN_ID, &ctx.rpc_url);

    // Point "bad_vault" at the USDC token contract (not a vault!)
    let tokens = token_manifest(&[("USDC", "ethereum", USDC_ETHEREUM)]);
    let contracts = contract_manifest(&[("bad_vault", "ethereum", USDC_ETHEREUM)]);

    let mut config = make_config(&ctx);
    config.dry_run = true;

    let mut vault = MorphoVault::new(&config, &tokens, &contracts).unwrap();
    let node = Node::Vault {
        id: "test_wrong_vault".to_string(),
        archetype: VaultArchetype::MorphoV2,
        chain: chain.clone(),
        vault_address: "bad_vault".to_string(),
        asset: "USDC".to_string(),
        action: VaultAction::Deposit,
        defillama_slug: None,
        trigger: None,
    };

    let result = vault.execute(&node, 1000.0).await;
    assert!(result.is_err(), "Preflight should catch wrong vault address");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("asset()") || err.contains("asset mismatch"),
        "Error should mention vault.asset() failure, got: {err}"
    );
    println!("  test_vault_dryrun_wrong_address PASSED: {err}");
}

/// Dry-run preflight should succeed when vault_address is correct.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_vault_dryrun_correct_address() {
    let ctx = spawn_fork(ETHEREUM_RPC, ETHEREUM_CHAIN_ID);
    let chain = Chain::custom("ethereum", ETHEREUM_CHAIN_ID, &ctx.rpc_url);

    let (tokens, contracts) = vault_manifests();
    let mut config = make_config(&ctx);
    config.dry_run = true;

    let mut vault = MorphoVault::new(&config, &tokens, &contracts).unwrap();
    let node = make_vault_node(&chain, VaultAction::Deposit);

    let result = vault.execute(&node, 1000.0).await;
    assert!(result.is_ok(), "Preflight should pass for correct vault: {:?}", result.err());
    match result.unwrap() {
        ExecutionResult::PositionUpdate { consumed, .. } => {
            assert_eq!(consumed, 1000.0);
            println!("  test_vault_dryrun_correct_address PASSED: dry-run deposit OK");
        }
        other => panic!("Expected PositionUpdate, got {other:?}"),
    }
}
