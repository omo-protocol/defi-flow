use std::collections::HashMap;
use std::sync::LazyLock;

use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::engine::venue::{ExecutionResult, SimMetrics};
use crate::model::chain::Chain;
use crate::model::node::{Node, PendleAction};
use crate::run::config::RuntimeConfig;

use super::evm;
use super::VenueExecutor;

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

// ── Known Pendle markets ───────────────────────────────────────────

struct PendleMarket {
    chain: Chain,
    market_address: Address,
    sy_address: Address,
    yt_address: Address,
}

static PENDLE_MARKETS: LazyLock<HashMap<String, PendleMarket>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    // PT-kHYPE on HyperEVM (placeholder addresses — update with real ones)
    m.insert(
        "PT-kHYPE".to_string(),
        PendleMarket {
            chain: Chain::HyperEvm,
            market_address: "0x0000000000000000000000000000000000000001"
                .parse()
                .unwrap(),
            sy_address: "0x0000000000000000000000000000000000000002"
                .parse()
                .unwrap(),
            yt_address: "0x0000000000000000000000000000000000000003"
                .parse()
                .unwrap(),
        },
    );
    m
});

// ── Pendle Executor ────────────────────────────────────────────────

pub struct PendleExecutor {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    /// PT tokens held per market.
    pt_holdings: HashMap<String, f64>,
    /// YT tokens held per market.
    yt_holdings: HashMap<String, f64>,
    metrics: SimMetrics,
}

impl PendleExecutor {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        Ok(PendleExecutor {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            pt_holdings: HashMap::new(),
            yt_holdings: HashMap::new(),
            metrics: SimMetrics::default(),
        })
    }

    async fn execute_mint_pt(
        &mut self,
        market_name: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let market_info = PENDLE_MARKETS.get(market_name);

        println!(
            "  PENDLE MINT_PT: {} with ${:.2}",
            market_name, input_amount,
        );

        if let Some(info) = market_info {
            let router = evm::pendle_router(&info.chain);
            println!(
                "  PENDLE: chain={}, market={}, router={}",
                info.chain,
                evm::short_addr(&info.market_address),
                router
                    .map(|r| evm::short_addr(&r))
                    .unwrap_or("none".to_string()),
            );
        }

        if self.dry_run {
            println!("  PENDLE: [DRY RUN] would approve SY + mintPyFromSy()");
            // PT is typically minted at a discount (~0.95 of underlying)
            let pt_amount = input_amount * 0.98; // small discount
            *self.pt_holdings.entry(market_name.to_string()).or_insert(0.0) += pt_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let Some(info) = market_info else {
            println!("  PENDLE: unknown market '{market_name}', treating as dry-run");
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        };

        let Some(router_addr) = evm::pendle_router(&info.chain) else {
            anyhow::bail!("No Pendle router for chain {}", info.chain);
        };

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(evm::rpc_url(&info.chain).parse()?);

        let amount_units = evm::to_token_units(input_amount, 1.0, 18);

        // Approve SY token
        let erc20 = IERC20::new(info.sy_address, &provider);
        erc20
            .approve(router_addr, amount_units)
            .send()
            .await
            .context("approve SY")?
            .get_receipt()
            .await?;

        // Mint PT+YT
        let router = IPendleRouter::new(router_addr, &provider);
        let result = router
            .mintPyFromSy(self.wallet_address, info.yt_address, amount_units, U256::ZERO)
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
            // At maturity, PT redeems 1:1
            let output = holdings;
            self.pt_holdings.remove(market_name);
            return Ok(ExecutionResult::TokenOutput {
                token: "USDC".to_string(),
                amount: output,
            });
        }

        // TODO: actual redemption via router.redeemPyToSy()
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

        // TODO: actual claim via router.redeemDueInterestAndRewards()
        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl VenueExecutor for PendleExecutor {
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

    async fn tick(&mut self) -> Result<()> {
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

    fn metrics(&self) -> SimMetrics {
        SimMetrics::default()
    }
}
