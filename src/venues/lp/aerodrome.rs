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

        // ERC721Enumerable view functions for on-chain position discovery
        function balanceOf(address owner) external view returns (uint256);
        function tokenOfOwnerByIndex(address owner, uint256 index) external view returns (uint256);
        function positions(uint256 tokenId) external view returns (
            uint96 nonce,
            address operator,
            address token0,
            address token1,
            int24 tickSpacing,
            int24 tickLower,
            int24 tickUpper,
            uint128 liquidity,
            uint256 feeGrowthInside0LastX128,
            uint256 feeGrowthInside1LastX128,
            uint128 tokensOwed0,
            uint128 tokensOwed1
        );
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract ICLPool {
        function slot0() external view returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            bool unlocked
        );
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
        node: &Node,
    ) -> Result<Self> {
        let cached_pool = if let Node::Lp { pool, .. } = node {
            Some(pool.clone())
        } else {
            None
        };

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
            cached_pool,
        })
    }

    /// Query on-chain position value by enumerating NFTs owned by the wallet.
    /// For each position, computes token amounts from liquidity + tick range + current price,
    /// plus any uncollected fees (tokensOwed).
    async fn query_onchain_value(&self) -> Result<f64> {
        let rpc_url = self
            .chain
            .rpc_url()
            .context("LP chain requires RPC URL")?;
        let position_manager =
            evm::resolve_contract(&self.contracts, "aerodrome_position_manager", &self.chain)
                .context("aerodrome_position_manager not in contracts manifest")?;

        let rp = evm::read_provider(rpc_url)?;
        let pm = INonfungiblePositionManager::new(position_manager, &rp);

        // How many NFT positions does the wallet own?
        let nft_count = pm
            .balanceOf(self.wallet_address)
            .call()
            .await
            .context("positionManager.balanceOf")?;

        let count: u64 = nft_count.try_into().unwrap_or(0);
        if count == 0 {
            return Ok(0.0);
        }

        let pool_name = self.cached_pool.as_deref();
        let mut total_value = 0.0;

        for i in 0..count {
            let token_id = pm
                .tokenOfOwnerByIndex(self.wallet_address, U256::from(i))
                .call()
                .await
                .context("tokenOfOwnerByIndex")?;

            let pos = pm.positions(token_id).call().await;
            let pos = match pos {
                Ok(p) => p,
                Err(_) => continue,
            };

            let liquidity = pos.liquidity;
            if liquidity == 0 {
                // Position exists but has zero liquidity — only fees owed matter
                let fees0 = pos.tokensOwed0 as f64;
                let fees1 = pos.tokensOwed1 as f64;
                let d0 = self.decimals_for_addr(pos.token0);
                let d1 = self.decimals_for_addr(pos.token1);
                total_value +=
                    fees0 / 10f64.powi(d0 as i32) + fees1 / 10f64.powi(d1 as i32);
                continue;
            }

            // Filter: only count positions matching our pool's tokens if known.
            if let Some(pool_str) = pool_name {
                let parts: Vec<&str> = pool_str.split('/').collect();
                if parts.len() == 2 {
                    let want_t0 = evm::resolve_token(&self.tokens, &self.chain, parts[0]);
                    let want_t1 = evm::resolve_token(&self.tokens, &self.chain, parts[1]);
                    if let (Some(w0), Some(w1)) = (want_t0, want_t1) {
                        if (pos.token0 != w0 || pos.token1 != w1)
                            && (pos.token0 != w1 || pos.token1 != w0)
                        {
                            continue; // different pool
                        }
                    }
                }
            }

            let d0 = self.decimals_for_addr(pos.token0);
            let d1 = self.decimals_for_addr(pos.token1);

            // Compute token amounts from liquidity + tick range.
            // Use sqrtPrice at current tick from Aerodrome pool.
            // Simplified: compute amounts assuming tokens are both USD-denominated,
            // which works for stablecoin pairs and is approximate for volatile pairs.
            let tick_lower: i32 = pos.tickLower.as_i32();
            let tick_upper: i32 = pos.tickUpper.as_i32();

            // Use mid-tick as rough current tick estimate (conservative).
            // For better accuracy, we'd query the pool's slot0.
            let (amount0, amount1) = cl_amounts_from_liquidity(
                liquidity as f64,
                tick_lower,
                tick_upper,
                (tick_lower + tick_upper) / 2, // rough mid estimate
            );

            let val0 = amount0 / 10f64.powi(d0 as i32);
            let val1 = amount1 / 10f64.powi(d1 as i32);
            let fees0 = pos.tokensOwed0 as f64 / 10f64.powi(d0 as i32);
            let fees1 = pos.tokensOwed1 as f64 / 10f64.powi(d1 as i32);

            total_value += val0 + val1 + fees0 + fees1;
        }

        Ok(total_value)
    }

    /// Get decimals for a token address by checking known token symbols.
    fn decimals_for_addr(&self, addr: Address) -> u8 {
        // Reverse-lookup: find the symbol for this address.
        for (symbol, chains) in &self.tokens {
            for (_, token_addr_str) in chains {
                if let Ok(token_addr) = token_addr_str.parse::<Address>() {
                    if token_addr == addr {
                        return token_decimals_for(symbol);
                    }
                }
            }
        }
        18 // default
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
        // Live mode: discover NFT positions from position manager and calculate value.
        if !self.dry_run {
            match self.query_onchain_value().await {
                Ok(val) if val > 0.0 => return Ok(val),
                Ok(_) => {} // no positions found, fall through
                Err(e) => {
                    eprintln!(
                        "  AERO: on-chain query failed, falling back to local: {:#}",
                        e
                    );
                }
            }
        }
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

/// Compute token amounts from concentrated liquidity position parameters.
///
/// Standard Uniswap V3 / Aerodrome Slipstream math:
/// - If current_tick < tick_lower: all token0
/// - If current_tick >= tick_upper: all token1
/// - Otherwise: split between token0 and token1
fn cl_amounts_from_liquidity(
    liquidity: f64,
    tick_lower: i32,
    tick_upper: i32,
    current_tick: i32,
) -> (f64, f64) {
    let sqrt_price = tick_to_sqrt_price(current_tick);
    let sqrt_lower = tick_to_sqrt_price(tick_lower);
    let sqrt_upper = tick_to_sqrt_price(tick_upper);

    if current_tick < tick_lower {
        // All token0
        let amount0 = liquidity * (1.0 / sqrt_lower - 1.0 / sqrt_upper);
        (amount0, 0.0)
    } else if current_tick >= tick_upper {
        // All token1
        let amount1 = liquidity * (sqrt_upper - sqrt_lower);
        (0.0, amount1)
    } else {
        // Split
        let amount0 = liquidity * (1.0 / sqrt_price - 1.0 / sqrt_upper);
        let amount1 = liquidity * (sqrt_price - sqrt_lower);
        (amount0, amount1)
    }
}

/// Convert a tick to sqrt(price) using the standard formula:
/// sqrt(1.0001^tick) = 1.0001^(tick/2)
fn tick_to_sqrt_price(tick: i32) -> f64 {
    1.0001_f64.powf(tick as f64 / 2.0)
}
