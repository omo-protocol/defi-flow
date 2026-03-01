mod anvil_common;

use alloy::primitives::{address, Address, U256};
use alloy::sol;

use anvil_common::*;

// ── Constants ────────────────────────────────────────────────────────

const HYPEREVM_RPC: &str = "https://rpc.hyperliquid.xyz/evm";
const HYPEREVM_CHAIN_ID: u64 = 999;

// PT-hwHLP contracts (accepts USDT0 directly, expiry 2026-06-25)
const PENDLE_ROUTER: &str = "0x888888888889758F76e7103c6CbF23ABbF58F946";
const PT_HWHLP_SY: &str = "0xc3bc221de037908fd24c79602b731df848b12157";
const PT_HWHLP_YT: &str = "0x7a2c73b4dd114ada21674312160ab27a10ce9e4c";

const USDT0_ADDR: Address = address!("B8CE59FC3717ada4C02eaDF9682A9e934F625ebb");
const USDT0_WHALE: Address = address!("2222222222222222222222222222222222222222");

// ── ABIs ─────────────────────────────────────────────────────────────

sol! {
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

sol! {
    #[sol(rpc)]
    contract IPendleSY {
        function deposit(
            address receiver,
            address tokenIn,
            uint256 amountTokenToDeposit,
            uint256 minSharesOut
        ) external payable returns (uint256 amountSharesOut);
    }
}

sol! {
    #[sol(rpc)]
    contract IPendleRouter {
        function mintPyFromSy(
            address receiver,
            address YT,
            uint256 netSyIn,
            uint256 minPyOut
        ) external returns (uint256 netPyOut);
    }
}

// ── Test ──────────────────────────────────────────────────────────────

/// Full E2E: fund USDT0 → approve SY → deposit ERC20 into SY → approve router → mintPyFromSy → verify PT/YT
///
/// This is the exact flow the PendleYield venue executes for PT-hwHLP.
/// No swap needed — SY accepts USDT0 directly.
///
/// WARNING: Agents must use this test as a harness. Minimum deposit amounts vary
/// by SY contract (e.g. kHYPE SY requires ~100+ HYPE due to staking minimums).
/// Always check `previewDeposit` or test with small amounts before deploying.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_pendle_pt_mint_e2e() {
    // 1. Fork HyperEVM
    let ctx = spawn_fork(HYPEREVM_RPC, HYPEREVM_CHAIN_ID);

    // 2. Fund wallet with USDT0 (ERC20, 6 decimals) via whale
    let deposit_amount = U256::from(100u128 * 10u128.pow(6)); // 100 USDT0
    fund_erc20(&ctx.rpc_url, USDT0_ADDR, USDT0_WHALE, ctx.wallet_address, deposit_amount).await;
    println!("  Funded 100 USDT0 to test wallet");

    // Build provider with test wallet
    let signer: alloy::signers::local::PrivateKeySigner = ctx.private_key.parse().unwrap();
    let wallet = alloy::network::EthereumWallet::from(signer);
    let provider = alloy::providers::ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(ctx.rpc_url.parse().unwrap());

    let sy_addr: Address = PT_HWHLP_SY.parse().unwrap();
    let yt_addr: Address = PT_HWHLP_YT.parse().unwrap();
    let router_addr: Address = PENDLE_ROUTER.parse().unwrap();

    // 3. Approve USDT0 for SY contract
    println!("  Step 1: Approve USDT0 for SY");
    let usdt0 = IERC20::new(USDT0_ADDR, &provider);
    usdt0
        .approve(sy_addr, deposit_amount)
        .send().await.expect("USDT0 approve failed")
        .get_receipt().await.expect("USDT0 approve receipt");

    // 4. Deposit USDT0 into SY contract (ERC20 path, no msg.value)
    println!("  Step 2: Deposit USDT0 into SY");
    let sy = IPendleSY::new(sy_addr, &provider);
    let receipt = sy
        .deposit(ctx.wallet_address, USDT0_ADDR, deposit_amount, U256::ZERO)
        .send().await.expect("SY deposit failed")
        .get_receipt().await.expect("SY deposit receipt");
    println!("  SY deposit tx: {:?}", receipt.transaction_hash);

    // 5. Check SY balance
    let sy_erc20 = IERC20::new(sy_addr, &provider);
    let sy_bal = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf");
    println!("  SY balance after deposit: {}", sy_bal);
    assert!(sy_bal > U256::ZERO, "SY balance should be > 0 after deposit");

    // 6. Approve SY for Pendle router
    println!("  Step 3: Approve SY for router");
    sy_erc20
        .approve(router_addr, sy_bal)
        .send().await.expect("approve SY failed")
        .get_receipt().await.expect("approve SY receipt");

    // 7. Mint PT+YT from SY
    println!("  Step 4: mintPyFromSy");
    let router = IPendleRouter::new(router_addr, &provider);
    let mint_receipt = router
        .mintPyFromSy(ctx.wallet_address, yt_addr, sy_bal, U256::ZERO)
        .send().await.expect("mintPyFromSy failed")
        .get_receipt().await.expect("mintPyFromSy receipt");
    println!("  mintPyFromSy tx: {:?}", mint_receipt.transaction_hash);

    // 8. Verify YT balance > 0 (mintPyFromSy mints equal PT + YT)
    let yt_erc20 = IERC20::new(yt_addr, &provider);
    let yt_bal = yt_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("YT balanceOf");
    println!("  YT balance after mint: {}", yt_bal);
    assert!(yt_bal > U256::ZERO, "Should have YT tokens after mint");

    // 9. Verify SY was fully consumed
    let sy_bal_after = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf after");
    println!("  SY balance after mint: {} (should be 0)", sy_bal_after);
    assert_eq!(sy_bal_after, U256::ZERO, "SY should be fully consumed");

    // 10. Verify USDT0 was consumed
    let usdt0_bal_after = usdt0
        .balanceOf(ctx.wallet_address)
        .call().await.expect("USDT0 balanceOf after");
    println!("  USDT0 balance after: {} (should be 0)", usdt0_bal_after);

    println!("\n  === test_pendle_pt_mint_e2e PASSED ===");
    println!("  Flow: USDT0 → approve → SY deposit → mintPyFromSy → PT+YT");
}

/// Test: deposit USDT0 directly into SY (just the deposit step).
#[tokio::test]
#[ignore]
async fn test_pendle_sy_deposit_usdt0() {
    let ctx = spawn_fork(HYPEREVM_RPC, HYPEREVM_CHAIN_ID);

    let deposit_amount = U256::from(50u128 * 10u128.pow(6)); // 50 USDT0
    fund_erc20(&ctx.rpc_url, USDT0_ADDR, USDT0_WHALE, ctx.wallet_address, deposit_amount).await;
    println!("  Funded 50 USDT0");

    let signer: alloy::signers::local::PrivateKeySigner = ctx.private_key.parse().unwrap();
    let wallet = alloy::network::EthereumWallet::from(signer);
    let provider = alloy::providers::ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(ctx.rpc_url.parse().unwrap());

    let sy_addr: Address = PT_HWHLP_SY.parse().unwrap();

    // Approve + deposit
    let usdt0 = IERC20::new(USDT0_ADDR, &provider);
    usdt0
        .approve(sy_addr, deposit_amount)
        .send().await.expect("approve failed")
        .get_receipt().await.expect("approve receipt");

    let sy = IPendleSY::new(sy_addr, &provider);
    let receipt = sy
        .deposit(ctx.wallet_address, USDT0_ADDR, deposit_amount, U256::ZERO)
        .send().await.expect("SY deposit failed")
        .get_receipt().await.expect("SY deposit receipt");
    println!("  SY deposit tx: {:?}", receipt.transaction_hash);

    let sy_erc20 = IERC20::new(sy_addr, &provider);
    let sy_bal = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf");
    println!("  SY balance: {} (should be > 0)", sy_bal);
    assert!(sy_bal > U256::ZERO, "SY deposit should produce shares");

    println!("  test_pendle_sy_deposit_usdt0 PASSED");
}
