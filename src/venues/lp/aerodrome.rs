use alloy::primitives::{Address, Signed, U256, Uint};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::chain::Chain;
use crate::model::node::{LpAction, Node};

use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

// ── Aerodrome Slipstream contract interfaces ───────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract INonfungiblePositionManager {
        struct MintParams {
            address token0;
            address token1;
            int24 tickSpacing;
            int24 tickLower;
            int24 tickUpper;
            uint256 amount0Desired;
            uint256 amount1Desired;
            uint256 amount0Min;
            uint256 amount1Min;
            address recipient;
            uint256 deadline;
            uint160 sqrtPriceX96;
        }

        struct DecreaseLiquidityParams {
            uint256 tokenId;
            uint128 liquidity;
            uint256 amount0Min;
            uint256 amount1Min;
            uint256 deadline;
        }

        struct CollectParams {
            uint256 tokenId;
            address recipient;
            uint128 amount0Max;
            uint128 amount1Max;
        }

        function mint(MintParams calldata params) external payable returns (uint256 tokenId, uint128 liquidity, uint256 amount0, uint256 amount1);
        function decreaseLiquidity(DecreaseLiquidityParams calldata params) external payable returns (uint256 amount0, uint256 amount1);
        function collect(CollectParams calldata params) external payable returns (uint256 amount0, uint256 amount1);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IGauge {
        function deposit(uint256 tokenId) external;
        function withdraw(uint256 tokenId) external;
        function getReward(address account) external;
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

// ── Aerodrome LP ─────────────────────────────────────────────────

pub struct AerodromeLp {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    chain: Chain,
    tokens: evm::TokenManifest,
    contracts: evm::ContractManifest,
    position_token_id: Option<U256>,
    gauge_address: Option<Address>,
    deposited_value: f64,
    metrics: SimMetrics,
    /// Cached pool name (e.g. "WETH/USDC") from first execute() — needed by unwind().
    cached_pool: Option<String>,
}

impl AerodromeLp {
    pub fn new(
        config: &RuntimeConfig,
        tokens: &evm::TokenManifest,
        contracts: &evm::ContractManifest,
        chain: Chain,
    ) -> Result<Self> {
        Ok(AerodromeLp {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            chain,
            tokens: tokens.clone(),
            contracts: contracts.clone(),
            position_token_id: None,
            gauge_address: None,
            deposited_value: 0.0,
            metrics: SimMetrics::default(),
            cached_pool: None,
        })
    }

    fn parse_pool_tokens(&self, pool: &str) -> Result<(Address, Address)> {
        let parts: Vec<&str> = pool.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid pool format '{}', expected 'TOKEN0/TOKEN1'", pool);
        }
        let token0 =
            evm::resolve_token(&self.tokens, &self.chain, parts[0]).with_context(|| {
                format!(
                    "Token '{}' on {} not in tokens manifest",
                    parts[0], self.chain
                )
            })?;
        let token1 =
            evm::resolve_token(&self.tokens, &self.chain, parts[1]).with_context(|| {
                format!(
                    "Token '{}' on {} not in tokens manifest",
                    parts[1], self.chain
                )
            })?;
        Ok((token0, token1))
    }

    async fn execute_add_liquidity(
        &mut self,
        pool: &str,
        tick_lower: Option<i32>,
        tick_upper: Option<i32>,
        tick_spacing: Option<i32>,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let (token0, token1) = self.parse_pool_tokens(pool)?;
        let spacing = tick_spacing.unwrap_or(100) as i32;
        // Align full-range ticks to tick spacing
        let max_tick = (887272 / spacing) * spacing;
        let lower = tick_lower.unwrap_or(-max_tick);
        let upper = tick_upper.unwrap_or(max_tick);
        let position_manager =
            evm::resolve_contract(&self.contracts, "aerodrome_position_manager", &self.chain)
                .context(
                    "aerodrome_position_manager not in contracts manifest and no hardcoded default",
                )?;

        let parts: Vec<&str> = pool.split('/').collect();
        let decimals0 = token_decimals_for(parts.get(0).unwrap_or(&"ETH"));
        let decimals1 = token_decimals_for(parts.get(1).unwrap_or(&"ETH"));

        let half_amount = input_amount / 2.0;
        let amount0 = evm::to_token_units(half_amount, 1.0, decimals0);
        let amount1 = evm::to_token_units(half_amount, 1.0, decimals1);

        println!(
            "  AERO ADD_LIQUIDITY: {} ticks=[{},{}] spacing={} ${:.2}",
            pool, lower, upper, spacing, input_amount,
        );
        println!(
            "  AERO: token0={} token1={} posManager={}",
            evm::short_addr(&token0),
            evm::short_addr(&token1),
            evm::short_addr(&position_manager),
        );

        if self.dry_run {
            // ── Preflight reads: verify both tokens are valid ERC20s ──
            let rpc = self
                .chain
                .rpc_url()
                .context("LP chain requires RPC URL for preflight")?;
            let rp = evm::read_provider(rpc)?;
            let erc20_0 = IERC20::new(token0, &rp);
            let erc20_1 = IERC20::new(token1, &rp);
            erc20_0
                .balanceOf(self.wallet_address)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Token {} at {} is not a valid ERC20",
                        parts[0],
                        evm::short_addr(&token0),
                    )
                })?;
            erc20_1
                .balanceOf(self.wallet_address)
                .call()
                .await
                .with_context(|| {
                    format!(
                        "Token {} at {} is not a valid ERC20",
                        parts[1],
                        evm::short_addr(&token1),
                    )
                })?;
            println!("  AERO: preflight OK — both tokens are valid ERC20s");
            println!("  AERO: [DRY RUN] would approve tokens + mint position");
            self.deposited_value += input_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let rpc_url = self.chain.rpc_url().context("LP chain requires RPC URL")?;
        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(rpc_url.parse()?);

        let erc20_0 = IERC20::new(token0, &provider);
        let receipt = erc20_0
            .approve(position_manager, amount0)
            .send()
            .await
            .context("approve token0")?
            .get_receipt()
            .await?;
        require_success(&receipt, "approve-token0")?;

        let erc20_1 = IERC20::new(token1, &provider);
        let receipt = erc20_1
            .approve(position_manager, amount1)
            .send()
            .await
            .context("approve token1")?
            .get_receipt()
            .await?;
        require_success(&receipt, "approve-token1")?;

        let deadline = U256::from(chrono::Utc::now().timestamp() as u64 + 300);
        let pm = INonfungiblePositionManager::new(position_manager, &provider);
        let params = INonfungiblePositionManager::MintParams {
            token0,
            token1,
            tickSpacing: Signed::<24, 1>::try_from(spacing)
                .unwrap_or(Signed::<24, 1>::try_from(100).unwrap()),
            tickLower: Signed::<24, 1>::try_from(lower)
                .unwrap_or(Signed::<24, 1>::try_from(-887220).unwrap()),
            tickUpper: Signed::<24, 1>::try_from(upper)
                .unwrap_or(Signed::<24, 1>::try_from(887220).unwrap()),
            amount0Desired: amount0,
            amount1Desired: amount1,
            amount0Min: U256::ZERO,
            amount1Min: U256::ZERO,
            recipient: self.wallet_address,
            deadline,
            sqrtPriceX96: Uint::<160, 3>::ZERO,
        };

        let result = pm
            .mint(params)
            .gas(1_000_000)
            .send()
            .await
            .context("mint LP position")?;
        let receipt = result.get_receipt().await.context("mint receipt")?;
        require_success(&receipt, "mint-lp")?;
        println!("  AERO: mint tx: {:?}", receipt.transaction_hash);

        self.deposited_value += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_remove_liquidity(&mut self, pool: &str) -> Result<ExecutionResult> {
        println!("  AERO REMOVE_LIQUIDITY: {}", pool);

        if self.dry_run {
            println!("  AERO: [DRY RUN] would decreaseLiquidity + collect");
            let value = self.deposited_value;
            self.deposited_value = 0.0;
            return Ok(ExecutionResult::TokenOutput {
                token: "USDC".to_string(),
                amount: value,
            });
        }

        if self.position_token_id.is_none() {
            println!("  AERO: no position to remove");
            return Ok(ExecutionResult::Noop);
        }

        let value = self.deposited_value;
        self.deposited_value = 0.0;
        Ok(ExecutionResult::TokenOutput {
            token: "USDC".to_string(),
            amount: value,
        })
    }

    async fn execute_claim_rewards(&mut self, pool: &str) -> Result<ExecutionResult> {
        println!("  AERO CLAIM_REWARDS: {}", pool);

        if self.dry_run {
            println!("  AERO: [DRY RUN] would call gauge.getReward()");
            let estimated_reward = self.deposited_value * 0.001;
            if estimated_reward > 0.0 {
                self.metrics.lp_fees += estimated_reward;
                return Ok(ExecutionResult::TokenOutput {
                    token: "AERO".to_string(),
                    amount: estimated_reward,
                });
            }
            return Ok(ExecutionResult::Noop);
        }

        if let Some(gauge) = self.gauge_address {
            let rpc_url = self.chain.rpc_url().context("LP chain requires RPC URL")?;
            let signer: alloy::signers::local::PrivateKeySigner = self
                .private_key
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
            let wallet = alloy::network::EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(rpc_url.parse()?);

            let gauge_contract = IGauge::new(gauge, &provider);
            let result = gauge_contract
                .getReward(self.wallet_address)
                .send()
                .await
                .context("getReward")?;
            let receipt = result.get_receipt().await.context("getReward receipt")?;
            println!("  AERO: getReward tx: {:?}", receipt.transaction_hash);
        } else {
            println!("  AERO: no gauge address set, skipping reward claim");
        }

        Ok(ExecutionResult::Noop)
    }

    async fn execute_stake_gauge(&mut self, pool: &str) -> Result<ExecutionResult> {
        println!("  AERO STAKE_GAUGE: {}", pool);
        if self.dry_run {
            println!("  AERO: [DRY RUN] would deposit NFT into gauge");
        }
        Ok(ExecutionResult::Noop)
    }

    async fn execute_unstake_gauge(&mut self, pool: &str) -> Result<ExecutionResult> {
        println!("  AERO UNSTAKE_GAUGE: {}", pool);
        if self.dry_run {
            println!("  AERO: [DRY RUN] would withdraw NFT from gauge");
        }
        Ok(ExecutionResult::Noop)
    }

    async fn execute_compound(&mut self, pool: &str) -> Result<ExecutionResult> {
        println!("  AERO COMPOUND: {}", pool);
        if self.dry_run {
            println!("  AERO: [DRY RUN] would claim + swap AERO→tokens + add liquidity");
        }
        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for AerodromeLp {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Lp {
                pool,
                action,
                tick_lower,
                tick_upper,
                tick_spacing,
                ..
            } => {
                self.cached_pool = Some(pool.clone());
                match action {
                    LpAction::AddLiquidity => {
                        self.execute_add_liquidity(
                            pool,
                            *tick_lower,
                            *tick_upper,
                            *tick_spacing,
                            input_amount,
                        )
                        .await
                    }
                    LpAction::RemoveLiquidity => self.execute_remove_liquidity(pool).await,
                    LpAction::ClaimRewards => self.execute_claim_rewards(pool).await,
                    LpAction::StakeGauge => self.execute_stake_gauge(pool).await,
                    LpAction::UnstakeGauge => self.execute_unstake_gauge(pool).await,
                    LpAction::Compound => self.execute_compound(pool).await,
                }
            }
            _ => {
                println!("  AERO: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.deposited_value)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        if self.deposited_value > 0.0 {
            println!("  AERO TICK: deposited=${:.2}", self.deposited_value,);
        }
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let freed = total * f;

        println!("  AERO: UNWIND {:.1}% (${:.2})", f * 100.0, freed);

        if self.dry_run {
            println!(
                "  AERO: [DRY RUN] would decreaseLiquidity by {:.1}% + collect",
                f * 100.0
            );
            self.deposited_value = (self.deposited_value * (1.0 - f)).max(0.0);
            return Ok(freed);
        }

        // Live: decreaseLiquidity for the fraction, then collect
        if let Some(token_id) = self.position_token_id {
            let rpc_url = self
                .chain
                .rpc_url()
                .context("LP chain requires RPC URL for unwind")?;
            let signer: alloy::signers::local::PrivateKeySigner = self
                .private_key
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
            let wallet = alloy::network::EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(rpc_url.parse()?);

            let position_manager =
                evm::resolve_contract(&self.contracts, "aerodrome_position_manager", &self.chain)
                    .context("aerodrome_position_manager not in contracts manifest")?;

            let pm = INonfungiblePositionManager::new(position_manager, &provider);

            // TODO: query actual liquidity from NFT position to compute exact amount.
            // For now, use uint128::MAX * fraction as a sentinel — the contract
            // will clamp to actual liquidity.
            let max_liq = u128::MAX;
            let liq_to_remove = ((max_liq as f64) * f) as u128;

            let deadline = U256::from(chrono::Utc::now().timestamp() as u64 + 300);
            let params = INonfungiblePositionManager::DecreaseLiquidityParams {
                tokenId: token_id,
                liquidity: liq_to_remove,
                amount0Min: U256::ZERO,
                amount1Min: U256::ZERO,
                deadline,
            };

            let result = pm
                .decreaseLiquidity(params)
                .gas(500_000)
                .send()
                .await
                .context("decreaseLiquidity for unwind")?;
            let receipt = result.get_receipt().await?;
            require_success(&receipt, "unwind-decreaseLiquidity")?;
            println!(
                "  AERO: decreaseLiquidity tx: {:?}",
                receipt.transaction_hash
            );

            // Collect the freed tokens
            let collect_params = INonfungiblePositionManager::CollectParams {
                tokenId: token_id,
                recipient: self.wallet_address,
                amount0Max: u128::MAX,
                amount1Max: u128::MAX,
            };
            let result = pm
                .collect(collect_params)
                .gas(300_000)
                .send()
                .await
                .context("collect after unwind")?;
            let receipt = result.get_receipt().await?;
            require_success(&receipt, "unwind-collect")?;
            println!("  AERO: collect tx: {:?}", receipt.transaction_hash);
        }

        self.deposited_value = (self.deposited_value * (1.0 - f)).max(0.0);
        Ok(freed)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lp_fees: self.metrics.lp_fees,
            ..SimMetrics::default()
        }
    }
}

fn require_success(
    receipt: &alloy::rpc::types::TransactionReceipt,
    label: &str,
) -> anyhow::Result<()> {
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

fn token_decimals_for(symbol: &str) -> u8 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        _ => 18,
    }
}
