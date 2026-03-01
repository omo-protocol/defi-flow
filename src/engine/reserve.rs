use alloy::primitives::{Address, Bytes, FixedBytes, U256};
use alloy::providers::ProviderBuilder;
use alloy::sol;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::reserve::ReserveConfig;
use crate::model::valuer::ValuerConfig;
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
    /// Morpho vaults on HyperEVM use _totalAssets()(uint128) as the real
    /// source of truth (selector 0xce04bebb). Standard totalAssets() may
    /// return 0 or revert.
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IMorphoVaultRead {
        function _totalAssets() external view returns (uint128);
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

// ── Vault allocate + adapter interfaces ─────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IVaultAllocate {
        function allocate(address adapter, bytes memory data, uint256 assets) external;
        function deallocate(address adapter, bytes memory data, uint256 assets) external;
        function totalSupply() external view returns (uint256);
    }
}

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IAdapter {
        struct Call {
            address target;
            bytes data;
            uint256 value;
        }
        function executeStrategyBypassCircuitBreaker(
            bytes32 strategyId,
            Call[] calls
        ) external;
        function refreshCachedValuation() external;
        function totalAllocations() external view returns (uint256);
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

/// Record of an allocation action — pulling excess funds from vault (serialized in RunState).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationRecord {
    pub timestamp: u64,
    pub vault_idle: f64,
    pub target_reserve: f64,
    pub excess: f64,
    pub pulled: f64,
}

// ── Retry helper ─────────────────────────────────────────────────────

const MAX_RETRIES: u32 = 3;
const BASE_DELAY_MS: u64 = 500;

/// Retry an async RPC call with exponential backoff (500ms, 1s, 2s).
async fn retry_rpc<F, Fut, T>(label: &str, f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, alloy::contract::Error>>,
{
    let mut last_err = None;
    for attempt in 0..=MAX_RETRIES {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = Some(e);
                if attempt < MAX_RETRIES {
                    let delay = BASE_DELAY_MS * (1 << attempt);
                    eprintln!(
                        "[reserve] {} failed (attempt {}/{}), retrying in {}ms...",
                        label,
                        attempt + 1,
                        MAX_RETRIES + 1,
                        delay,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
    }
    Err(last_err
        .map(|e| anyhow::anyhow!("{}: {}", label, e))
        .unwrap_or_else(|| anyhow::anyhow!("{}: unknown error", label)))
}

// ── Read vault state ─────────────────────────────────────────────────

/// Read on-chain vault state: totalAssets, idle balance, reserve ratio.
/// Uses exponential backoff (3 retries) for RPC resilience.
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

    // Read totalAssets with retry + backoff.
    // Morpho vaults on HyperEVM use _totalAssets()(uint128) as the real
    // source of truth. Standard totalAssets() may return 0 or revert.
    // Try _totalAssets first, then fall back to totalAssets.
    let morpho = IMorphoVaultRead::new(vault_addr, &rp);
    let vault = IErc4626Read::new(vault_addr, &rp);

    let total_assets_raw = match retry_rpc("_totalAssets()", || async {
        morpho._totalAssets().call().await
    })
    .await
    {
        Ok(v) => U256::from(v),
        Err(_) => {
            // Morpho selector not available — try standard ERC4626
            match retry_rpc("totalAssets()", || async {
                vault.totalAssets().call().await
            })
            .await
            {
                Ok(v) if v > U256::ZERO => v,
                Ok(_) => {
                    anyhow::bail!(
                        "vault totalAssets returned 0 — vault may be misconfigured"
                    );
                }
                Err(e) => {
                    anyhow::bail!(
                        "vault totalAssets unavailable after {} retries (RPC rate-limited?) — \
                         skipping allocator/reserve this tick: {:#}",
                        MAX_RETRIES + 1,
                        e,
                    );
                }
            }
        }
    };

    // Get underlying token address and decimals (with retry)
    let underlying = retry_rpc("vault.asset()", || async { vault.asset().call().await })
        .await
        .context("vault.asset() call failed after retries")?;

    let erc20 = IErc20Read::new(underlying, &rp);
    let decimals = retry_rpc("decimals()", || async { erc20.decimals().call().await })
        .await
        .context("underlying.decimals() call failed after retries")?;

    // Read idle balance (vault's underlying token balance = reserve)
    let idle_raw =
        retry_rpc("balanceOf(vault)", || async {
            erc20.balanceOf(vault_addr).call().await
        })
        .await
        .context("underlying.balanceOf(vault) call failed after retries")?;

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

/// Read adapter's totalAllocations() — the amount of underlying allocated to
/// strategies. Used as a fallback TVL estimate when vault.totalAssets() reverts
/// (e.g. unconfigured valuer) to bootstrap the initial valuation push.
pub async fn read_adapter_allocations(
    config: &ReserveConfig,
    contracts: &evm::ContractManifest,
) -> Result<f64> {
    let adapter_key = config
        .adapter_address
        .as_ref()
        .context("no adapter_address in reserve config")?;

    let adapter_addr =
        evm::resolve_contract(contracts, adapter_key, &config.vault_chain).with_context(|| {
            format!(
                "Adapter '{}' on {} not in contracts manifest",
                adapter_key, config.vault_chain,
            )
        })?;

    let rpc_url = config
        .vault_chain
        .rpc_url()
        .context("vault chain requires RPC URL")?;
    let rp = evm::read_provider(rpc_url)?;

    let adapter = IAdapter::new(adapter_addr, &rp);
    let total = retry_rpc("totalAllocations()", || async {
        adapter.totalAllocations().call().await
    })
    .await
    .context("adapter.totalAllocations() failed after retries")?;

    // Resolve decimals from the vault's underlying token
    let vault_addr = evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
        .context("vault address not in contracts manifest")?;
    let vault = IErc4626Read::new(vault_addr, &rp);
    let underlying = vault.asset().call().await.context("vault.asset()")?;
    let erc20 = IErc20Read::new(underlying, &rp);
    let decimals = erc20.decimals().call().await.context("decimals()")?;

    Ok(evm::from_token_units(total, decimals))
}

// ── Allocation: pull excess funds from vault ─────────────────────────

/// Check vault reserve and pull excess idle funds into the strategy wallet.
///
/// Logic: keep 20% (target_ratio) of vault TVL idle. If idle > target + 5% buffer,
/// pull the excess through: vault.allocate(adapter, data, excess) →
/// adapter.executeStrategyBypassCircuitBreaker(strategyId, [transfer(wallet, excess)]).
///
/// Returns `Some(AllocationRecord)` if funds were pulled, `None` if no excess.
pub async fn check_and_allocate(
    config: &ReserveConfig,
    valuer_config: Option<&ValuerConfig>,
    contracts: &evm::ContractManifest,
    tokens: &evm::TokenManifest,
    private_key: &str,
    wallet_address: Address,
    dry_run: bool,
) -> Result<Option<AllocationRecord>> {
    // Must have adapter configured
    let adapter_key = match &config.adapter_address {
        Some(k) => k,
        None => return Ok(None), // No adapter = can't pull
    };

    // Must have strategy_id from valuer config
    let strategy_id_text = match valuer_config {
        Some(vc) => &vc.strategy_id,
        None => {
            eprintln!("[allocator] No valuer config — can't derive strategy_id for adapter calls");
            return Ok(None);
        }
    };

    let state = read_vault_state(config, contracts).await?;

    let target_reserve = state.total_assets * config.target_ratio;
    let buffer = target_reserve * 0.05; // 5% buffer to prevent oscillation

    eprintln!(
        "[allocator] vault: total=${:.2}, idle=${:.2}, target_reserve=${:.2}, buffer=${:.2}",
        state.total_assets, state.idle_balance, target_reserve, buffer,
    );

    // Only pull if idle exceeds target + buffer
    if state.idle_balance <= target_reserve + buffer {
        return Ok(None);
    }

    let excess = state.idle_balance - target_reserve;

    // Don't pull dust
    if excess < config.min_unwind {
        eprintln!(
            "[allocator] excess ${:.2} below min_unwind ${:.2}, skipping",
            excess, config.min_unwind,
        );
        return Ok(None);
    }

    eprintln!(
        "[allocator] EXCESS: idle=${:.2}, target=${:.2}, pulling ${:.2}",
        state.idle_balance, target_reserve, excess,
    );

    if dry_run {
        eprintln!(
            "[allocator] [DRY RUN] would pull ${:.2} from vault via adapter",
            excess,
        );
        let now = chrono::Utc::now().timestamp() as u64;
        return Ok(Some(AllocationRecord {
            timestamp: now,
            vault_idle: state.idle_balance,
            target_reserve,
            excess,
            pulled: excess,
        }));
    }

    // Resolve addresses
    let adapter_addr =
        evm::resolve_contract(contracts, adapter_key, &config.vault_chain).with_context(|| {
            format!(
                "Adapter '{}' on {} not in contracts manifest",
                adapter_key, config.vault_chain,
            )
        })?;

    let vault_addr = evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
        .context("vault address not in contracts manifest")?;

    let token_addr = evm::resolve_token(tokens, &config.vault_chain, &config.vault_token)
        .with_context(|| {
            format!(
                "Token '{}' on {} not in tokens manifest",
                config.vault_token, config.vault_chain,
            )
        })?;

    let rpc_url = config
        .vault_chain
        .rpc_url()
        .context("vault chain requires RPC URL")?;

    let strategy_id = crate::run::valuer::strategy_id_from_text(strategy_id_text);
    let decimals = evm::query_decimals(rpc_url, token_addr).await?;
    let excess_units = evm::to_token_units(excess, 1.0, decimals);

    // Build signer provider
    let provider = make_signer_provider(private_key, rpc_url)?;

    // Step 1: vault.allocate(adapter, encodedData, excess)
    let allocation_data = encode_allocation_data(strategy_id);
    let vault = IVaultAllocate::new(vault_addr, &provider);

    let pending = vault
        .allocate(adapter_addr, allocation_data, excess_units)
        .gas(500_000)
        .send()
        .await
        .context("vault.allocate() send failed")?;

    let receipt = pending
        .get_receipt()
        .await
        .context("vault.allocate() receipt failed")?;

    if !receipt.status() {
        anyhow::bail!(
            "vault.allocate() reverted (tx: {:?})",
            receipt.transaction_hash,
        );
    }

    eprintln!(
        "[allocator] vault.allocate() tx: {:?}",
        receipt.transaction_hash,
    );

    // Step 2: adapter.executeStrategyBypassCircuitBreaker(strategyId, [transfer(wallet, excess)])
    // Build the ERC20 transfer call: token.transfer(wallet, excess)
    let transfer_calldata = IErc20Transfer::transferCall {
        to: wallet_address,
        amount: excess_units,
    };
    let transfer_bytes = Bytes::from(alloy::sol_types::SolCall::abi_encode(&transfer_calldata));

    let calls = vec![IAdapter::Call {
        target: token_addr,
        data: transfer_bytes,
        value: U256::ZERO,
    }];

    let adapter = IAdapter::new(adapter_addr, &provider);
    let pending = adapter
        .executeStrategyBypassCircuitBreaker(strategy_id, calls)
        .gas(500_000)
        .send()
        .await
        .context("adapter.executeStrategyBypassCircuitBreaker() send failed")?;

    let receipt = pending
        .get_receipt()
        .await
        .context("adapter.executeStrategyBypassCircuitBreaker() receipt failed")?;

    if !receipt.status() {
        anyhow::bail!(
            "adapter.executeStrategyBypassCircuitBreaker() reverted (tx: {:?})",
            receipt.transaction_hash,
        );
    }

    eprintln!(
        "[allocator] adapter.executeStrategy() tx: {:?}",
        receipt.transaction_hash,
    );

    let now = chrono::Utc::now().timestamp() as u64;
    Ok(Some(AllocationRecord {
        timestamp: now,
        vault_idle: state.idle_balance,
        target_reserve,
        excess,
        pulled: excess,
    }))
}

/// Encode allocation data matching Solidity's:
/// `abi.encode(bytes32 strategyId, uint256 0, bool false, Call[] [])`.
///
/// Manual encoding to match exact Solidity ABI layout:
/// [0..32]   bytes32 strategyId
/// [32..64]  uint256 0
/// [64..96]  bool false
/// [96..128] offset to Call[] (= 128)
/// [128..160] Call[] length (= 0)
fn encode_allocation_data(strategy_id: FixedBytes<32>) -> Bytes {
    let mut data = Vec::with_capacity(160);
    // bytes32 strategyId
    data.extend_from_slice(strategy_id.as_slice());
    // uint256 0
    data.extend_from_slice(&[0u8; 32]);
    // bool false
    data.extend_from_slice(&[0u8; 32]);
    // offset to Call[] dynamic data (= 4 * 32 = 128)
    let mut offset = [0u8; 32];
    offset[31] = 128;
    data.extend_from_slice(&offset);
    // Call[] length = 0
    data.extend_from_slice(&[0u8; 32]);
    Bytes::from(data)
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

    let decimals = evm::query_decimals(rpc_url, token_addr).await?;
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

// ── Adapter balance recovery (deallocate stranded funds) ─────────────

/// Record of a deallocate recovery action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeallocateRecord {
    pub timestamp: u64,
    pub adapter_balance: f64,
    pub deallocated: f64,
}

/// Check if the adapter has stranded USDT0 and deallocate it back to the vault.
///
/// When funds get stuck in the adapter (e.g. partial unwind, failed deploy),
/// `totalAllocations` remains inflated. This causes `refreshCachedValuation()`
/// to fail with "Valuation too low" because: `TVL < totalAllocations * 75%`.
///
/// Calling `vault.deallocate(adapter, data, balance)` returns the stranded
/// funds to vault idle and reduces `totalAllocations` proportionally.
///
/// Returns `Some(DeallocateRecord)` if funds were recovered, `None` otherwise.
pub async fn recover_adapter_balance(
    config: &ReserveConfig,
    valuer_config: Option<&ValuerConfig>,
    contracts: &evm::ContractManifest,
    tokens: &evm::TokenManifest,
    private_key: &str,
    dry_run: bool,
) -> Result<Option<DeallocateRecord>> {
    let adapter_key = match &config.adapter_address {
        Some(k) => k,
        None => return Ok(None),
    };

    let strategy_id_text = match valuer_config {
        Some(vc) => &vc.strategy_id,
        None => return Ok(None),
    };

    let rpc_url = config
        .vault_chain
        .rpc_url()
        .context("vault chain requires RPC URL")?;

    let adapter_addr =
        evm::resolve_contract(contracts, adapter_key, &config.vault_chain).with_context(|| {
            format!(
                "Adapter '{}' on {} not in contracts manifest",
                adapter_key, config.vault_chain,
            )
        })?;

    let vault_addr = evm::resolve_contract(contracts, &config.vault_address, &config.vault_chain)
        .context("vault address not in contracts manifest")?;

    let token_addr = evm::resolve_token(tokens, &config.vault_chain, &config.vault_token)
        .with_context(|| {
            format!(
                "Token '{}' on {} not in tokens manifest",
                config.vault_token, config.vault_chain,
            )
        })?;

    // Read adapter's token balance
    let rp = evm::read_provider(rpc_url)?;
    let erc20 = IErc20Read::new(token_addr, &rp);
    let adapter_balance_raw = retry_rpc("adapter.balanceOf", || async {
        erc20.balanceOf(adapter_addr).call().await
    })
    .await
    .context("failed to read adapter token balance")?;

    let decimals = retry_rpc("decimals()", || async { erc20.decimals().call().await })
        .await
        .context("decimals() failed")?;

    let adapter_balance = evm::from_token_units(adapter_balance_raw, decimals);

    // Skip dust (< $0.50)
    if adapter_balance < 0.50 {
        return Ok(None);
    }

    eprintln!(
        "[deallocate] Adapter {} has ${:.2} stranded — deallocating back to vault",
        evm::short_addr(&adapter_addr),
        adapter_balance,
    );

    if dry_run {
        eprintln!(
            "[deallocate] [DRY RUN] would deallocate ${:.2} from adapter",
            adapter_balance,
        );
        let now = chrono::Utc::now().timestamp() as u64;
        return Ok(Some(DeallocateRecord {
            timestamp: now,
            adapter_balance,
            deallocated: adapter_balance,
        }));
    }

    let strategy_id = crate::run::valuer::strategy_id_from_text(strategy_id_text);
    let allocation_data = encode_allocation_data(strategy_id);

    let provider = make_signer_provider(private_key, rpc_url)?;
    let vault = IVaultAllocate::new(vault_addr, &provider);

    let pending = vault
        .deallocate(adapter_addr, allocation_data, adapter_balance_raw)
        .gas(500_000)
        .send()
        .await
        .context("vault.deallocate() send failed")?;

    let receipt = pending
        .get_receipt()
        .await
        .context("vault.deallocate() receipt failed")?;

    if !receipt.status() {
        anyhow::bail!(
            "vault.deallocate() reverted (tx: {:?})",
            receipt.transaction_hash,
        );
    }

    eprintln!(
        "[deallocate] vault.deallocate() tx: {:?} — recovered ${:.2}",
        receipt.transaction_hash,
        adapter_balance,
    );

    let now = chrono::Utc::now().timestamp() as u64;
    Ok(Some(DeallocateRecord {
        timestamp: now,
        adapter_balance,
        deallocated: adapter_balance,
    }))
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

