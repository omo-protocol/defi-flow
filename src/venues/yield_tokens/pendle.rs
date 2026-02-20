use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::chain::Chain;
use crate::model::node::{Node, PendleAction};
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, Venue};

// ── Pendle Router interface ────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IPendleRouter {
        function mintPyFromSy(
            address receiver,
            address YT,
            uint256 netSyIn,
            uint256 minPyOut
        ) external returns (uint256 netPyOut);

        function redeemPyToSy(
            address receiver,
            address YT,
            uint256 netPyIn,
            uint256 minSyOut
        ) external returns (uint256 netSyOut);

        function redeemDueInterestAndRewards(
            address user,
            address[] calldata sys,
            address[] calldata yts,
            address[] calldata markets
        ) external;
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

// ── Contract key derivation ──────────────────────────────────────

/// Derive a contracts manifest key from a Pendle market name.
/// E.g. `pendle_contract_key("PT-kHYPE", "market")` → `"pendle_pt_khype_market"`.
pub fn pendle_contract_key(market: &str, suffix: &str) -> String {
    let normalized = market.to_lowercase().replace('-', "_");
    format!("pendle_{}_{}", normalized, suffix)
}

/// Determine the chain for a Pendle market by inspecting the contracts manifest.
fn pendle_chain(contracts: &evm::ContractManifest, market: &str) -> Option<Chain> {
    let market_key = pendle_contract_key(market, "market");
    let chains = contracts.get(&market_key)?;
    let (chain_name, _) = chains.iter().next()?;
    Some(Chain::from_name(chain_name))
}

// ── Pendle Yield ──────────────────────────────────────────────────

pub struct PendleYield {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    contracts: evm::ContractManifest,
    pt_holdings: HashMap<String, f64>,
    yt_holdings: HashMap<String, f64>,
}

impl PendleYield {
    pub fn new(config: &RuntimeConfig, contracts: &evm::ContractManifest) -> Result<Self> {
        Ok(PendleYield {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            contracts: contracts.clone(),
            pt_holdings: HashMap::new(),
            yt_holdings: HashMap::new(),
        })
    }

    async fn execute_mint_pt(
        &mut self,
        market_name: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let market_key = pendle_contract_key(market_name, "market");
        let sy_key = pendle_contract_key(market_name, "sy");
        let yt_key = pendle_contract_key(market_name, "yt");

        let chain = pendle_chain(&self.contracts, market_name);

        println!(
            "  PENDLE MINT_PT: {} with ${:.2}",
            market_name, input_amount,
        );

        if let Some(ref ch) = chain {
            let market_addr = evm::resolve_contract(&self.contracts, &market_key, ch);
            let router = evm::resolve_contract(&self.contracts, "pendle_router", ch);
            println!(
                "  PENDLE: chain={}, market={}, router={}",
                ch,
                market_addr
                    .map(|a| evm::short_addr(&a))
                    .unwrap_or("none".to_string()),
                router
                    .map(|r| evm::short_addr(&r))
                    .unwrap_or("none".to_string()),
            );
        }

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would approve SY + mintPyFromSy()");
            let pt_amount = input_amount * 0.98;
            *self.pt_holdings.entry(market_name.to_string()).or_insert(0.0) += pt_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let Some(ch) = chain else {
            anyhow::bail!(
                "Pendle market '{}' not in contracts manifest (key: {})",
                market_name, market_key,
            );
        };

        let sy_addr = evm::resolve_contract(&self.contracts, &sy_key, &ch)
            .with_context(|| format!("'{}' not in contracts manifest for chain {}", sy_key, ch))?;
        let yt_addr = evm::resolve_contract(&self.contracts, &yt_key, &ch)
            .with_context(|| format!("'{}' not in contracts manifest for chain {}", yt_key, ch))?;
        let router_addr = evm::resolve_contract(&self.contracts, "pendle_router", &ch)
            .with_context(|| format!("'pendle_router' not in contracts manifest for chain {}", ch))?;

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(ch.rpc_url().expect("Pendle chain requires RPC").parse()?);

        let amount_units = evm::to_token_units(input_amount, 1.0, 18);

        let erc20 = IERC20::new(sy_addr, &provider);
        erc20
            .approve(router_addr, amount_units)
            .send()
            .await
            .context("approve SY")?
            .get_receipt()
            .await?;

        let router = IPendleRouter::new(router_addr, &provider);
        let result = router
            .mintPyFromSy(self.wallet_address, yt_addr, amount_units, U256::ZERO)
            .send()
            .await
            .context("mintPyFromSy")?;
        let receipt = result.get_receipt().await.context("mint receipt")?;
        println!("  PENDLE: mint tx: {:?}", receipt.transaction_hash);

        *self
            .pt_holdings
            .entry(market_name.to_string())
            .or_insert(0.0) += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_redeem_pt(
        &mut self,
        market_name: &str,
    ) -> Result<ExecutionResult> {
        let holdings = self.pt_holdings.get(market_name).copied().unwrap_or(0.0);

        println!(
            "  PENDLE REDEEM_PT: {} (holdings: ${:.2})",
            market_name, holdings,
        );

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would redeemPyToSy()");
            let output = holdings;
            self.pt_holdings.remove(market_name);
            return Ok(ExecutionResult::TokenOutput {
                token: "USDC".to_string(),
                amount: output,
            });
        }

        let output = holdings;
        self.pt_holdings.remove(market_name);
        Ok(ExecutionResult::TokenOutput {
            token: "USDC".to_string(),
            amount: output,
        })
    }

    async fn execute_claim_rewards(
        &mut self,
        market_name: &str,
    ) -> Result<ExecutionResult> {
        println!("  PENDLE CLAIM_REWARDS: {}", market_name);

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would redeemDueInterestAndRewards()");
            return Ok(ExecutionResult::Noop);
        }

        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for PendleYield {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Pendle {
                market, action, ..
            } => match action {
                PendleAction::MintPt => self.execute_mint_pt(market, input_amount).await,
                PendleAction::RedeemPt => self.execute_redeem_pt(market).await,
                PendleAction::MintYt => {
                    println!("  PENDLE MINT_YT: {} ${:.2}", market, input_amount);
                    if self.dry_run {
                        println!("  PENDLE: [DRY RUN] would mint YT");
                    }
                    *self
                        .yt_holdings
                        .entry(market.to_string())
                        .or_insert(0.0) += input_amount;
                    Ok(ExecutionResult::PositionUpdate {
                        consumed: input_amount,
                        output: None,
                    })
                }
                PendleAction::RedeemYt => {
                    println!("  PENDLE REDEEM_YT: {}", market);
                    let holdings = self.yt_holdings.get(market).copied().unwrap_or(0.0);
                    if self.dry_run {
                        println!("  PENDLE: [DRY RUN] would redeem YT");
                    }
                    self.yt_holdings.remove(market);
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: holdings,
                    })
                }
                PendleAction::ClaimRewards => self.execute_claim_rewards(market).await,
            },
            _ => {
                println!("  PENDLE: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        let pt_total: f64 = self.pt_holdings.values().sum();
        let yt_total: f64 = self.yt_holdings.values().sum();
        Ok(pt_total + yt_total)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        let pt_total: f64 = self.pt_holdings.values().sum();
        let yt_total: f64 = self.yt_holdings.values().sum();
        if pt_total > 0.0 || yt_total > 0.0 {
            println!(
                "  PENDLE TICK: PT=${:.2}, YT=${:.2}",
                pt_total, yt_total,
            );
        }
        Ok(())
    }
}
