use alloy::primitives::{Address, Signed, Uint, U256};
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
    position_token_id: Option<U256>,
    gauge_address: Option<Address>,
    deposited_value: f64,
    metrics: SimMetrics,
}

impl AerodromeLp {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        Ok(AerodromeLp {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            position_token_id: None,
            gauge_address: None,
            deposited_value: 0.0,
            metrics: SimMetrics::default(),
        })
    }

    fn parse_pool_tokens(pool: &str) -> Result<(Address, Address)> {
        let parts: Vec<&str> = pool.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid pool format '{}', expected 'TOKEN0/TOKEN1'", pool);
        }
        let chain = Chain::base();
        let token0 = evm::token_address(&chain, parts[0])
            .with_context(|| format!("Unknown token '{}' on Base", parts[0]))?;
        let token1 = evm::token_address(&chain, parts[1])
            .with_context(|| format!("Unknown token '{}' on Base", parts[1]))?;
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
        let (token0, token1) = Self::parse_pool_tokens(pool)?;
        let spacing = tick_spacing.unwrap_or(100) as i32;
        let lower = tick_lower.unwrap_or(-887220);
        let upper = tick_upper.unwrap_or(887220);
        let position_manager = evm::aerodrome_position_manager();

        let half_amount = input_amount / 2.0;
        let amount0 = evm::to_token_units(half_amount, 1.0, 18);
        let amount1 = evm::to_token_units(half_amount, 1.0, 18);

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
            println!("  AERO: [DRY RUN] would approve tokens + mint position");
            self.deposited_value += input_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(Chain::base().rpc_url().unwrap().parse()?);

        let erc20_0 = IERC20::new(token0, &provider);
        erc20_0
            .approve(position_manager, amount0)
            .send()
            .await
            .context("approve token0")?
            .get_receipt()
            .await?;

        let erc20_1 = IERC20::new(token1, &provider);
        erc20_1
            .approve(position_manager, amount1)
            .send()
            .await
            .context("approve token1")?
            .get_receipt()
            .await?;

        let deadline = U256::from(chrono::Utc::now().timestamp() as u64 + 300);
        let pm = INonfungiblePositionManager::new(position_manager, &provider);
        let params = INonfungiblePositionManager::MintParams {
            token0,
            token1,
            tickSpacing: Signed::<24, 1>::try_from(spacing).unwrap_or(Signed::<24, 1>::try_from(100).unwrap()),
            tickLower: Signed::<24, 1>::try_from(lower).unwrap_or(Signed::<24, 1>::try_from(-887220).unwrap()),
            tickUpper: Signed::<24, 1>::try_from(upper).unwrap_or(Signed::<24, 1>::try_from(887220).unwrap()),
            amount0Desired: amount0,
            amount1Desired: amount1,
            amount0Min: U256::ZERO,
            amount1Min: U256::ZERO,
            recipient: self.wallet_address,
            deadline,
            sqrtPriceX96: Uint::<160, 3>::ZERO,
        };

        let result = pm.mint(params).send().await.context("mint LP position")?;
        let receipt = result.get_receipt().await.context("mint receipt")?;
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
            let signer: alloy::signers::local::PrivateKeySigner = self
                .private_key
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
            let wallet = alloy::network::EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(Chain::base().rpc_url().unwrap().parse()?);

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
            } => match action {
                LpAction::AddLiquidity => {
                    self.execute_add_liquidity(pool, *tick_lower, *tick_upper, *tick_spacing, input_amount)
                        .await
                }
                LpAction::RemoveLiquidity => self.execute_remove_liquidity(pool).await,
                LpAction::ClaimRewards => self.execute_claim_rewards(pool).await,
                LpAction::StakeGauge => self.execute_stake_gauge(pool).await,
                LpAction::UnstakeGauge => self.execute_unstake_gauge(pool).await,
                LpAction::Compound => self.execute_compound(pool).await,
            },
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
            println!(
                "  AERO TICK: deposited=${:.2}",
                self.deposited_value,
            );
        }
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lp_fees: self.metrics.lp_fees,
            ..SimMetrics::default()
        }
    }
}
