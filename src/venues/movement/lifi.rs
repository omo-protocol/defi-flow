use anyhow::{bail, Context, Result};
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
    dry_run: bool,
    tokens: evm::TokenManifest,
    slippage_bps: f64,
    metrics: SimMetrics,
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
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            slippage_bps: config.slippage_bps,
            metrics: SimMetrics::default(),
        })
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
        from_token: &str,
        to_token: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        if input_amount <= 0.0 {
            println!("  LiFi SWAP: skipping zero amount {from_token} → {to_token}");
            return Ok(ExecutionResult::Noop);
        }

        let decimals = token_decimals(from_token);
        let amount_raw = (input_amount * 10f64.powi(decimals as i32)) as u128;
        let amount_wei = amount_raw.to_string();

        println!(
            "  LiFi SWAP: {} → {} on {} (${:.2}, raw: {})",
            from_token, to_token, from_chain, input_amount, amount_wei
        );

        let quote = self
            .get_quote(from_chain, from_chain, from_token, to_token, &amount_wei)
            .await?;

        let to_amount_raw: f64 = quote.estimate.to_amount.parse().unwrap_or(0.0);
        let to_decimals = token_decimals(to_token);
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

        if let Some(tx) = &quote.transaction_request {
            println!(
                "  LiFi TX: to={}, value={}, data_len={}",
                &tx.to[..10],
                tx.value,
                tx.data.len()
            );

            println!("  LiFi: submitting transaction...");
            self.metrics.swap_costs += input_amount - output_amount;
            Ok(ExecutionResult::TokenOutput {
                token: to_token.to_string(),
                amount: output_amount,
            })
        } else {
            bail!("LiFi quote did not include transactionRequest");
        }
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

        let decimals = token_decimals(token);
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
        let output_amount = to_amount_raw / 10f64.powi(decimals as i32);
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

        if let Some(tx) = &quote.transaction_request {
            println!(
                "  LiFi TX: to={}, value={}, data_len={}",
                &tx.to[..10],
                tx.value,
                tx.data.len()
            );

            println!("  LiFi: submitting bridge transaction...");
            self.metrics.swap_costs += bridge_fee;
            Ok(ExecutionResult::TokenOutput {
                token: token.to_string(),
                amount: output_amount,
            })
        } else {
            bail!("LiFi quote did not include transactionRequest for bridge");
        }
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
                        let swap_chain = from_chain.as_ref().cloned().unwrap_or_else(Chain::hyperevm);
                        self.execute_swap(&swap_chain, from_token, to_token, input_amount)
                            .await
                    }
                    MovementType::Bridge => {
                        let fc = from_chain.as_ref().ok_or_else(|| anyhow::anyhow!("bridge requires from_chain"))?;
                        let tc = to_chain.as_ref().ok_or_else(|| anyhow::anyhow!("bridge requires to_chain"))?;
                        self.execute_bridge(fc, tc, from_token, input_amount)
                            .await
                    }
                    MovementType::SwapBridge => {
                        // For now, LiFi handles swap+bridge atomically — same as swap
                        // TODO: use LiFi's cross-chain swap endpoint
                        let swap_chain = from_chain.as_ref().cloned().unwrap_or_else(Chain::hyperevm);
                        self.execute_swap(&swap_chain, from_token, to_token, input_amount)
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
}

fn token_decimals(symbol: &str) -> i32 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        "DAI" | "USDE" => 18,
        "WETH" | "ETH" => 18,
        "AERO" | "HYPE" | "WHYPE" | "OP" | "ARB" => 18,
        _ => 18,
    }
}
