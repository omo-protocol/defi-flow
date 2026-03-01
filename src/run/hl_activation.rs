//! HyperLiquid wallet auto-activation.
//!
//! When a strategy needs HyperLiquid (perp or spot nodes), the wallet must have
//! been activated on HyperCore with a USDC deposit. This module handles that
//! automatically:
//!
//! 1. Query HL info API -> check if wallet has any account value
//! 2. LiFi cross-chain: USDT0 (HyperEVM) -> USDC (Arbitrum)
//! 3. Bridge2 on Arbitrum: EIP-2612 permit deposit -> funds land on HyperCore

use alloy::hex;
use alloy::primitives::{address, Address, Bytes, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::node::Node;
use crate::model::workflow::Workflow;
use crate::run::config::RuntimeConfig;
use crate::venues::evm;

// ── Constants ────────────────────────────────────────────────────────

const HYPEREVM_RPC: &str = "https://rpc.hyperliquid.xyz/evm";
const HYPEREVM_CHAIN_ID: u64 = 999;

const ARBITRUM_RPC: &str = "https://arb1.arbitrum.io/rpc";
const ARBITRUM_CHAIN_ID: u64 = 42161;

const LIFI_API_BASE: &str = "https://li.quest/v1";

// Token addresses
const USDT0_HYPEREVM: Address = address!("B8CE59FC3717ada4C02eaDF9682A9e934F625ebb");
const USDC_ARBITRUM: Address = address!("af88d065e77c8cC2239327C5EDb3A432268e5831");

// Protocol contracts
const BRIDGE2: Address = address!("2Df1c51E09aECF9cacB7bc98cB1742757f163dF7");

/// USDT0 to swap (11 USDT0 -> ~10.9 USDC after fees). 6 decimals.
const SWAP_AMOUNT_USDT0: u64 = 11_000_000;

/// Minimum USDC for activation (HL min is 5 USDC). 6 decimals.
const MIN_USDC_DEPOSIT: u64 = 5_000_000;

// ── ABIs ─────────────────────────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IUSDCPermit {
        function nonces(address owner) external view returns (uint256);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IBridge2 {
        struct Signature {
            uint256 r;
            uint256 s;
            uint8 v;
        }

        struct DepositWithPermit {
            address user;
            uint64 usd;
            uint64 deadline;
            Signature signature;
        }

        function batchedDepositWithPermit(DepositWithPermit[] memory deposits) external;
    }
}

// EIP-2612 Permit struct for signing
sol! {
    #[derive(Debug)]
    struct Permit {
        address owner;
        address spender;
        uint256 value;
        uint256 nonce;
        uint256 deadline;
    }
}

// ── LiFi API types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LiFiQuote {
    estimate: LiFiEstimate,
    #[serde(rename = "transactionRequest")]
    transaction_request: Option<LiFiTxRequest>,
}

#[derive(Debug, Deserialize)]
struct LiFiEstimate {
    #[serde(rename = "toAmount")]
    to_amount: String,
    #[serde(rename = "approvalAddress")]
    approval_address: Option<String>,
    #[serde(rename = "executionDuration")]
    execution_duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LiFiTxRequest {
    to: String,
    data: String,
    value: String,
    #[serde(rename = "gasLimit")]
    gas_limit: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_u256_maybe_hex(s: &str) -> U256 {
    if let Some(hex_str) = s.strip_prefix("0x") {
        U256::from_str_radix(hex_str, 16).unwrap_or(U256::ZERO)
    } else {
        s.parse().unwrap_or(U256::ZERO)
    }
}

fn parse_u64_maybe_hex(s: &str) -> Option<u64> {
    s.parse().ok().or_else(|| {
        s.strip_prefix("0x")
            .and_then(|h| u64::from_str_radix(h, 16).ok())
    })
}

// ── Logic ────────────────────────────────────────────────────────────

/// Returns true if the workflow has any HyperLiquid-dependent nodes.
fn needs_hl(workflow: &Workflow) -> bool {
    workflow
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Perp { .. } | Node::Spot { .. }))
}

/// Ensure the wallet is activated on HyperLiquid exchange.
///
/// Flow:
/// 1. Check if wallet already active on HyperCore
/// 2. LiFi cross-chain swap: USDT0 (HyperEVM) -> USDC (Arbitrum)
/// 3. Wait for USDC to arrive on Arbitrum
/// 4. Bridge2 deposit: USDC (Arbitrum) -> HyperCore via EIP-2612 permit
///
/// Returns `Ok(true)` if activation was performed, `Ok(false)` if already active.
pub async fn ensure_hl_wallet(
    workflow: &Workflow,
    config: &RuntimeConfig,
) -> Result<bool> {
    if !needs_hl(workflow) || config.dry_run {
        return Ok(false);
    }

    // Check if wallet already has funds on HyperCore
    let info = ferrofluid::InfoProvider::new(config.network);
    match info.user_state(config.wallet_address).await {
        Ok(state) => {
            let account_value: f64 = state
                .margin_summary
                .account_value
                .parse()
                .unwrap_or(0.0);
            if account_value > 1.0 {
                return Ok(false); // Already active
            }
        }
        Err(e) => {
            let err_str = format!("{e}");
            if !err_str.contains("does not exist") {
                eprintln!("[hl-activate] WARNING: user_state query failed: {err_str}");
            }
        }
    }

    println!("[hl-activate] Wallet has no funds on HyperLiquid — activating...");

    let signer: alloy::signers::local::PrivateKeySigner = config
        .private_key
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;

    // Build Arbitrum provider to check existing USDC
    let arb_wallet = alloy::network::EthereumWallet::from(signer.clone());
    let arb_provider = ProviderBuilder::new()
        .wallet(arb_wallet)
        .connect_http(ARBITRUM_RPC.parse()?);

    let usdc_arb = IERC20::new(USDC_ARBITRUM, &arb_provider);
    let existing_usdc = usdc_arb
        .balanceOf(config.wallet_address)
        .call()
        .await
        .unwrap_or(U256::ZERO);

    let need_swap = existing_usdc < U256::from(MIN_USDC_DEPOSIT);

    if need_swap {
        // ── Step 1: LiFi cross-chain USDT0 (HyperEVM) -> USDC (Arbitrum) ──

        let hyperevm_wallet = alloy::network::EthereumWallet::from(signer.clone());
        let hyperevm_provider = ProviderBuilder::new()
            .wallet(hyperevm_wallet)
            .connect_http(HYPEREVM_RPC.parse()?);

        lifi_swap_usdt0_to_usdc(&hyperevm_provider, config.wallet_address).await?;

        // ── Step 2: Wait for USDC to arrive on Arbitrum ──

        println!("[hl-activate] Waiting for USDC on Arbitrum...");
        let poll_timeout = std::time::Duration::from_secs(180);
        let poll_start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(10);

        loop {
            tokio::time::sleep(poll_interval).await;
            let bal = usdc_arb
                .balanceOf(config.wallet_address)
                .call()
                .await
                .unwrap_or(U256::ZERO);

            if bal > existing_usdc {
                let received = bal - existing_usdc;
                println!(
                    "[hl-activate] USDC arrived on Arbitrum: {:.2}",
                    evm::from_token_units(received, 6),
                );
                break;
            }

            if poll_start.elapsed() > poll_timeout {
                anyhow::bail!(
                    "[hl-activate] Timeout ({:.0}s) waiting for USDC on Arbitrum",
                    poll_start.elapsed().as_secs_f64(),
                );
            }

            println!(
                "[hl-activate]   polling... ({:.0}s elapsed)",
                poll_start.elapsed().as_secs_f64(),
            );
        }
    } else {
        println!(
            "[hl-activate] Already have {:.2} USDC on Arbitrum, skipping LiFi swap",
            evm::from_token_units(existing_usdc, 6),
        );
    }

    // Re-check USDC balance on Arbitrum
    let usdc_balance = usdc_arb
        .balanceOf(config.wallet_address)
        .call()
        .await
        .context("USDC balanceOf on Arbitrum")?;

    if usdc_balance < U256::from(MIN_USDC_DEPOSIT) {
        anyhow::bail!(
            "[hl-activate] USDC on Arbitrum too low: {:.2} (need >= 5)",
            evm::from_token_units(usdc_balance, 6),
        );
    }

    // ── Step 3: Check Arbitrum ETH for gas ──

    use alloy::providers::Provider;
    let eth_balance = arb_provider
        .get_balance(config.wallet_address)
        .await
        .unwrap_or(U256::ZERO);
    let eth_f = evm::from_token_units(eth_balance, 18);

    if eth_f < 0.0001 {
        anyhow::bail!(
            "[hl-activate] Need ETH on Arbitrum for Bridge2 gas but have {:.6} ETH. \
             Send ~0.001 ETH to {:?} on Arbitrum.",
            eth_f,
            config.wallet_address,
        );
    }

    // ── Step 4: Bridge2 deposit via EIP-2612 permit ──

    bridge2_deposit(&arb_provider, &signer, config.wallet_address, usdc_balance).await?;

    // ── Step 5: Verify activation ──

    println!("[hl-activate] Waiting for HyperCore to process deposit...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    match info.user_state(config.wallet_address).await {
        Ok(state) => {
            let val: f64 = state
                .margin_summary
                .account_value
                .parse()
                .unwrap_or(0.0);
            println!("[hl-activate] Wallet activated! Account value: ${:.2}", val);
        }
        Err(e) => {
            eprintln!(
                "[hl-activate] WARNING: verification failed ({e}), deposit may still be processing"
            );
        }
    }

    Ok(true)
}

/// Get LiFi quote and execute cross-chain swap: USDT0 (HyperEVM) -> USDC (Arbitrum).
async fn lifi_swap_usdt0_to_usdc<P: alloy::providers::Provider + Clone>(
    provider: &P,
    wallet_address: Address,
) -> Result<()> {
    let usdt0 = IERC20::new(USDT0_HYPEREVM, provider);
    let usdt0_bal = usdt0
        .balanceOf(wallet_address)
        .call()
        .await
        .context("USDT0 balanceOf")?;

    let swap_amount = U256::from(SWAP_AMOUNT_USDT0);
    if usdt0_bal < swap_amount {
        anyhow::bail!(
            "[hl-activate] Need {} USDT0 on HyperEVM but have {}",
            evm::from_token_units(swap_amount, 6),
            evm::from_token_units(usdt0_bal, 6),
        );
    }

    // Get LiFi quote
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("defi-flow/0.1")
        .build()?;

    let url = format!(
        "{LIFI_API_BASE}/quote?\
         fromChain={HYPEREVM_CHAIN_ID}&\
         toChain={ARBITRUM_CHAIN_ID}&\
         fromToken={USDT0_HYPEREVM:?}&\
         toToken={USDC_ARBITRUM:?}&\
         fromAmount={SWAP_AMOUNT_USDT0}&\
         fromAddress={wallet_address:?}&\
         slippage=0.05"
    );

    println!(
        "[hl-activate] Getting LiFi quote: {} USDT0 (HyperEVM) -> USDC (Arbitrum)...",
        evm::from_token_units(swap_amount, 6),
    );

    let resp = client.get(&url).send().await.context("LiFi quote request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("[hl-activate] LiFi API error {status}: {body}");
    }

    let quote: LiFiQuote = resp.json().await.context("parsing LiFi quote")?;
    let to_amount: f64 = quote.estimate.to_amount.parse().unwrap_or(0.0);
    let to_usdc = to_amount / 1e6;

    println!(
        "[hl-activate] LiFi: {} USDT0 -> {:.2} USDC (est. {:.0}s)",
        evm::from_token_units(swap_amount, 6),
        to_usdc,
        quote.estimate.execution_duration.unwrap_or(0.0),
    );

    if to_usdc < 5.0 {
        anyhow::bail!(
            "[hl-activate] LiFi output {:.2} USDC < 5 minimum",
            to_usdc,
        );
    }

    let tx_req = quote
        .transaction_request
        .ok_or_else(|| anyhow::anyhow!("[hl-activate] LiFi missing transactionRequest"))?;

    // Approve USDT0 for LiFi's contract
    if let Some(ref approval_addr) = quote.estimate.approval_address {
        let spender: Address = approval_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("bad approval address: {e}"))?;
        println!(
            "[hl-activate] Approving USDT0 for LiFi ({})...",
            evm::short_addr(&spender),
        );
        usdt0
            .approve(spender, swap_amount)
            .gas(200_000)
            .send()
            .await
            .context("approve USDT0 for LiFi")?
            .get_receipt()
            .await?;
    }

    // Execute LiFi tx on HyperEVM
    let to_addr: Address = tx_req
        .to
        .parse()
        .map_err(|e| anyhow::anyhow!("bad tx.to: {e}"))?;
    let data = Bytes::from(hex::decode(tx_req.data.trim_start_matches("0x"))?);
    let value = parse_u256_maybe_hex(&tx_req.value);
    let gas_limit: u64 = tx_req
        .gas_limit
        .as_ref()
        .and_then(|s| parse_u64_maybe_hex(s))
        .unwrap_or(500_000);

    println!("[hl-activate] Submitting LiFi cross-chain tx on HyperEVM...");

    use alloy::network::TransactionBuilder;
    let tx = alloy::rpc::types::TransactionRequest::default()
        .with_to(to_addr)
        .with_input(data)
        .with_value(value)
        .with_gas_limit(gas_limit);

    let receipt = provider
        .send_transaction(tx)
        .await
        .context("LiFi tx send")?
        .get_receipt()
        .await
        .context("LiFi tx receipt")?;

    println!("[hl-activate] LiFi tx: {:?}", receipt.transaction_hash);
    Ok(())
}

/// Deposit USDC to HyperLiquid via Bridge2 on Arbitrum.
///
/// Bridge2's `batchedDepositWithPermit` tries EIP-2612 permit first, and if it
/// fails (emitting `FailedPermitDeposit`), falls back to `transferFrom`. We
/// pre-approve USDC so the fallback always works.
async fn bridge2_deposit<P: alloy::providers::Provider + Clone>(
    provider: &P,
    signer: &alloy::signers::local::PrivateKeySigner,
    wallet_address: Address,
    amount: U256,
) -> Result<()> {
    let deposit_u64 = amount.to::<u64>();

    // Pre-approve USDC so transferFrom works even if permit fails
    let usdc = IERC20::new(USDC_ARBITRUM, provider);
    usdc.approve(BRIDGE2, amount)
        .gas(100_000)
        .send()
        .await
        .context("approve USDC for Bridge2")?
        .get_receipt()
        .await
        .context("approve USDC receipt")?;

    // Get permit nonce from USDC contract
    let usdc_permit = IUSDCPermit::new(USDC_ARBITRUM, provider);
    let nonce = usdc_permit
        .nonces(wallet_address)
        .call()
        .await
        .context("USDC nonces")?;

    let deadline_ts = chrono::Utc::now().timestamp() as u64 + 3600; // 1 hour

    // Build EIP-712 domain for USDC on Arbitrum
    let domain = alloy::sol_types::eip712_domain! {
        name: "USD Coin",
        version: "2",
        chain_id: ARBITRUM_CHAIN_ID,
        verifying_contract: USDC_ARBITRUM,
    };

    let permit = Permit {
        owner: wallet_address,
        spender: BRIDGE2,
        value: amount,
        nonce,
        deadline: U256::from(deadline_ts),
    };

    // Compute EIP-712 signing hash and sign
    use alloy::signers::Signer;
    use alloy::sol_types::SolStruct;
    let signing_hash = permit.eip712_signing_hash(&domain);
    let sig = signer
        .sign_hash(&signing_hash)
        .await
        .map_err(|e| anyhow::anyhow!("EIP-2612 permit sign failed: {e}"))?;

    println!(
        "[hl-activate] Depositing {:.2} USDC to HyperLiquid via Bridge2...",
        evm::from_token_units(amount, 6),
    );

    let bridge2 = IBridge2::new(BRIDGE2, provider);
    let deposit = IBridge2::DepositWithPermit {
        user: wallet_address,
        usd: deposit_u64,
        deadline: deadline_ts,
        signature: IBridge2::Signature {
            r: sig.r(),
            s: sig.s(),
            v: if sig.v() { 28 } else { 27 },
        },
    };

    let receipt = bridge2
        .batchedDepositWithPermit(vec![deposit])
        .gas(300_000)
        .send()
        .await
        .context("Bridge2 batchedDepositWithPermit")?
        .get_receipt()
        .await
        .context("Bridge2 receipt")?;

    println!(
        "[hl-activate] Bridge2 tx: {:?} ({:.2} USDC -> wallet {})",
        receipt.transaction_hash,
        evm::from_token_units(amount, 6),
        evm::short_addr(&wallet_address),
    );
    Ok(())
}
