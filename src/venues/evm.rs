use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use alloy::sol;

use crate::model::chain::Chain;

// ── ERC20 contract interface ───────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
        function decimals() external view returns (uint8);
        function allowance(address owner, address spender) external view returns (uint256);
    }
}

// ── Token manifest ────────────────────────────────────────────────

/// Token manifest: symbol → (chain_name → contract_address).
/// Populated from the workflow JSON `tokens` field.
pub type TokenManifest = HashMap<String, HashMap<String, String>>;

/// Resolve a token symbol to its contract address using the workflow manifest.
pub fn resolve_token(manifest: &TokenManifest, chain: &Chain, symbol: &str) -> Option<Address> {
    let chains = manifest.get(symbol)?;
    chains
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&chain.name))
        .and_then(|(_, addr)| addr.parse::<Address>().ok())
}

// ── Utility functions ──────────────────────────────────────────────

pub fn to_token_units(amount_usd: f64, price: f64, decimals: u8) -> U256 {
    let token_amount = amount_usd / price;
    let scaled = token_amount * 10f64.powi(decimals as i32);
    U256::from(scaled as u128)
}

pub fn from_token_units(value: U256, decimals: u8) -> f64 {
    let s = value.to_string();
    let raw: f64 = s.parse().unwrap_or(0.0);
    raw / 10f64.powi(decimals as i32)
}

pub fn short_addr(addr: &Address) -> String {
    let s = format!("{addr}");
    if s.len() > 10 {
        format!("{}...{}", &s[..6], &s[s.len() - 4..])
    } else {
        s
    }
}

// ── Contract manifest ─────────────────────────────────────────────

/// Contract manifest: contract_name → (chain_name → contract_address).
/// Populated from the workflow JSON `contracts` field.
pub type ContractManifest = HashMap<String, HashMap<String, String>>;

/// Resolve a protocol contract address by name and chain.
/// Looks up the workflow's `contracts` manifest — no hardcoded fallbacks.
pub fn resolve_contract(manifest: &ContractManifest, name: &str, chain: &Chain) -> Option<Address> {
    let chains = manifest.get(name)?;
    chains
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(&chain.name))
        .and_then(|(_, addr_str)| addr_str.parse::<Address>().ok())
}

/// Create a read-only provider (no wallet/signer) for on-chain queries.
pub fn read_provider(
    rpc_url: &str,
) -> anyhow::Result<impl alloy::providers::Provider + Clone> {
    Ok(alloy::providers::ProviderBuilder::new().connect_http(rpc_url.parse()?))
}
