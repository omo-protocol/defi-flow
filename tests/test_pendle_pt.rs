mod anvil_common;

use alloy::primitives::{Address, U256};
use alloy::sol;

use anvil_common::*;

// ── Constants ────────────────────────────────────────────────────────

const HYPEREVM_RPC: &str = "https://rpc.hyperliquid.xyz/evm";
const HYPEREVM_CHAIN_ID: u64 = 999;

// PT-kHYPE contracts
const PENDLE_ROUTER: &str = "0x888888888889758F76e7103c6CbF23ABbF58F946";
const PT_KHYPE_SY: &str = "0x57fc55dff8ceca86ee94a6bf255af2f0ed90eb9e";
const PT_KHYPE_YT: &str = "0x8e8df024cf6d3e916be0821ff3177db6981fcad2";
const PT_KHYPE_MARKET: &str = "0x31104779b2a07a273d6c662419377773083d0b2e";

const WHYPE: &str = "0x5555555555555555555555555555555555555555";

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
    contract IWETH9 {
        function withdraw(uint256 amount) external;
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

/// Full E2E: fund HYPE → wrap to WHYPE (simulating LiFi swap output) →
/// unwrap → deposit native HYPE into SY → approve → mintPyFromSy → verify PT/YT
///
/// This is the exact flow the PendleYield venue executes.
#[tokio::test]
#[ignore] // Requires Anvil + network access
async fn test_pendle_pt_mint_e2e() {
    // 1. Fork HyperEVM
    let ctx = spawn_fork(HYPEREVM_RPC, HYPEREVM_CHAIN_ID);

    // 2. Fund wallet with native HYPE (200 HYPE — kHYPE has min stake)
    let hype_amount = U256::from(500u128 * 10u128.pow(18));
    fund_eth(&ctx.rpc_url, ctx.wallet_address, hype_amount).await;
    println!("  Funded 500 HYPE to test wallet");

    // 3. Wrap 200 HYPE → WHYPE (simulating what LiFi swap outputs)
    let whype_addr: Address = WHYPE.parse().unwrap();
    let mint_amount = U256::from(200u128 * 10u128.pow(18));
    wrap_eth(&ctx.rpc_url, &ctx.private_key, whype_addr, mint_amount).await;

    let whype_bal = balance_of(&ctx.rpc_url, whype_addr, ctx.wallet_address).await;
    println!("  WHYPE balance: {} (should be 200e18)", whype_bal);
    assert!(whype_bal >= mint_amount, "WHYPE wrap failed");

    // Build provider with test wallet
    let signer: alloy::signers::local::PrivateKeySigner = ctx.private_key.parse().unwrap();
    let wallet = alloy::network::EthereumWallet::from(signer);
    let provider = alloy::providers::ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(ctx.rpc_url.parse().unwrap());

    let sy_addr: Address = PT_KHYPE_SY.parse().unwrap();
    let yt_addr: Address = PT_KHYPE_YT.parse().unwrap();
    let router_addr: Address = PENDLE_ROUTER.parse().unwrap();
    let deposit_amount = U256::from(100u128 * 10u128.pow(18)); // 100 HYPE

    // 4. Unwrap WHYPE → native HYPE (venue does this for HYPE input)
    println!("  Step 1: Unwrap WHYPE → native HYPE");
    let whype = IWETH9::new(whype_addr, &provider);
    whype
        .withdraw(deposit_amount)
        .send().await.expect("WHYPE withdraw failed")
        .get_receipt().await.expect("WHYPE withdraw receipt");
    println!("  Unwrapped 1 WHYPE → 1 native HYPE");

    // 5. Deposit native HYPE into SY contract
    println!("  Step 2: Deposit native HYPE into SY");
    let sy = IPendleSY::new(sy_addr, &provider);
    let receipt = sy
        .deposit(ctx.wallet_address, Address::ZERO, deposit_amount, U256::ZERO)
        .value(deposit_amount)
        .send().await.expect("SY deposit failed")
        .get_receipt().await.expect("SY deposit receipt");
    println!("  SY deposit tx: {:?}", receipt.transaction_hash);

    // 6. Check SY balance
    let sy_erc20 = IERC20::new(sy_addr, &provider);
    let sy_bal = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf");
    println!("  SY balance after deposit: {}", sy_bal);
    assert!(sy_bal > U256::ZERO, "SY balance should be > 0 after deposit");

    // 7. Approve SY for Pendle router
    println!("  Step 3: Approve SY for router");
    sy_erc20
        .approve(router_addr, sy_bal)
        .send().await.expect("approve SY failed")
        .get_receipt().await.expect("approve SY receipt");

    // 8. Mint PT+YT from SY
    println!("  Step 4: mintPyFromSy");
    let router = IPendleRouter::new(router_addr, &provider);
    let mint_receipt = router
        .mintPyFromSy(ctx.wallet_address, yt_addr, sy_bal, U256::ZERO)
        .send().await.expect("mintPyFromSy failed")
        .get_receipt().await.expect("mintPyFromSy receipt");
    println!("  mintPyFromSy tx: {:?}", mint_receipt.transaction_hash);

    // 9. Verify YT balance > 0 (mintPyFromSy mints equal PT + YT)
    let yt_erc20 = IERC20::new(yt_addr, &provider);
    let yt_bal = yt_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("YT balanceOf");
    println!("  YT balance after mint: {}", yt_bal);
    assert!(yt_bal > U256::ZERO, "Should have YT tokens after mint");

    // 10. Verify SY was fully consumed
    let sy_bal_after = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf after");
    println!("  SY balance after mint: {} (should be 0)", sy_bal_after);
    assert_eq!(sy_bal_after, U256::ZERO, "SY should be fully consumed");

    println!("\n  === test_pendle_pt_mint_e2e PASSED ===");
    println!("  Flow: WHYPE → unwrap → native HYPE → SY deposit → mintPyFromSy → PT+YT");
}

/// Test: deposit native HYPE directly into SY (no WHYPE unwrap step).
/// This is what happens when the venue receives native HYPE from LiFi.
#[tokio::test]
#[ignore]
async fn test_pendle_sy_deposit_native_hype() {
    let ctx = spawn_fork(HYPEREVM_RPC, HYPEREVM_CHAIN_ID);

    // Fund with native HYPE only (200 — kHYPE has min stake)
    let hype_amount = U256::from(500u128 * 10u128.pow(18));
    fund_eth(&ctx.rpc_url, ctx.wallet_address, hype_amount).await;
    println!("  Funded 500 HYPE (native)");

    let signer: alloy::signers::local::PrivateKeySigner = ctx.private_key.parse().unwrap();
    let wallet = alloy::network::EthereumWallet::from(signer);
    let provider = alloy::providers::ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(ctx.rpc_url.parse().unwrap());

    let sy_addr: Address = PT_KHYPE_SY.parse().unwrap();
    let deposit_amount = U256::from(200u128 * 10u128.pow(18)); // 200 HYPE (kHYPE has min stake)

    // Deposit native HYPE into SY directly
    let sy = IPendleSY::new(sy_addr, &provider);
    let receipt = sy
        .deposit(ctx.wallet_address, Address::ZERO, deposit_amount, U256::ZERO)
        .value(deposit_amount)
        .send().await.expect("SY deposit failed")
        .get_receipt().await.expect("SY deposit receipt");
    println!("  SY deposit tx: {:?}", receipt.transaction_hash);

    let sy_erc20 = IERC20::new(sy_addr, &provider);
    let sy_bal = sy_erc20
        .balanceOf(ctx.wallet_address)
        .call().await.expect("SY balanceOf");
    println!("  SY balance: {} (should be > 0)", sy_bal);
    assert!(sy_bal > U256::ZERO, "SY deposit should produce shares");

    println!("  test_pendle_sy_deposit_native_hype PASSED");
}
