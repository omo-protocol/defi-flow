use alloy::hex;
use alloy::primitives::{Bytes, U256};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use crate::model::chain::Chain;
use crate::model::node::Node;
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

const LIFI_API_BASE: &str = "https://li.quest/v1";

// ── LiFi API response types ───────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QuoteResponse {
    estimate: QuoteEstimate,
    #[serde(rename = "transactionRequest")]
    transaction_request: Option<TransactionRequest>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct QuoteEstimate {
    #[serde(rename = "toAmount")]
    to_amount: String,
    #[serde(rename = "toAmountMin")]
    to_amount_min: Option<String>,
    #[serde(rename = "approvalAddress")]
    approval_address: Option<String>,
    #[serde(rename = "executionDuration")]
    execution_duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TransactionRequest {
    to: String,
    data: String,
    value: String,
    #[serde(rename = "gasLimit")]
    gas_limit: Option<String>,
    #[serde(rename = "gasPrice")]
    gas_price: Option<String>,
}

// ── LiFi Movement ─────────────────────────────────────────────────

pub struct LiFiMovement {
    client: reqwest::Client,
    wallet_address: String,
    private_key: String,
    dry_run: bool,
    tokens: evm::TokenManifest,
    slippage_bps: f64,
    metrics: SimMetrics,
    decimals_cache: std::collections::HashMap<(String, String), u8>,
}

impl LiFiMovement {
    pub fn new(config: &RuntimeConfig, tokens: &evm::TokenManifest) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("defi-flow/0.1")
            .build()
            .context("creating LiFi HTTP client")?;

        Ok(LiFiMovement {
            client,
            wallet_address: format!("{:?}", config.wallet_address),
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            slippage_bps: config.slippage_bps,
            metrics: SimMetrics::default(),
            decimals_cache: std::collections::HashMap::new(),
        })
    }

    async fn query_decimals(&mut self, token: &str, chain: &Chain) -> Result<u8> {
        let key = (token.to_string(), chain.to_string());
        if let Some(&d) = self.decimals_cache.get(&key) {
            return Ok(d);
        }
        let addr = evm::resolve_token(&self.tokens, chain, token)
            .with_context(|| format!("token '{token}' not in manifest for {chain}"))?;
        let rpc = chain.rpc_url().context("chain requires rpc_url for decimals query")?;
        let d = evm::query_decimals(rpc, addr).await
            .with_context(|| format!("decimals() call failed for {token} on {chain}"))?;
        self.decimals_cache.insert(key, d);
        Ok(d)
    }

    /// Submit the LiFi transactionRequest on-chain: approve input token then send tx.
    async fn submit_lifi_tx(
        &self,
        chain: &Chain,
        from_token: &str,
        amount_raw: u128,
        quote: &QuoteResponse,
    ) -> Result<()> {
        let rpc = chain.rpc_url().context("chain requires rpc_url")?;
        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = alloy::providers::ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(rpc.parse().context("invalid rpc url")?);

        // Approve the LiFi approval contract to spend input token
        if let Some(ref approval_addr) = quote.estimate.approval_address {
            let spender: alloy::primitives::Address = approval_addr
                .parse()
                .map_err(|e| anyhow::anyhow!("bad approval address: {e}"))?;
            let token_addr = evm::resolve_token(&self.tokens, chain, from_token)
                .with_context(|| format!("token '{from_token}' not in manifest for {chain}"))?;
            let token_contract = evm::IERC20::new(token_addr, &provider);
            println!(
                "  LiFi: approving {} for {} ({})",
                from_token,
                evm::short_addr(&spender),
                evm::short_addr(&token_addr),
            );
            token_contract
                .approve(spender, U256::from(amount_raw))
                .gas(200_000)
                .send()
                .await
                .context("approve for LiFi")?
                .get_receipt()
                .await
                .context("approve receipt")?;
        }

        let tx_req = quote
            .transaction_request
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("LiFi missing transactionRequest"))?;

        let to_addr: alloy::primitives::Address = tx_req
            .to
            .parse()
            .map_err(|e| anyhow::anyhow!("bad tx.to: {e}"))?;
        let data = Bytes::from(hex::decode(tx_req.data.trim_start_matches("0x"))?);
        let value = if let Some(hex_str) = tx_req.value.strip_prefix("0x") {
            U256::from_str_radix(hex_str, 16).unwrap_or(U256::ZERO)
        } else {
            tx_req.value.parse().unwrap_or(U256::ZERO)
        };
        let gas_limit: u64 = tx_req
            .gas_limit
            .as_ref()
            .and_then(|s| {
                s.parse().ok().or_else(|| {
                    s.strip_prefix("0x")
                        .and_then(|h| u64::from_str_radix(h, 16).ok())
                })
            })
            .unwrap_or(500_000);

        use alloy::network::TransactionBuilder;
        use alloy::providers::Provider;
        let tx = alloy::rpc::types::TransactionRequest::default()
            .with_to(to_addr)
            .with_input(data)
            .with_value(value)
            .with_gas_limit(gas_limit);

        let pending = provider
            .send_transaction(tx)
            .await
            .context("LiFi tx send")?;
        let receipt = pending
            .get_receipt()
            .await
            .context("LiFi tx receipt")?;

        println!("  LiFi: tx {:?}", receipt.transaction_hash);
        Ok(())
    }

    async fn get_quote(
        &self,
        from_chain: &Chain,
        to_chain: &Chain,
        from_token: &str,
        to_token: &str,
        amount_wei: &str,
    ) -> Result<QuoteResponse> {
        let from_chain_id = from_chain.chain_id().expect("LiFi requires chain_id");
        let to_chain_id = to_chain.chain_id().expect("LiFi requires chain_id");

        let from_addr = evm::resolve_token(&self.tokens, from_chain, from_token)
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| from_token.to_string());
        let to_addr = evm::resolve_token(&self.tokens, to_chain, to_token)
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| to_token.to_string());

        let slippage = self.slippage_bps / 10000.0;

        let url = format!(
            "{LIFI_API_BASE}/quote?\
            fromChain={from_chain_id}&\
            toChain={to_chain_id}&\
            fromToken={from_addr}&\
            toToken={to_addr}&\
            fromAmount={amount_wei}&\
            fromAddress={}&\
            slippage={slippage}",
            self.wallet_address,
        );

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("LiFi quote request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("LiFi API error {status}: {body}");
        }

        resp.json::<QuoteResponse>()
            .await
            .context("parsing LiFi quote response")
    }

    async fn execute_swap(
        &mut self,
        from_chain: &Chain,
        to_chain: &Chain,
        from_token: &str,
        to_token: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        if input_amount <= 0.0 {
            println!("  LiFi SWAP: skipping zero amount {from_token} → {to_token}");
            return Ok(ExecutionResult::Noop);
        }

        let decimals = self.query_decimals(from_token, from_chain).await?;
        let amount_raw = (input_amount * 10f64.powi(decimals as i32)) as u128;
        let amount_wei = amount_raw.to_string();

        println!(
            "  LiFi SWAP: {} → {} on {} (${:.2}, raw: {})",
            from_token, to_token, from_chain, input_amount, amount_wei
        );

        let quote = self
            .get_quote(from_chain, to_chain, from_token, to_token, &amount_wei)
            .await?;

        let to_amount_raw: f64 = quote.estimate.to_amount.parse().unwrap_or(0.0);
        let to_decimals = self.query_decimals(to_token, to_chain).await.unwrap_or(18);
        let output_amount = to_amount_raw / 10f64.powi(to_decimals as i32);

        println!(
            "  LiFi QUOTE: {} {} → {:.6} {} (est. {:.0}s)",
            input_amount,
            from_token,
            output_amount,
            to_token,
            quote.estimate.execution_duration.unwrap_or(0.0),
        );

        if self.dry_run {
            println!("  LiFi: [DRY RUN] swap would be executed");
            self.metrics.swap_costs += input_amount - output_amount;
            return Ok(ExecutionResult::TokenOutput {
                token: to_token.to_string(),
                amount: output_amount,
            });
        }

        self.submit_lifi_tx(from_chain, from_token, amount_raw, &quote).await?;
        self.metrics.swap_costs += input_amount - output_amount;
        Ok(ExecutionResult::TokenOutput {
            token: to_token.to_string(),
            amount: output_amount,
        })
    }

    async fn execute_bridge(
        &mut self,
        from_chain: &Chain,
        to_chain: &Chain,
        token: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        if input_amount <= 0.0 {
            println!("  LiFi BRIDGE: skipping zero amount {token} {from_chain} → {to_chain}");
            return Ok(ExecutionResult::Noop);
        }

        let decimals = self.query_decimals(token, from_chain).await?;
        let amount_raw = (input_amount * 10f64.powi(decimals as i32)) as u128;
        let amount_wei = amount_raw.to_string();

        println!(
            "  LiFi BRIDGE: {} {} → {} (${:.2})",
            token, from_chain, to_chain, input_amount
        );

        let quote = self
            .get_quote(from_chain, to_chain, token, token, &amount_wei)
            .await?;

        let to_amount_raw: f64 = quote.estimate.to_amount.parse().unwrap_or(0.0);
        let to_decimals = self.query_decimals(token, to_chain).await.unwrap_or(decimals);
        let output_amount = to_amount_raw / 10f64.powi(to_decimals as i32);
        let bridge_fee = input_amount - output_amount;

        println!(
            "  LiFi QUOTE: bridge {:.2} {} → {:.2} {} (fee: ${:.2}, est. {:.0}s)",
            input_amount,
            token,
            output_amount,
            token,
            bridge_fee,
            quote.estimate.execution_duration.unwrap_or(0.0),
        );

        if self.dry_run {
            println!("  LiFi: [DRY RUN] bridge would be executed");
            self.metrics.swap_costs += bridge_fee;
            return Ok(ExecutionResult::TokenOutput {
                token: token.to_string(),
                amount: output_amount,
            });
        }

        self.submit_lifi_tx(from_chain, token, amount_raw, &quote).await?;
        self.metrics.swap_costs += bridge_fee;
        Ok(ExecutionResult::TokenOutput {
            token: token.to_string(),
            amount: output_amount,
        })
    }
}

#[async_trait]
impl Venue for LiFiMovement {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Movement {
                movement_type,
                from_token,
                to_token,
                from_chain,
                to_chain,
                ..
            } => {
                use crate::model::node::MovementType;
                match movement_type {
                    MovementType::Swap => {
                        let swap_chain =
                            from_chain.as_ref().cloned().unwrap_or_else(Chain::hyperevm);
                        self.execute_swap(&swap_chain, &swap_chain, from_token, to_token, input_amount)
                            .await
                    }
                    MovementType::Bridge => {
                        let fc = from_chain
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("bridge requires from_chain"))?;
                        let tc = to_chain
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("bridge requires to_chain"))?;
                        self.execute_bridge(fc, tc, from_token, input_amount).await
                    }
                    MovementType::SwapBridge => {
                        let fc = from_chain.as_ref().cloned().unwrap_or_else(Chain::hyperevm);
                        let tc = to_chain.as_ref().cloned().unwrap_or_else(Chain::hyperevm);
                        self.execute_swap(&fc, &tc, from_token, to_token, input_amount)
                            .await
                    }
                }
            }
            _ => {
                println!("  LiFi: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(0.0)
    }

    async fn unwind(&mut self, _fraction: f64) -> Result<f64> {
        Ok(0.0) // pass-through venue, nothing to unwind
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            swap_costs: self.metrics.swap_costs,
            ..SimMetrics::default()
        }
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        None // pass-through venue
    }
}

