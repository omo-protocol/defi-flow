use alloy::primitives::{Address, Bytes, FixedBytes, U256, keccak256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy::sol;
use alloy::sol_types::SolValue;
use anyhow::{Context, Result};

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
pub async fn maybe_push_value(
    config: &ValuerConfig,
    contracts: &evm::ContractManifest,
    private_key: &str,
    tvl: f64,
    valuer_state: &mut ValuerState,
    dry_run: bool,
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

    let strategy_id = strategy_id_from_text(&config.strategy_id);
    let value = tvl_to_uint256(tvl, config.underlying_decimals);

    if dry_run {
        eprintln!(
            "[valuer] [DRY RUN] would push value={} (TVL=${:.2}) for '{}'",
            value, tvl, config.strategy_id,
        );
        valuer_state.last_push = now;
        return Ok(true);
    }

    // Read current report for nonce + emergency check
    let rp = evm::read_provider(rpc_url)?;
    let valuer_read = IValuer::new(valuer_addr, &rp);

    let report = valuer_read
        .getReport(strategy_id)
        .call()
        .await
        .context("valuer.getReport() failed")?;

    let current_value = report.value;
    let current_nonce = report.nonce;

    // Build write provider
    let signer: PrivateKeySigner = private_key
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid key: {e}"))?;
    let wallet = alloy::network::EthereumWallet::from(signer.clone());
    let wp = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url.parse()?);
    let valuer_write = IValuer::new(valuer_addr, &wp);

    // Check if emergency mode needed
    let needs_emergency = check_needs_emergency(&valuer_read, current_value, value).await;

    if needs_emergency {
        eprintln!(
            "[valuer] Emergency push for '{}' (current={}, new={})",
            config.strategy_id, current_value, value,
        );
        push_emergency(&valuer_write, strategy_id, value).await?;
    } else {
        let nonce = current_nonce + U256::from(1);
        let expiry = U256::from(now + config.ttl);
        let chain_id = config
            .chain
            .chain_id()
            .context("valuer chain requires chain_id")?;

        let signature = sign_update(
            &signer,
            strategy_id,
            value,
            U256::from(config.confidence),
            nonce,
            expiry,
            chain_id,
            valuer_addr,
        )
        .await?;

        let pending = valuer_write
            .updateValue(
                strategy_id,
                value,
                U256::from(config.confidence),
                nonce,
                expiry,
                vec![signature],
            )
            .send()
            .await
            .context("valuer.updateValue() send failed")?;

        let receipt = pending
            .get_receipt()
            .await
            .context("valuer.updateValue() receipt failed")?;

        if !receipt.status() {
            anyhow::bail!(
                "valuer.updateValue reverted (tx: {:?})",
                receipt.transaction_hash,
            );
        }

        eprintln!(
            "[valuer] Pushed value={} for '{}' (tx: {:?})",
            value, config.strategy_id, receipt.transaction_hash,
        );
    }

    valuer_state.last_push = now;
    Ok(true)
}

// ── Internals ────────────────────────────────────────────────────────

/// Compute bytes32 strategy ID: `keccak256(abi.encodePacked(text))`.
pub fn strategy_id_from_text(text: &str) -> FixedBytes<32> {
    keccak256(text.as_bytes())
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

/// EIP-191 signing matching the keeper's contract `_verifySignatures()`:
///
/// ```text
/// messageHash = keccak256(abi.encode(
///     strategyId, value, confidence, nonce, expiry, chainId, valuerAddress
/// ))
/// signature = sign("\x19Ethereum Signed Message:\n32" + messageHash)
/// ```
async fn sign_update(
    signer: &PrivateKeySigner,
    strategy_id: FixedBytes<32>,
    value: U256,
    confidence: U256,
    nonce: U256,
    expiry: U256,
    chain_id: u64,
    valuer_addr: Address,
) -> Result<Bytes> {
    // abi.encode the 7 values
    let encoded = (
        strategy_id,
        value,
        confidence,
        nonce,
        expiry,
        U256::from(chain_id),
        valuer_addr,
    )
        .abi_encode();

    let message_hash = keccak256(&encoded);

    // alloy's sign_message handles the EIP-191 prefix internally
    let signature = signer
        .sign_message(message_hash.as_ref())
        .await
        .context("EIP-191 signing failed")?;

    Ok(Bytes::from(signature.as_bytes().to_vec()))
}

/// Check if emergency mode is needed:
/// - Current value is 0 (initial push)
/// - Price change exceeds maxPriceChangeBps
async fn check_needs_emergency<P: alloy::providers::Provider>(
    valuer: &IValuer::IValuerInstance<P>,
    current_value: U256,
    new_value: U256,
) -> bool {
    // Initial push (0 → non-zero)
    if current_value.is_zero() && !new_value.is_zero() {
        return true;
    }
    if current_value.is_zero() {
        return false;
    }

    let max_bps = valuer
        .maxPriceChangeBps()
        .call()
        .await
        .unwrap_or(U256::from(5000));

    let diff = if new_value > current_value {
        new_value - current_value
    } else {
        current_value - new_value
    };

    let change_bps = diff * U256::from(10000) / current_value;
    change_bps > max_bps
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
