use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::chain::Chain;
use crate::model::node::{LendingAction, Node};

use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

// ── Aave V3 Pool interface (works for any Aave fork) ────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IAavePool {
        function supply(address asset, uint256 amount, address onBehalfOf, uint16 referralCode) external;
        function withdraw(address asset, uint256 amount, address to) external returns (uint256);
        function borrow(address asset, uint256 amount, uint256 interestRateMode, uint16 referralCode, address onBehalfOf) external;
        function repay(address asset, uint256 amount, uint256 interestRateMode, address onBehalfOf) external returns (uint256);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IRewardsController {
        function claimAllRewards(address[] calldata assets, address to) external returns (address[] memory rewardsList, uint256[] memory claimedAmounts);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
        function decimals() external view returns (uint8);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IAavePoolRead {
        function getReserveData(address asset) external view returns (
            uint256, uint128, uint128, uint128, uint128, uint128,
            uint40, uint16, address, address, address, address,
            uint128, uint128, uint128
        );
    }
}

// ── Aave Lending ──────────────────────────────────────────────────

/// Cached node context for unwind() — resolved on first execute().
struct CachedLendingContext {
    pool_addr: Address,
    token_addr: Address,
    rpc_url: String,
    asset_symbol: String,
}

pub struct AaveLending {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    tokens: evm::TokenManifest,
    contracts: evm::ContractManifest,
    supplied_value: f64,
    borrowed_value: f64,
    metrics: SimMetrics,
    cached_ctx: Option<CachedLendingContext>,
}

impl AaveLending {
    pub fn new(
        config: &RuntimeConfig,
        tokens: &evm::TokenManifest,
        contracts: &evm::ContractManifest,
        node: &Node,
    ) -> Result<Self> {
        // Pre-populate cached context from the node so total_value() can query
        // on-chain state even before the first execute() (e.g. after a restart).
        let cached_ctx = if let Node::Lending {
            chain,
            pool_address,
            asset,
            ..
        } = node
        {
            let rpc_url = chain.rpc_url();
            let pool_addr = evm::resolve_contract(contracts, pool_address, chain);
            let token_addr = evm::resolve_token(tokens, chain, asset);
            match (rpc_url, pool_addr, token_addr) {
                (Some(rpc), Some(pool), Some(token)) => Some(CachedLendingContext {
                    pool_addr: pool,
                    token_addr: token,
                    rpc_url: rpc.to_string(),
                    asset_symbol: asset.clone(),
                }),
                _ => None,
            }
        } else {
            None
        };

        Ok(AaveLending {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            contracts: contracts.clone(),
            supplied_value: 0.0,
            borrowed_value: 0.0,
            metrics: SimMetrics::default(),
            cached_ctx,
        })
    }

    /// Query on-chain aToken balance (supply) and variable debt token balance (borrow).
    async fn query_onchain_value(&self, ctx: &CachedLendingContext) -> Result<f64> {
        let rp = evm::read_provider(&ctx.rpc_url)?;
        let pool_read = IAavePoolRead::new(ctx.pool_addr, &rp);
        let reserve_data = pool_read
            .getReserveData(ctx.token_addr)
            .call()
            .await
            .context("getReserveData for total_value")?;

        let decimals = evm::query_decimals(&ctx.rpc_url, ctx.token_addr).await?;

        // aToken balance = supplied value (includes accrued interest)
        let a_token_addr = reserve_data._8;
        let a_token = IERC20::new(a_token_addr, &rp);
        let supply_balance = a_token
            .balanceOf(self.wallet_address)
            .call()
            .await
            .context("aToken.balanceOf")?;
        let supplied = evm::from_token_units(supply_balance, decimals);

        // Variable debt token balance = borrowed value (includes accrued interest)
        let var_debt_addr = reserve_data._10;
        let var_debt = IERC20::new(var_debt_addr, &rp);
        let debt_balance = var_debt
            .balanceOf(self.wallet_address)
            .call()
            .await
            .unwrap_or(U256::ZERO);
        let borrowed = evm::from_token_units(debt_balance, decimals);

        Ok((supplied - borrowed).max(0.0))
    }

    async fn execute_supply(
        &mut self,
        pool_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = evm::query_decimals(rpc_url, token_addr).await?;
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING SUPPLY: {} {} to pool {}",
            input_amount,
            asset_symbol,
            evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify pool supports this asset ──
            let rp = evm::read_provider(rpc_url)?;
            let pool_read = IAavePoolRead::new(pool_addr, &rp);
            pool_read
                .getReserveData(token_addr)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Aave pool {} does not support {} — getReserveData reverted",
                        evm::short_addr(&pool_addr),
                        asset_symbol,
                    )
                })?;
            println!("  LENDING: preflight OK — pool supports {}", asset_symbol);
            println!(
                "  LENDING: [DRY RUN] would approve {} + supply to pool",
                asset_symbol
            );
            self.supplied_value += input_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let erc20 = IERC20::new(token_addr, &provider);
        let approve_tx = erc20.approve(pool_addr, amount_units);
        let pending = approve_tx.send().await.context("ERC20 approve failed")?;
        let receipt = pending.get_receipt().await.context("approve receipt")?;
        require_success(&receipt, "approve")?;
        println!("  LENDING: approve tx: {:?}", receipt.transaction_hash);

        let pool = IAavePool::new(pool_addr, &provider);
        let supply_tx = pool.supply(token_addr, amount_units, self.wallet_address, 0);
        let pending = supply_tx.send().await.context("supply failed")?;
        let receipt = pending.get_receipt().await.context("supply receipt")?;
        require_success(&receipt, "supply")?;
        println!("  LENDING: supply tx: {:?}", receipt.transaction_hash);

        self.supplied_value += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_withdraw(
        &mut self,
        pool_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = evm::query_decimals(rpc_url, token_addr).await?;
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING WITHDRAW: {} {} from pool {}",
            input_amount,
            asset_symbol,
            evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify pool supports this asset ──
            let rp = evm::read_provider(rpc_url)?;
            let pool_read = IAavePoolRead::new(pool_addr, &rp);
            pool_read
                .getReserveData(token_addr)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Aave pool {} does not support {} — getReserveData reverted",
                        evm::short_addr(&pool_addr),
                        asset_symbol,
                    )
                })?;
            println!("  LENDING: preflight OK — pool supports {}", asset_symbol);
            println!("  LENDING: [DRY RUN] would withdraw from pool");
            self.supplied_value = (self.supplied_value - input_amount).max(0.0);
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        // Query aToken balance to avoid NotEnoughAvailableUserBalance from
        // rounding (scaledBalance = amount / liquidityIndex truncates).
        let rp = evm::read_provider(rpc_url)?;
        let pool_read = IAavePoolRead::new(pool_addr, &rp);
        let reserve_data = pool_read
            .getReserveData(token_addr)
            .call()
            .await
            .context("getReserveData failed during withdraw")?;
        let a_token_addr = reserve_data._8; // aTokenAddress
        let a_token = IERC20::new(a_token_addr, &rp);
        let a_balance = a_token
            .balanceOf(self.wallet_address)
            .call()
            .await
            .context("aToken balanceOf failed")?;
        let withdraw_units = amount_units.min(a_balance);

        let pool = IAavePool::new(pool_addr, &provider);
        let withdraw_tx = pool.withdraw(token_addr, withdraw_units, self.wallet_address);
        let pending = withdraw_tx.send().await.context("withdraw failed")?;
        let receipt = pending.get_receipt().await.context("withdraw receipt")?;
        require_success(&receipt, "withdraw")?;
        println!("  LENDING: withdraw tx: {:?}", receipt.transaction_hash);

        self.supplied_value = (self.supplied_value - input_amount).max(0.0);
        Ok(ExecutionResult::TokenOutput {
            token: asset_symbol.to_string(),
            amount: input_amount,
        })
    }

    async fn execute_borrow(
        &mut self,
        pool_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = evm::query_decimals(rpc_url, token_addr).await?;
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING BORROW: {} {} from pool {}",
            input_amount,
            asset_symbol,
            evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify pool supports this asset ──
            let rp = evm::read_provider(rpc_url)?;
            let pool_read = IAavePoolRead::new(pool_addr, &rp);
            pool_read
                .getReserveData(token_addr)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Aave pool {} does not support {} — getReserveData reverted",
                        evm::short_addr(&pool_addr),
                        asset_symbol,
                    )
                })?;
            println!("  LENDING: preflight OK — pool supports {}", asset_symbol);
            println!("  LENDING: [DRY RUN] would borrow from pool");
            self.borrowed_value += input_amount;
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let pool = IAavePool::new(pool_addr, &provider);
        let borrow_tx = pool
            .borrow(
                token_addr,
                amount_units,
                U256::from(2),
                0,
                self.wallet_address,
            )
            .gas(500_000);
        let pending = borrow_tx.send().await.context("borrow failed")?;
        let receipt = pending.get_receipt().await.context("borrow receipt")?;
        require_success(&receipt, "borrow")?;
        println!("  LENDING: borrow tx: {:?}", receipt.transaction_hash);

        self.borrowed_value += input_amount;
        Ok(ExecutionResult::TokenOutput {
            token: asset_symbol.to_string(),
            amount: input_amount,
        })
    }

    async fn execute_repay(
        &mut self,
        pool_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = evm::query_decimals(rpc_url, token_addr).await?;
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING REPAY: {} {} to pool {}",
            input_amount,
            asset_symbol,
            evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify pool supports this asset ──
            let rp = evm::read_provider(rpc_url)?;
            let pool_read = IAavePoolRead::new(pool_addr, &rp);
            pool_read
                .getReserveData(token_addr)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Aave pool {} does not support {} — getReserveData reverted",
                        evm::short_addr(&pool_addr),
                        asset_symbol,
                    )
                })?;
            println!("  LENDING: preflight OK — pool supports {}", asset_symbol);
            println!("  LENDING: [DRY RUN] would approve + repay to pool");
            self.borrowed_value = (self.borrowed_value - input_amount).max(0.0);
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let erc20 = IERC20::new(token_addr, &provider);
        let approve_tx = erc20.approve(pool_addr, amount_units);
        let pending = approve_tx.send().await.context("approve failed")?;
        let receipt = pending.get_receipt().await.context("approve receipt")?;
        require_success(&receipt, "repay-approve")?;

        let pool = IAavePool::new(pool_addr, &provider);
        let repay_tx = pool.repay(token_addr, amount_units, U256::from(2), self.wallet_address);
        let pending = repay_tx.send().await.context("repay failed")?;
        let receipt = pending.get_receipt().await.context("repay receipt")?;
        require_success(&receipt, "repay")?;
        println!("  LENDING: repay tx: {:?}", receipt.transaction_hash);

        self.borrowed_value = (self.borrowed_value - input_amount).max(0.0);
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_claim_rewards(
        &mut self,
        rewards_controller: Option<&str>,
        chain: &Chain,
        rpc_url: &str,
        token_addr: Address,
    ) -> Result<ExecutionResult> {
        let rc_name = match rewards_controller {
            Some(name) => name,
            None => {
                println!("  LENDING: no rewards_controller configured, skipping claim");
                return Ok(ExecutionResult::Noop);
            }
        };

        let rewards_addr =
            evm::resolve_contract(&self.contracts, rc_name, chain).with_context(|| {
                format!(
                    "Contract '{}' on {} not in contracts manifest",
                    rc_name, chain
                )
            })?;

        println!(
            "  LENDING CLAIM: rewards from {}",
            evm::short_addr(&rewards_addr),
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would claim rewards");
            return Ok(ExecutionResult::Noop);
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let rewards = IRewardsController::new(rewards_addr, &provider);
        let assets = vec![token_addr];
        let claim_tx = rewards.claimAllRewards(assets, self.wallet_address);
        let pending = claim_tx.send().await.context("claimAllRewards failed")?;
        let receipt = pending.get_receipt().await.context("claim receipt")?;
        require_success(&receipt, "claim")?;
        println!("  LENDING: claim tx: {:?}", receipt.transaction_hash);

        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for AaveLending {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Lending {
                chain,
                pool_address,
                asset,
                action,
                rewards_controller,
                ..
            } => {
                let rpc_url = chain.rpc_url().context("lending chain requires RPC URL")?;
                let pool_addr = evm::resolve_contract(&self.contracts, pool_address, chain)
                    .with_context(|| {
                        format!(
                            "Contract '{}' on {} not in contracts manifest",
                            pool_address, chain
                        )
                    })?;
                let token_addr =
                    evm::resolve_token(&self.tokens, chain, asset).with_context(|| {
                        format!("Token '{asset}' on {chain} not in tokens manifest")
                    })?;

                // Cache context for unwind()
                self.cached_ctx = Some(CachedLendingContext {
                    pool_addr,
                    token_addr,
                    rpc_url: rpc_url.to_string(),
                    asset_symbol: asset.clone(),
                });

                match action {
                    LendingAction::Supply => {
                        self.execute_supply(pool_addr, rpc_url, token_addr, asset, input_amount)
                            .await
                    }
                    LendingAction::Withdraw => {
                        self.execute_withdraw(pool_addr, rpc_url, token_addr, asset, input_amount)
                            .await
                    }
                    LendingAction::Borrow => {
                        self.execute_borrow(pool_addr, rpc_url, token_addr, asset, input_amount)
                            .await
                    }
                    LendingAction::Repay => {
                        self.execute_repay(pool_addr, rpc_url, token_addr, asset, input_amount)
                            .await
                    }
                    LendingAction::ClaimRewards => {
                        self.execute_claim_rewards(
                            rewards_controller.as_deref(),
                            chain,
                            rpc_url,
                            token_addr,
                        )
                        .await
                    }
                }
            }
            _ => {
                println!("  LENDING: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        // Live mode: query on-chain aToken + debt token balances for accurate TVL.
        if !self.dry_run {
            if let Some(ctx) = &self.cached_ctx {
                match self.query_onchain_value(ctx).await {
                    Ok(val) => return Ok(val),
                    Err(e) => {
                        eprintln!(
                            "  LENDING: on-chain query failed, falling back to local: {:#}",
                            e
                        );
                    }
                }
            }
        }
        Ok((self.supplied_value - self.borrowed_value).max(0.0))
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        if self.supplied_value > 0.0 || self.borrowed_value > 0.0 {
            println!(
                "  LENDING TICK: supplied=${:.2}, borrowed=${:.2}, net=${:.2}",
                self.supplied_value,
                self.borrowed_value,
                self.supplied_value - self.borrowed_value,
            );
        }
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let withdraw_amount = total * f;

        println!(
            "  LENDING: UNWIND {:.1}% (${:.2})",
            f * 100.0,
            withdraw_amount
        );

        let ctx = self
            .cached_ctx
            .as_ref()
            .context("unwind() called before execute() — no cached lending context")?;

        // Reuses execute_withdraw() which handles dry_run (preflight only) vs live (actual tx)
        self.execute_withdraw(
            ctx.pool_addr,
            &ctx.rpc_url.clone(),
            ctx.token_addr,
            &ctx.asset_symbol.clone(),
            withdraw_amount,
        )
        .await?;

        Ok(withdraw_amount)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lending_interest: self.metrics.lending_interest,
            ..SimMetrics::default()
        }
    }
}

fn make_provider(
    private_key: &str,
    rpc_url: &str,
) -> Result<impl alloy::providers::Provider + Clone> {
    let signer: alloy::signers::local::PrivateKeySigner = private_key
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
    let wallet = alloy::network::EthereumWallet::from(signer);
    Ok(ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse()?))
}

fn require_success(receipt: &alloy::rpc::types::TransactionReceipt, label: &str) -> Result<()> {
    if !receipt.status() {
        anyhow::bail!(
            "{} tx reverted (hash: {:?}, gas_used: {:?})",
            label,
            receipt.transaction_hash,
            receipt.gas_used,
        );
    }
    Ok(())
}

