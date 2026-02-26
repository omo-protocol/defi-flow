use std::collections::HashMap;

use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use ferrofluid::{ExchangeProvider, InfoProvider, Network};

use crate::model::node::Node;
use crate::run::config::RuntimeConfig;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

/// System address prefix byte for HyperCore spot tokens.
/// System address = 0x20 + 00..00 + token_index (big-endian).
/// Exception: HYPE uses 0x2222222222222222222222222222222222222222.
const HYPE_SYSTEM_ADDRESS: &str = "0x2222222222222222222222222222222222222222";

/// Token metadata resolved from spot_meta at construction time.
#[derive(Debug, Clone)]
struct TokenInfo {
    /// The token index on HyperCore.
    index: u32,
    /// System address for HyperCore ↔ HyperEVM transfers.
    system_address: Address,
    /// Token's wei decimals on HyperCore.
    wei_decimals: u32,
}

/// Native HyperCore ↔ HyperEVM spot transfers via Hyperliquid's `spotSend` action.
///
/// - **HyperCore → HyperEVM**: `spotSend` to the token's system address.
/// - **HyperEVM → HyperCore**: ERC20 `transfer()` to the system address on EVM.
pub struct HyperliquidNativeMovement {
    exchange: ExchangeProvider<PrivateKeySigner>,
    info: InfoProvider,
    wallet_address: Address,
    dry_run: bool,
    /// symbol (uppercase) → token info
    token_map: HashMap<String, TokenInfo>,
    metrics: SimMetrics,
}

impl HyperliquidNativeMovement {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;

        let (exchange, info) = match config.network {
            Network::Mainnet => (
                ExchangeProvider::mainnet(signer),
                InfoProvider::mainnet(),
            ),
            Network::Testnet => (
                ExchangeProvider::testnet(signer),
                InfoProvider::testnet(),
            ),
        };

        Ok(HyperliquidNativeMovement {
            exchange,
            info,
            wallet_address: config.wallet_address,
            dry_run: config.dry_run,
            token_map: HashMap::new(),
            metrics: SimMetrics::default(),
        })
    }

    /// Fetch spot metadata from Hyperliquid and build the symbol → system address map.
    pub async fn init_metadata(&mut self) -> Result<()> {
        let meta = self
            .info
            .spot_meta()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch spot meta: {e}"))?;

        for token in &meta.tokens {
            let system_address = if token.name.eq_ignore_ascii_case("HYPE") {
                HYPE_SYSTEM_ADDRESS
                    .parse::<Address>()
                    .expect("hardcoded HYPE system address")
            } else {
                token_index_to_system_address(token.index)
            };

            self.token_map.insert(
                token.name.to_uppercase(),
                TokenInfo {
                    index: token.index,
                    system_address,
                    wei_decimals: token.wei_decimals,
                },
            );
        }

        println!(
            "  HyperliquidNative: loaded {} spot tokens",
            self.token_map.len()
        );

        Ok(())
    }

    /// Transfer spot tokens from HyperCore → HyperEVM via spotSend to system address.
    async fn core_to_evm(
        &mut self,
        token: &str,
        amount: f64,
    ) -> Result<ExecutionResult> {
        let info = self
            .token_map
            .get(&token.to_uppercase())
            .ok_or_else(|| anyhow::anyhow!("Token '{token}' not found in HyperCore spot meta"))?
            .clone();

        let amount_raw = (amount * 10f64.powi(info.wei_decimals as i32)) as u128;
        let amount_str = amount_raw.to_string();

        println!(
            "  HyperliquidNative: Core→EVM {:.6} {} (system_addr: {:?})",
            amount, token, info.system_address
        );

        if self.dry_run {
            println!("  HyperliquidNative: [DRY RUN] spotSend would be executed");
            return Ok(ExecutionResult::TokenOutput {
                token: token.to_string(),
                amount,
            });
        }

        let result = self
            .exchange
            .spot_transfer(info.system_address, token.to_string(), &amount_str)
            .await
            .context("spotSend failed")?;

        match result {
            ferrofluid::types::responses::ExchangeResponseStatus::Ok(_) => {
                println!("  HyperliquidNative: spotSend OK");
                Ok(ExecutionResult::TokenOutput {
                    token: token.to_string(),
                    amount,
                })
            }
            ferrofluid::types::responses::ExchangeResponseStatus::Err(e) => {
                bail!("spotSend error: {e}")
            }
        }
    }

    /// Transfer spot tokens from HyperEVM → HyperCore via ERC20 transfer to system address.
    async fn evm_to_core(
        &mut self,
        _token: &str,
        _amount: f64,
    ) -> Result<ExecutionResult> {
        // TODO: ERC20 transfer to system address on HyperEVM.
        // Requires: alloy provider + signer with HyperEVM RPC,
        // resolve token symbol → ERC20 address from evm_contract in spot_meta,
        // then call transfer(system_address, amount).
        bail!(
            "HyperEVM → HyperCore transfers not yet implemented. \
             Use the Hyperliquid frontend for now."
        )
    }
}

#[async_trait]
impl Venue for HyperliquidNativeMovement {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Movement {
                from_chain,
                to_chain,
                from_token,
                ..
            } => {
                if input_amount <= 0.0 {
                    return Ok(ExecutionResult::Noop);
                }

                // Initialize metadata on first use
                if self.token_map.is_empty() {
                    self.init_metadata().await?;
                }

                let fc = from_chain
                    .as_ref()
                    .map(|c| c.name.to_lowercase())
                    .unwrap_or_default();
                let tc = to_chain
                    .as_ref()
                    .map(|c| c.name.to_lowercase())
                    .unwrap_or_default();

                match (fc.as_str(), tc.as_str()) {
                    ("hyperliquid", "hyperevm") => {
                        self.core_to_evm(from_token, input_amount).await
                    }
                    ("hyperevm", "hyperliquid") => {
                        self.evm_to_core(from_token, input_amount).await
                    }
                    _ => bail!(
                        "HyperliquidNative only supports hyperliquid↔hyperevm, got {fc}→{tc}"
                    ),
                }
            }
            _ => Ok(ExecutionResult::Noop),
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0)
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
}

/// Compute the system address for a token index.
/// System address = first byte 0x20, remaining bytes all zeros except token index in big-endian.
fn token_index_to_system_address(index: u32) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0x20;
    // Encode token index in big-endian in the last 4 bytes
    let index_bytes = index.to_be_bytes();
    bytes[16..20].copy_from_slice(&index_bytes);
    Address::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_address() {
        // Token index 0 → 0x2000000000000000000000000000000000000000
        let addr = token_index_to_system_address(0);
        assert_eq!(
            format!("{addr:?}"),
            "0x2000000000000000000000000000000000000000"
        );

        // Token index 200 → 0x20000000000000000000000000000000000000c8
        let addr = token_index_to_system_address(200);
        assert_eq!(
            format!("{addr:?}"),
            "0x20000000000000000000000000000000000000c8"
        );

        // Token index 1 → 0x2000000000000000000000000000000000000001
        let addr = token_index_to_system_address(1);
        assert_eq!(
            format!("{addr:?}"),
            "0x2000000000000000000000000000000000000001"
        );
    }
}
