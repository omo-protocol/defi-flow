use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::chain::Chain;
use crate::model::node::{LendingAction, LendingVenue, Node};
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

// ── Aave V3 Pool interface (works for HyperLend, Aave, Lendle) ────

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
    supplied_value: f64,
    borrowed_value: f64,
    metrics: SimMetrics,
}

impl AaveLending {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        Ok(AaveLending {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            supplied_value: 0.0,
            borrowed_value: 0.0,
            metrics: SimMetrics::default(),
        })
    }

    fn venue_chain(venue: &LendingVenue) -> Chain {
        match venue {
            LendingVenue::HyperLend => Chain::HyperEvm,
            LendingVenue::Aave => Chain::Ethereum,
            LendingVenue::Lendle => Chain::Mantle,
            LendingVenue::Morpho => Chain::Ethereum,
            LendingVenue::Compound => Chain::Ethereum,
            LendingVenue::InitCapital => Chain::Mantle,
        }
    }

    fn venue_name(venue: &LendingVenue) -> &'static str {
        match venue {
            LendingVenue::HyperLend => "hyperlend",
            LendingVenue::Aave => "aave",
            LendingVenue::Lendle => "lendle",
            LendingVenue::Morpho => "morpho",
            LendingVenue::Compound => "compound",
            LendingVenue::InitCapital => "initcapital",
        }
    }

    async fn execute_supply(
        &mut self,
        venue: &LendingVenue,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let chain = Self::venue_chain(venue);
        let venue_name = Self::venue_name(venue);

        let pool_addr = evm::lending_pool_address(&chain, venue_name)
            .with_context(|| format!("No pool address for {venue_name} on {chain}"))?;

        let token_addr = evm::token_address(&chain, asset_symbol)
            .with_context(|| format!("Unknown token '{asset_symbol}' on {chain}"))?;

        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING SUPPLY: {} {} to {} on {} (pool: {})",
            input_amount,
            asset_symbol,
            venue_name,
            chain,
            evm::short_addr(&pool_addr),
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would approve {} + supply to pool", asset_symbol);
            self.supplied_value += input_amount;
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
            .connect_http(evm::rpc_url(&chain).parse()?);

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
        venue: &LendingVenue,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let chain = Self::venue_chain(venue);
        let venue_name = Self::venue_name(venue);

        let pool_addr = evm::lending_pool_address(&chain, venue_name)
            .with_context(|| format!("No pool address for {venue_name} on {chain}"))?;

        let token_addr = evm::token_address(&chain, asset_symbol)
            .with_context(|| format!("Unknown token '{asset_symbol}' on {chain}"))?;

        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING WITHDRAW: {} {} from {} on {}",
            input_amount, asset_symbol, venue_name, chain,
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would withdraw from pool");
            self.supplied_value = (self.supplied_value - input_amount).max(0.0);
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(evm::rpc_url(&chain).parse()?);

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
        venue: &LendingVenue,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let chain = Self::venue_chain(venue);
        let venue_name = Self::venue_name(venue);

        let pool_addr = evm::lending_pool_address(&chain, venue_name)
            .with_context(|| format!("No pool address for {venue_name} on {chain}"))?;

        let token_addr = evm::token_address(&chain, asset_symbol)
            .with_context(|| format!("Unknown token '{asset_symbol}' on {chain}"))?;

        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING BORROW: {} {} from {} on {}",
            input_amount, asset_symbol, venue_name, chain,
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would borrow from pool");
            self.borrowed_value += input_amount;
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(evm::rpc_url(&chain).parse()?);

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
        venue: &LendingVenue,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let chain = Self::venue_chain(venue);
        let venue_name = Self::venue_name(venue);

        let pool_addr = evm::lending_pool_address(&chain, venue_name)
            .with_context(|| format!("No pool address for {venue_name} on {chain}"))?;

        let token_addr = evm::token_address(&chain, asset_symbol)
            .with_context(|| format!("Unknown token '{asset_symbol}' on {chain}"))?;

        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  LENDING REPAY: {} {} to {} on {}",
            input_amount, asset_symbol, venue_name, chain,
        );

        if self.dry_run {
            println!("  LENDING: [DRY RUN] would approve + repay to pool");
            self.borrowed_value = (self.borrowed_value - input_amount).max(0.0);
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
            .connect_http(evm::rpc_url(&chain).parse()?);

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
        venue: &LendingVenue,
        asset_symbol: &str,
    ) -> Result<ExecutionResult> {
        let chain = Self::venue_chain(venue);
        let venue_name = Self::venue_name(venue);

        let rewards_addr = evm::rewards_controller_address(&chain, venue_name);

        println!(
            "  LENDING CLAIM: rewards from {} on {} for {}",
            venue_name, chain, asset_symbol,
        );

        if self.dry_run || rewards_addr.is_none() {
            if rewards_addr.is_none() {
                println!("  LENDING: no rewards controller address for {venue_name}, skipping");
            } else {
                println!("  LENDING: [DRY RUN] would claim rewards");
            }
            return Ok(ExecutionResult::Noop);
        }

        let rewards_addr = rewards_addr.unwrap();
        let token_addr = evm::token_address(&chain, asset_symbol)
            .unwrap_or(Address::ZERO);

        let signer: alloy::signers::local::PrivateKeySigner = self
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
        let wallet = alloy::network::EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(evm::rpc_url(&chain).parse()?);

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
                venue,
                asset,
                action,
                ..
            } => match action {
                LendingAction::Supply => {
                    self.execute_supply(venue, asset, input_amount).await
                }
                LendingAction::Withdraw => {
                    self.execute_withdraw(venue, asset, input_amount).await
                }
                LendingAction::Borrow => {
                    self.execute_borrow(venue, asset, input_amount).await
                }
                LendingAction::Repay => {
                    self.execute_repay(venue, asset, input_amount).await
                }
                LendingAction::ClaimRewards => {
                    self.execute_claim_rewards(venue, asset).await
                }
            },
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

fn token_decimals_for(symbol: &str) -> u8 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        _ => 18,
    }
}
