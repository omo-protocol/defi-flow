use alloy::primitives::{Address, FixedBytes, U256, keccak256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use anyhow::{Context, Result};

use crate::engine::reserve::IAdapter;
use crate::model::reserve::ReserveConfig;
use crate::model::valuer::ValuerConfig;
use crate::venues::evm;

// ── Valuer contract interface ────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IValuer {
        function updateValue(
            bytes32 strategyId,
            uint256 value,
            uint256 confidence,
            uint256 nonce,
            uint256 expiry,
            bytes[] calldata signatures
        ) external;

        function getReport(bytes32 strategyId)
            external view returns (
                uint256 value,
                uint256 timestamp,
                uint256 confidence,
                uint256 nonce,
                bool isPush,
                address lastUpdater
            );

        function emergencyUpdate(bytes32 strategyId, uint256 value) external;
        function setEmergencyMode(bool enabled) external;
        function maxPriceChangeBps() external view returns (uint256);
    }
}

// ── Throttle state (in-memory, resets on restart) ────────────────────

/// Tracks last push time to throttle update frequency.
pub struct ValuerState {
    pub last_push: u64,
}

impl Default for ValuerState {
    fn default() -> Self {
        Self { last_push: 0 }
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Push TVL to onchain valuer if enough time has elapsed.
///
/// Returns `Ok(true)` if a push was performed, `Ok(false)` if throttled.
/// If `reserve_config` has an adapter, calls `refreshCachedValuation()` after push.
pub async fn maybe_push_value(
    config: &ValuerConfig,
    contracts: &evm::ContractManifest,
    private_key: &str,
    tvl: f64,
    valuer_state: &mut ValuerState,
    dry_run: bool,
    reserve_config: Option<&ReserveConfig>,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp() as u64;

    // Throttle: skip if pushed recently
    if valuer_state.last_push > 0
        && now.saturating_sub(valuer_state.last_push) < config.push_interval
    {
        return Ok(false);
    }

    let valuer_addr = resolve_valuer(config, contracts)?;
    let rpc_url = config
        .chain
        .rpc_url()
        .context("valuer chain requires rpc_url")?;

    // Use escrow totalId when adapter is configured (adapter reads from this key).
    // Falls back to keccak256(strategy_name) for non-vault strategies.
    let strategy_id = match reserve_config.and_then(|rc| rc.adapter_address.as_ref()) {
        Some(adapter_key) => {
            let adapter_addr =
                evm::resolve_contract(contracts, adapter_key, &config.chain).with_context(|| {
                    format!("Adapter '{}' on {} not in contracts manifest", adapter_key, config.chain)
                })?;
            let id = escrow_total_id(adapter_addr);
            eprintln!("[valuer] Using escrow totalId={:?} (adapter={})", id, adapter_addr);
            id
        }
        None => strategy_id_from_text(&config.strategy_id),
    };
    let value = tvl_to_uint256(tvl, config.underlying_decimals);

    if dry_run {
        eprintln!(
            "[valuer] [DRY RUN] would push value={} (TVL=${:.2}) for '{}'",
            value, tvl, config.strategy_id,
        );
        valuer_state.last_push = now;
        return Ok(true);
    }

    // Build write provider
    let signer: PrivateKeySigner = private_key
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
    let wallet = alloy::network::EthereumWallet::from(signer);
    let wp = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse()?);
    let valuer_write = IValuer::new(valuer_addr, &wp);

    // Always use emergency push — the strategy wallet is a keeper and can call
    // setEmergencyMode/emergencyUpdate, but updateValue() requires onlyOwner
    // and also hits SignatureExpiryTooFar with the current TTL config.
    eprintln!(
        "[valuer] Pushing value={} (TVL=${:.2}) for '{}'",
        value, tvl, config.strategy_id,
    );
    push_emergency(&valuer_write, strategy_id, value).await?;

    // Refresh adapter's cached valuation after successful push
    if !dry_run {
        if let Some(rc) = reserve_config {
            if let Some(ref adapter_key) = rc.adapter_address {
                if let Some(adapter_addr) =
                    evm::resolve_contract(contracts, adapter_key, &config.chain)
                {
                    let adapter = IAdapter::new(adapter_addr, &wp);
                    match adapter.refreshCachedValuation().send().await {
                        Ok(pending) => match pending.get_receipt().await {
                            Ok(receipt) if receipt.status() => {
                                eprintln!(
                                    "[valuer] adapter.refreshCachedValuation() tx: {:?}",
                                    receipt.transaction_hash,
                                );
                            }
                            Ok(receipt) => {
                                eprintln!(
                                    "[valuer] WARNING: refreshCachedValuation() reverted (tx: {:?})",
                                    receipt.transaction_hash,
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "[valuer] WARNING: refreshCachedValuation() receipt failed: {:#}",
                                    e
                                );
                            }
                        },
                        Err(e) => {
                            eprintln!(
                                "[valuer] WARNING: refreshCachedValuation() send failed: {:#}",
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    valuer_state.last_push = now;
    Ok(true)
}

// ── Internals ────────────────────────────────────────────────────────

/// Compute bytes32 strategy ID: `keccak256(abi.encodePacked(text))`.
pub fn strategy_id_from_text(text: &str) -> FixedBytes<32> {
    keccak256(text.as_bytes())
}

/// Compute the adapter's totalId for the valuer:
/// `keccak256(abi.encodePacked("ESCROW_TOTAL", adapter_address))`.
///
/// This is the key the adapter uses to look up its total value via
/// `valuer.getValue(totalId)`. The valuer must have a report stored
/// under this key for `realAssets()` and `refreshCachedValuation()`.
pub fn escrow_total_id(adapter_address: Address) -> FixedBytes<32> {
    let mut packed = Vec::with_capacity(32);
    packed.extend_from_slice(b"ESCROW_TOTAL");
    packed.extend_from_slice(adapter_address.as_slice());
    keccak256(&packed)
}

/// Convert f64 TVL (USD) to uint256 scaled by `decimals`.
pub fn tvl_to_uint256(tvl: f64, decimals: u8) -> U256 {
    let scaled = tvl * 10f64.powi(decimals as i32);
    U256::from(scaled.max(0.0) as u128)
}

/// Resolve valuer contract address from the contracts manifest.
fn resolve_valuer(config: &ValuerConfig, contracts: &evm::ContractManifest) -> Result<Address> {
    evm::resolve_contract(contracts, &config.contract, &config.chain).with_context(|| {
        format!(
            "Valuer contract '{}' on {} not in contracts manifest",
            config.contract, config.chain,
        )
    })
}

/// Push value using emergency mode (3-step: enable → update → disable).
async fn push_emergency<P: alloy::providers::Provider>(
    valuer: &IValuer::IValuerInstance<P>,
    strategy_id: FixedBytes<32>,
    value: U256,
) -> Result<()> {
    // Step 1: Enable emergency mode
    let pending = valuer
        .setEmergencyMode(true)
        .send()
        .await
        .context("setEmergencyMode(true) failed")?;
    let receipt = pending.get_receipt().await?;
    if !receipt.status() {
        anyhow::bail!("setEmergencyMode(true) reverted");
    }

    // Step 2: Emergency update
    let pending = valuer
        .emergencyUpdate(strategy_id, value)
        .send()
        .await
        .context("emergencyUpdate failed")?;
    let receipt = pending.get_receipt().await?;
    let update_ok = receipt.status();

    // Step 3: Disable emergency mode (ALWAYS, even if update failed)
    let pending = valuer
        .setEmergencyMode(false)
        .send()
        .await
        .context("setEmergencyMode(false) failed")?;
    let receipt = pending.get_receipt().await?;
    if !receipt.status() {
        eprintln!("[valuer] WARNING: setEmergencyMode(false) reverted — valuer may still be in emergency mode");
    }

    if !update_ok {
        anyhow::bail!("emergencyUpdate reverted");
    }

    Ok(())
}
