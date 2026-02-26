use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::reserve::ReserveConfig;
use crate::venues::evm;

use super::Engine;

// ── ERC4626 + ERC20 read interfaces ─────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IErc4626Read {
        function totalAssets() external view returns (uint256);
        function asset() external view returns (address);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IErc20Read {
        function balanceOf(address account) external view returns (uint256);
        function decimals() external view returns (uint8);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IErc20Transfer {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

// ── Types ────────────────────────────────────────────────────────────

/// On-chain vault state.
#[derive(Debug, Clone)]
pub struct VaultState {
    pub total_assets: f64,
    pub idle_balance: f64,
    pub reserve_ratio: f64,
}

/// Record of a reserve management action (serialized in RunState).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveActionRecord {
    pub timestamp: u64,
    pub total_assets: f64,
    pub reserve_ratio: f64,
    pub deficit: f64,
    pub freed: f64,
}

// ── Read vault state ─────────────────────────────────────────────────

/// Read on-chain vault state: totalAssets, idle balance, reserve ratio.
pub async fn read_vault_state(
    config: &ReserveConfig,
    contracts: &evm::ContractManifest,
) -> Result<VaultState> {
    let rpc_url = config
        .vault_chain
        .rpc_url()
        .context("vault chain requires RPC URL")?;

    let vault_addr = evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
        .with_context(|| {
        format!(
            "Vault '{}' on {} not in contracts manifest",
            config.vault_address, config.vault_chain,
        )
    })?;

    let rp = evm::read_provider(rpc_url)?;

    // Read totalAssets from the ERC4626 vault
    let vault = IErc4626Read::new(vault_addr, &rp);
    let total_assets_raw = vault
        .totalAssets()
        .call()
        .await
        .context("vault.totalAssets() call failed")?;

    // Get underlying token address and decimals
    let underlying = vault
        .asset()
        .call()
        .await
        .context("vault.asset() call failed")?;

    let erc20 = IErc20Read::new(underlying, &rp);
    let decimals = erc20
        .decimals()
        .call()
        .await
        .context("underlying.decimals() call failed")?;

    // Read idle balance (vault's underlying token balance = reserve)
    let idle_raw = erc20
        .balanceOf(vault_addr)
        .call()
        .await
        .context("underlying.balanceOf(vault) call failed")?;

    let total_assets = evm::from_token_units(total_assets_raw, decimals);
    let idle_balance = evm::from_token_units(idle_raw, decimals);

    let reserve_ratio = if total_assets > 0.0 {
        idle_balance / total_assets
    } else {
        1.0 // empty vault = fully reserved
    };

    Ok(VaultState {
        total_assets,
        idle_balance,
        reserve_ratio,
    })
}

// ── Reserve management ───────────────────────────────────────────────

/// Check vault reserve and unwind venues if depleted.
///
/// Returns `Some(ReserveActionRecord)` if an unwind was performed,
/// `None` if the reserve is healthy.
pub async fn check_and_manage(
    engine: &mut Engine,
    config: &ReserveConfig,
    contracts: &evm::ContractManifest,
    tokens: &evm::TokenManifest,
    private_key: &str,
    dry_run: bool,
) -> Result<Option<ReserveActionRecord>> {
    let state = read_vault_state(config, contracts).await?;

    eprintln!(
        "[reserve] vault: total=${:.2}, idle=${:.2}, ratio={:.1}% (trigger={:.1}%)",
        state.total_assets,
        state.idle_balance,
        state.reserve_ratio * 100.0,
        config.trigger_threshold * 100.0,
    );

    // Reserve is healthy — do nothing
    if state.reserve_ratio >= config.trigger_threshold {
        return Ok(None);
    }

    // Reserve depleted — compute deficit to reach target
    let target_idle = state.total_assets * config.target_ratio;
    let deficit = target_idle - state.idle_balance;

    if deficit < config.min_unwind {
        eprintln!(
            "[reserve] deficit ${:.2} below min_unwind ${:.2}, skipping",
            deficit, config.min_unwind,
        );
        return Ok(None);
    }

    eprintln!(
        "[reserve] DEPLETED: ratio={:.1}%, deficit=${:.2}, target_idle=${:.2}",
        state.reserve_ratio * 100.0,
        deficit,
        target_idle,
    );

    // Try optimizer-aware unwind (takes more from low-alpha groups),
    // fall back to flat pro-rata if no optimizer node exists.
    let total_freed = match engine.optimizer_unwind(deficit).await {
        Ok(freed) => freed,
        Err(_) => flat_pro_rata_unwind(engine, deficit).await,
    };

    if total_freed > 0.0 {
        let vault_addr =
            evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
                .unwrap_or_default();

        if dry_run {
            eprintln!(
                "[reserve] [DRY RUN] would transfer ${:.2} to vault {}",
                total_freed,
                evm::short_addr(&vault_addr),
            );
        } else {
            match transfer_to_vault(config, contracts, tokens, private_key, total_freed).await {
                Ok(()) => {
                    eprintln!(
                        "[reserve] transferred ${:.2} to vault {}",
                        total_freed,
                        evm::short_addr(&vault_addr),
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[reserve] ERROR transferring to vault: {:#}. \
                         Freed capital sits in wallet — keeper deposit cycle will pick it up.",
                        e,
                    );
                }
            }
        }
    }

    let now = chrono::Utc::now().timestamp() as u64;
    Ok(Some(ReserveActionRecord {
        timestamp: now,
        total_assets: state.total_assets,
        reserve_ratio: state.reserve_ratio,
        deficit,
        freed: total_freed,
    }))
}

// ── ERC20 transfer to vault ─────────────────────────────────────────

/// Transfer freed capital (vault_token) to the vault address on-chain.
async fn transfer_to_vault(
    config: &ReserveConfig,
    contracts: &evm::ContractManifest,
    tokens: &evm::TokenManifest,
    private_key: &str,
    amount: f64,
) -> Result<()> {
    let rpc_url = config
        .vault_chain
        .rpc_url()
        .context("vault chain requires RPC URL for transfer")?;

    let vault_addr = evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
        .context("vault address not in contracts manifest")?;

    let token_addr = evm::resolve_token(tokens, &config.vault_chain, &config.vault_token)
        .with_context(|| {
            format!(
                "Token '{}' on {} not in tokens manifest",
                config.vault_token, config.vault_chain,
            )
        })?;

    let decimals = token_decimals_for(&config.vault_token);
    let amount_units = evm::to_token_units(amount, 1.0, decimals);

    let provider = make_signer_provider(private_key, rpc_url)?;
    let erc20 = IErc20Transfer::new(token_addr, &provider);

    let pending = erc20
        .transfer(vault_addr, amount_units)
        .send()
        .await
        .context("ERC20 transfer to vault failed")?;

    let receipt = pending
        .get_receipt()
        .await
        .context("transfer receipt failed")?;

    if !receipt.status() {
        anyhow::bail!(
            "ERC20 transfer reverted (tx: {:?}, gas: {:?})",
            receipt.transaction_hash,
            receipt.gas_used,
        );
    }

    eprintln!("[reserve] transfer tx: {:?}", receipt.transaction_hash,);

    Ok(())
}

fn make_signer_provider(
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

/// Flat pro-rata unwind: same fraction from every venue.
/// Used as fallback when no optimizer node exists.
async fn flat_pro_rata_unwind(engine: &mut Engine, deficit: f64) -> f64 {
    let mut total_venue_value = 0.0;
    let venue_ids: Vec<String> = engine.venues.keys().cloned().collect();
    for id in &venue_ids {
        if let Some(venue) = engine.venues.get(id.as_str()) {
            total_venue_value += venue.total_value().await.unwrap_or(0.0);
        }
    }

    if total_venue_value <= 0.0 {
        eprintln!("[reserve] no venue positions to unwind");
        return 0.0;
    }

    let unwind_fraction = (deficit / total_venue_value).min(1.0);
    eprintln!(
        "[reserve] flat pro-rata: unwinding {:.1}% from all venues (venue_total=${:.2})",
        unwind_fraction * 100.0,
        total_venue_value,
    );

    let mut total_freed = 0.0;
    for id in &venue_ids {
        if let Some(venue) = engine.venues.get_mut(id.as_str()) {
            let freed = venue.unwind(unwind_fraction).await.unwrap_or(0.0);
            if freed > 0.0 {
                total_freed += freed;
                eprintln!("[reserve]   {} → freed ${:.2}", id, freed);
            }
        }
    }

    total_freed
}

fn token_decimals_for(symbol: &str) -> u8 {
    match symbol.to_uppercase().as_str() {
        "USDC" | "USDT" => 6,
        "WBTC" | "CBBTC" => 8,
        _ => 18,
    }
}
