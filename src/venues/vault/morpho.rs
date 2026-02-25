use alloy::primitives::Address;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::model::node::{Node, VaultAction};
use crate::run::config::RuntimeConfig;
use crate::venues::evm;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

// ── Morpho Vault V2 interface (ERC4626) ─────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IMorphoVault {
        function deposit(uint256 assets, address receiver) external returns (uint256 shares);
        function withdraw(uint256 assets, address receiver, address owner) external returns (uint256 shares);
        function redeem(uint256 shares, address receiver, address owner) external returns (uint256 assets);
        function maxWithdraw(address owner) external view returns (uint256);
        function convertToAssets(uint256 shares) external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function asset() external view returns (address);
        function decimals() external view returns (uint8);
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

// ── Morpho Vault V2 Live Executor ───────────────────────────────────

/// Cached node context for unwind() — resolved on first execute().
struct CachedVaultContext {
    vault_addr: Address,
    rpc_url: String,
    asset_symbol: String,
}

pub struct MorphoVault {
    wallet_address: Address,
    private_key: String,
    dry_run: bool,
    tokens: evm::TokenManifest,
    contracts: evm::ContractManifest,
    deposited_value: f64,
    metrics: SimMetrics,
    cached_ctx: Option<CachedVaultContext>,
}

impl MorphoVault {
    pub fn new(
        config: &RuntimeConfig,
        tokens: &evm::TokenManifest,
        contracts: &evm::ContractManifest,
    ) -> Result<Self> {
        Ok(MorphoVault {
            wallet_address: config.wallet_address,
            private_key: config.private_key.clone(),
            dry_run: config.dry_run,
            tokens: tokens.clone(),
            contracts: contracts.clone(),
            deposited_value: 0.0,
            metrics: SimMetrics::default(),
            cached_ctx: None,
        })
    }

    async fn execute_deposit(
        &mut self,
        vault_addr: Address,
        rpc_url: &str,
        token_addr: Address,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  VAULT DEPOSIT: {} {} to vault {}",
            input_amount, asset_symbol, evm::short_addr(&vault_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify vault asset matches expected token ──
            let rp = evm::read_provider(rpc_url)?;
            let vault = IMorphoVault::new(vault_addr, &rp);
            let underlying = vault.asset().call().await
                .context("vault.asset() call failed — not a valid ERC4626 vault")?;
            if underlying != token_addr {
                anyhow::bail!(
                    "Vault asset mismatch: vault {} has underlying {} but expected {} ({})",
                    evm::short_addr(&vault_addr),
                    evm::short_addr(&underlying),
                    asset_symbol,
                    evm::short_addr(&token_addr),
                );
            }
            println!("  VAULT: preflight OK — vault asset matches {}", asset_symbol);
            println!("  VAULT: [DRY RUN] would approve {} + deposit to vault", asset_symbol);
            self.deposited_value += input_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        // Approve vault to spend underlying token
        let erc20 = IERC20::new(token_addr, &provider);
        let approve_tx = erc20.approve(vault_addr, amount_units);
        let pending = approve_tx.send().await.context("ERC20 approve failed")?;
        let receipt = pending.get_receipt().await.context("approve receipt")?;
        require_success(&receipt, "vault-approve")?;
        println!("  VAULT: approve tx: {:?}", receipt.transaction_hash);

        // Deposit into vault (ERC4626)
        let vault = IMorphoVault::new(vault_addr, &provider);
        let deposit_tx = vault.deposit(amount_units, self.wallet_address).gas(500_000);
        let pending = deposit_tx.send().await.context("vault deposit failed")?;
        let receipt = pending.get_receipt().await.context("deposit receipt")?;
        require_success(&receipt, "vault-deposit")?;
        println!("  VAULT: deposit tx: {:?}", receipt.transaction_hash);

        self.deposited_value += input_amount;
        Ok(ExecutionResult::PositionUpdate {
            consumed: input_amount,
            output: None,
        })
    }

    async fn execute_withdraw(
        &mut self,
        vault_addr: Address,
        rpc_url: &str,
        asset_symbol: &str,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let decimals = token_decimals_for(asset_symbol);
        let amount_units = evm::to_token_units(input_amount, 1.0, decimals);

        println!(
            "  VAULT WITHDRAW: {} {} from vault {}",
            input_amount, asset_symbol, evm::short_addr(&vault_addr),
        );

        if self.dry_run {
            // ── Preflight reads: verify vault is valid ERC4626 ──
            let rp = evm::read_provider(rpc_url)?;
            let vault = IMorphoVault::new(vault_addr, &rp);
            vault.asset().call().await
                .context("vault.asset() call failed — not a valid ERC4626 vault")?;
            println!("  VAULT: preflight OK — vault responds to ERC4626 interface");
            println!("  VAULT: [DRY RUN] would withdraw from vault");
            self.deposited_value = (self.deposited_value - input_amount).max(0.0);
            return Ok(ExecutionResult::TokenOutput {
                token: asset_symbol.to_string(),
                amount: input_amount,
            });
        }

        let provider = make_provider(&self.private_key, rpc_url)?;

        let vault = IMorphoVault::new(vault_addr, &provider);
        let withdraw_tx = vault
            .withdraw(amount_units, self.wallet_address, self.wallet_address)
            .gas(500_000);
        let pending = withdraw_tx.send().await.context("vault withdraw failed")?;
        let receipt = pending.get_receipt().await.context("withdraw receipt")?;
        require_success(&receipt, "vault-withdraw")?;
        println!("  VAULT: withdraw tx: {:?}", receipt.transaction_hash);

        self.deposited_value = (self.deposited_value - input_amount).max(0.0);
        Ok(ExecutionResult::TokenOutput {
            token: asset_symbol.to_string(),
            amount: input_amount,
        })
    }
}

#[async_trait]
impl Venue for MorphoVault {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Vault {
                chain,
                vault_address,
                asset,
                action,
                ..
            } => {
                let rpc_url = chain
                    .rpc_url()
                    .context("vault chain requires RPC URL")?;
                let vault_addr = evm::resolve_contract(&self.contracts, vault_address, chain)
                    .with_context(|| format!("Contract '{}' on {} not in contracts manifest", vault_address, chain))?;
                let token_addr = evm::resolve_token(&self.tokens, chain, asset)
                    .with_context(|| format!("Token '{asset}' on {chain} not in tokens manifest"))?;

                // Cache context for unwind()
                self.cached_ctx = Some(CachedVaultContext {
                    vault_addr,
                    rpc_url: rpc_url.to_string(),
                    asset_symbol: asset.clone(),
                });

                match action {
                    VaultAction::Deposit => {
                        self.execute_deposit(vault_addr, rpc_url, token_addr, asset, input_amount).await
                    }
                    VaultAction::Withdraw => {
                        self.execute_withdraw(vault_addr, rpc_url, asset, input_amount).await
                    }
                    VaultAction::ClaimRewards => {
                        // Morpho rewards are claimed via off-chain merkle proofs
                        // through the Universal Rewards Distributor — not automated here yet.
                        println!("  VAULT: claim_rewards not yet automated for Morpho V2");
                        Ok(ExecutionResult::Noop)
                    }
                }
            }
            _ => {
                println!("  VAULT: unsupported node type '{}'", node.type_name());
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
                "  VAULT TICK: deposited=${:.2}",
                self.deposited_value,
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

        println!("  VAULT: UNWIND {:.1}% (${:.2})", f * 100.0, withdraw_amount);

        let ctx = self.cached_ctx.as_ref()
            .context("unwind() called before execute() — no cached vault context")?;

        // Reuses execute_withdraw() which handles dry_run (preflight only) vs live (actual tx)
        self.execute_withdraw(
            ctx.vault_addr,
            &ctx.rpc_url.clone(),
            &ctx.asset_symbol.clone(),
            withdraw_amount,
        ).await?;

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

fn token_decimals_for(symbol: &str) -> u8 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        _ => 18,
    }
}
