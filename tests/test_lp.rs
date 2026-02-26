mod anvil_common;

use alloy::primitives::U256;

use defi_flow::model::chain::Chain;
use defi_flow::model::node::{LpAction, LpVenue, Node};
use defi_flow::venues::lp::aerodrome::AerodromeLp;
use defi_flow::venues::{ExecutionResult, Venue};

use anvil_common::*;

// ── Constants ────────────────────────────────────────────────────────

const BASE_RPC: &str = "https://mainnet.base.org";
const BASE_CHAIN_ID: u64 = 8453;

const WETH_BASE: &str = "0x4200000000000000000000000000000000000006";
const USDC_BASE: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const AERODROME_POSITION_MANAGER: &str = "0x827922686190790b37229fd06084350E74485b72";

// Large USDC holder on Base (Binance)
const USDC_WHALE_BASE: &str = "0xF977814e90dA44bFA03b6295A0616a897441aceC";

// ── Helpers ──────────────────────────────────────────────────────────

fn lp_manifests() -> (
    defi_flow::venues::evm::TokenManifest,
    defi_flow::venues::evm::ContractManifest,
) {
    let tokens = token_manifest(&[("WETH", "base", WETH_BASE), ("USDC", "base", USDC_BASE)]);
    let contracts = contract_manifest(&[(
        "aerodrome_position_manager",
        "base",
        AERODROME_POSITION_MANAGER,
    )]);
    (tokens, contracts)
}

// ── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_lp_add_liquidity() {
    // 1. Fork Base
    let ctx = spawn_fork(BASE_RPC, BASE_CHAIN_ID);
    let chain = Chain::custom("base", BASE_CHAIN_ID, &ctx.rpc_url);

    // 2. Fund test wallet
    let weth: alloy::primitives::Address = WETH_BASE.parse().unwrap();
    let usdc: alloy::primitives::Address = USDC_BASE.parse().unwrap();
    let usdc_whale: alloy::primitives::Address = USDC_WHALE_BASE.parse().unwrap();

    // Fund ETH for gas + WETH wrapping
    fund_eth(
        &ctx.rpc_url,
        ctx.wallet_address,
        U256::from(100u128 * 10u128.pow(18)), // 100 ETH
    )
    .await;

    // Wrap 1 ETH → WETH
    wrap_eth(
        &ctx.rpc_url,
        &ctx.private_key,
        weth,
        U256::from(10u64.pow(18)), // 1 WETH
    )
    .await;

    // Fund 10,000 USDC from whale
    let usdc_amount = U256::from(10_000_000_000u64); // 10,000 USDC (6 decimals)
    fund_erc20(
        &ctx.rpc_url,
        usdc,
        usdc_whale,
        ctx.wallet_address,
        usdc_amount,
    )
    .await;

    // Verify funding
    let weth_balance = balance_of(&ctx.rpc_url, weth, ctx.wallet_address).await;
    let usdc_balance = balance_of(&ctx.rpc_url, usdc, ctx.wallet_address).await;
    println!("  WETH balance: {weth_balance}");
    println!("  USDC balance: {usdc_balance}");
    assert!(weth_balance > U256::ZERO, "WETH funding failed");
    assert!(usdc_balance > U256::ZERO, "USDC funding failed");

    // 3. Create LP venue with custom chain pointing to Anvil
    let config = make_config(&ctx);
    let (tokens, contracts) = lp_manifests();
    let mut lp = AerodromeLp::new(&config, &tokens, &contracts, chain).unwrap();

    // 4. Add liquidity — full range, tick spacing 100
    let node = Node::Lp {
        id: "test_add_liq".to_string(),
        venue: LpVenue::Aerodrome,
        pool: "WETH/USDC".to_string(),
        action: LpAction::AddLiquidity,
        tick_lower: None,
        tick_upper: None,
        tick_spacing: Some(100),
        chain: None,
        trigger: None,
    };

    let result = lp.execute(&node, 200.0).await.unwrap();
    match &result {
        ExecutionResult::PositionUpdate { consumed, output } => {
            assert_eq!(*consumed, 200.0);
            assert!(output.is_none());
            println!("  ADD_LIQUIDITY OK: consumed={consumed}");
        }
        other => panic!("Expected PositionUpdate, got {other:?}"),
    }

    // 5. Verify total value tracked
    let tv = lp.total_value().await.unwrap();
    assert!(tv > 0.0, "Total value should be > 0 after adding liquidity");
    println!("  Total value: ${tv:.2}");
    println!("  test_lp_add_liquidity PASSED");
}
