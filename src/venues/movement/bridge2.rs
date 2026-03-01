//! Bridge2 movement venue — deposit USDC from Arbitrum to HyperCore.
//!
//! Bridge2 contract on Arbitrum (`0x2Df1c51E09aECF9cacB7bc98cB1742757f163dF7`)
//! uses `batchedDepositWithPermit()` which tries EIP-2612 permit, then falls
//! back to `transferFrom`. We pre-approve USDC so the fallback always works.

use alloy::primitives::{address, Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::node::Node;
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

// ── Constants ────────────────────────────────────────────────────────

const ARBITRUM_RPC: &str = "https://arb1.arbitrum.io/rpc";
const USDC_ARBITRUM: Address = address!("af88d065e77c8cC2239327C5EDb3A432268e5831");
const BRIDGE2: Address = address!("2Df1c51E09aECF9cacB7bc98cB1742757f163dF7");

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

// ── Venue implementation ─────────────────────────────────────────────

pub struct Bridge2Movement {
    signer: PrivateKeySigner,
    wallet_address: Address,
    dry_run: bool,
    metrics: SimMetrics,
}

impl Bridge2Movement {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;

        Ok(Bridge2Movement {
            signer,
            wallet_address: config.wallet_address,
            dry_run: config.dry_run,
            metrics: SimMetrics::default(),
        })
    }

    /// Check if there's stranded USDC on Arbitrum from a previous partial execution.
    async fn check_stranded_usdc(&self) -> f64 {
        let Ok(rp) = evm::read_provider(ARBITRUM_RPC) else {
            return 0.0;
        };
        let usdc = IERC20::new(USDC_ARBITRUM, &rp);
        match usdc.balanceOf(self.wallet_address).call().await {
            Ok(balance) => evm::from_token_units(balance, 6),
            Err(_) => 0.0,
        }
    }

    /// Execute USDC deposit from Arbitrum to HyperCore via Bridge2.
    async fn deposit(&self, amount_usd: f64) -> Result<f64> {
        let wallet = alloy::network::EthereumWallet::from(self.signer.clone());
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(ARBITRUM_RPC.parse()?);

        // Convert USD amount to 6-decimal USDC units
        let amount_raw = U256::from((amount_usd * 1e6) as u64);

        // Pre-approve USDC for Bridge2 (fallback for permit failure)
        let usdc = IERC20::new(USDC_ARBITRUM, &provider);
        usdc.approve(BRIDGE2, amount_raw)
            .gas(100_000)
            .send()
            .await
            .context("approve USDC for Bridge2")?
            .get_receipt()
            .await
            .context("approve USDC receipt")?;

        // Build EIP-2612 permit signature
        let usdc_permit = IUSDCPermit::new(USDC_ARBITRUM, &provider);
        let nonce = usdc_permit
            .nonces(self.wallet_address)
            .call()
            .await
            .context("USDC nonces")?;

        let deadline_ts = chrono::Utc::now().timestamp() as u64 + 3600;

        let domain = alloy::sol_types::eip712_domain! {
            name: "USD Coin",
            version: "2",
            chain_id: 42161u64,
            verifying_contract: USDC_ARBITRUM,
        };

        let permit = Permit {
            owner: self.wallet_address,
            spender: BRIDGE2,
            value: amount_raw,
            nonce,
            deadline: U256::from(deadline_ts),
        };

        use alloy::signers::Signer;
        use alloy::sol_types::SolStruct;
        let signing_hash = permit.eip712_signing_hash(&domain);
        let sig = self
            .signer
            .sign_hash(&signing_hash)
            .await
            .map_err(|e| anyhow::anyhow!("permit sign failed: {e}"))?;

        println!(
            "  Bridge2: depositing {:.2} USDC to HyperCore...",
            amount_usd,
        );

        // Call batchedDepositWithPermit
        let bridge2 = IBridge2::new(BRIDGE2, &provider);
        let deposit = IBridge2::DepositWithPermit {
            user: self.wallet_address,
            usd: amount_raw.to::<u64>(),
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
            "  Bridge2: tx {:?} ({:.2} USDC → {})",
            receipt.transaction_hash,
            amount_usd,
            evm::short_addr(&self.wallet_address),
        );

        Ok(amount_usd)
    }
}

#[async_trait]
impl Venue for Bridge2Movement {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Movement { .. } => {
                // Recovery: check for stranded USDC on Arbitrum from a previous
                // partial execution (upstream LiFi succeeded but Bridge2 failed).
                let effective_amount = if input_amount <= 0.0 {
                    let stranded = self.check_stranded_usdc().await;
                    if stranded > 0.50 {
                        println!(
                            "  Bridge2: [recovery] found {:.2} stranded USDC on Arbitrum",
                            stranded
                        );
                        stranded
                    } else {
                        return Ok(ExecutionResult::Noop);
                    }
                } else {
                    input_amount
                };

                if self.dry_run {
                    println!(
                        "  Bridge2: [DRY RUN] would deposit {:.2} USDC to HyperCore",
                        effective_amount
                    );
                    return Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: effective_amount,
                    });
                }

                let deposited = self.deposit(effective_amount).await?;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: deposited,
                })
            }
            _ => Ok(ExecutionResult::Noop),
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0) // pass-through venue
    }

    async fn unwind(&mut self, _fraction: f64) -> Result<f64> {
        Ok(0.0) // pass-through venue
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        self.metrics.clone()
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        None // pass-through venue
    }
}
