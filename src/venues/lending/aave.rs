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

// ── Aave Lending ──────────────────────────────────────────────────

pub struct AaveLending {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    tokens: evm::TokenManifest,
    contracts: evm::ContractManifest,
    supplied_value: f64,
    borrowed_value: f64,
    metrics: SimMetrics,
}

impl AaveLending {
    pub fn new(
        config: &RuntimeConfig,
        tokens: &evm::TokenManifest,
        contracts: &evm::ContractManifest,
    ) -> Result<Self> {
        Ok(AaveLending {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            contracts: contracts.clone(),
            supplied_value: 0.0,
            borrowed_value: 0.0,
            metrics: SimMetrics::default(),
        })
    }

    async fn execute_supply(
        &mut self,
        pool_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING SUPPLY: {} {} to pool {}",
            input_amount, asset_symbol, evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would approve {} + supply to pool", asset_symbol);
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
        println!("  LENDING: approve tx: {:?}", receipt.transaction_hash);

        let pool = IAavePool::new(pool_addr, &provider);
        let supply_tx = pool.supply(token_addr, amount_units, self.wallet_address, 0);
        let pending = supply_tx.send().await.context("supply failed")?;
        let receipt = pending.get_receipt().await.context("supply receipt")?;
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
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING WITHDRAW: {} {} from pool {}",
            input_amount, asset_symbol, evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would withdraw from pool");
            self.supplied_value = (self.supplied_value - input_amount).max(0.0);
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let pool = IAavePool::new(pool_addr, &provider);
        let withdraw_tx = pool.withdraw(token_addr, amount_units, self.wallet_address);
        let pending = withdraw_tx.send().await.context("withdraw failed")?;
        let receipt = pending.get_receipt().await.context("withdraw receipt")?;
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
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING BORROW: {} {} from pool {}",
            input_amount, asset_symbol, evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would borrow from pool");
            self.borrowed_value += input_amount;
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let pool = IAavePool::new(pool_addr, &provider);
        let borrow_tx = pool.borrow(token_addr, amount_units, U256::from(2), 0, self.wallet_address);
        let pending = borrow_tx.send().await.context("borrow failed")?;
        let receipt = pending.get_receipt().await.context("borrow receipt")?;
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
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING REPAY: {} {} to pool {}",
            input_amount, asset_symbol, evm::short_addr(&pool_addr),
        );

        if self.dry_run {
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
        pending.get_receipt().await.context("approve receipt")?;

        let pool = IAavePool::new(pool_addr, &provider);
        let repay_tx = pool.repay(token_addr, amount_units, U256::from(2), self.wallet_address);
        let pending = repay_tx.send().await.context("repay failed")?;
        let receipt = pending.get_receipt().await.context("repay receipt")?;
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

        let rewards_addr = evm::resolve_contract(&self.contracts, rc_name, chain)
            .with_context(|| format!("Contract '{}' on {} not in contracts manifest", rc_name, chain))?;

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
                let rpc_url = chain
                    .rpc_url()
                    .context("lending chain requires RPC URL")?;
                let pool_addr = evm::resolve_contract(&self.contracts, pool_address, chain)
                    .with_context(|| format!("Contract '{}' on {} not in contracts manifest", pool_address, chain))?;
                let token_addr = evm::resolve_token(&self.tokens, chain, asset)
                    .with_context(|| format!("Token '{asset}' on {chain} not in tokens manifest"))?;

                match action {
                    LendingAction::Supply => {
                        self.execute_supply(pool_addr, rpc_url, token_addr, asset, input_amount).await
                    }
                    LendingAction::Withdraw => {
                        self.execute_withdraw(pool_addr, rpc_url, token_addr, asset, input_amount).await
                    }
                    LendingAction::Borrow => {
                        self.execute_borrow(pool_addr, rpc_url, token_addr, asset, input_amount).await
                    }
                    LendingAction::Repay => {
                        self.execute_repay(pool_addr, rpc_url, token_addr, asset, input_amount).await
                    }
                    LendingAction::ClaimRewards => {
                        self.execute_claim_rewards(rewards_controller.as_deref(), chain, rpc_url, token_addr).await
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

fn token_decimals_for(symbol: &str) -> u8 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        _ => 18,
    }
}
