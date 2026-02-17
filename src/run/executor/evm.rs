use std::collections::HashMap;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{bail, Result};

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

// ── Chain configuration ────────────────────────────────────────────

pub fn chain_id(chain: &Chain) -> u64 {
    match chain {
        Chain::Ethereum => 1,
        Chain::Arbitrum => 42161,
        Chain::Optimism => 10,
        Chain::Base => 8453,
        Chain::Mantle => 5000,
        Chain::HyperEvm => 999,
        Chain::HyperCore => 999, // HyperCore uses same RPC as HyperEVM for EVM calls
    }
}

pub fn rpc_url(chain: &Chain) -> &'static str {
    match chain {
        Chain::Ethereum => "https://eth.llamarpc.com",
        Chain::Arbitrum => "https://arb1.arbitrum.io/rpc",
        Chain::Optimism => "https://mainnet.optimism.io",
        Chain::Base => "https://mainnet.base.org",
        Chain::Mantle => "https://rpc.mantle.xyz",
        Chain::HyperEvm | Chain::HyperCore => "https://rpc.hyperliquid.xyz/evm",
    }
}

pub fn lifi_chain_id(chain: &Chain) -> u64 {
    // LiFi uses standard chain IDs
    chain_id(chain)
}

// ── Provider factory ───────────────────────────────────────────────

/// Create an alloy HTTP provider for a given chain.
pub fn create_provider(
    chain: &Chain,
) -> Result<impl Provider + Clone> {
    let url = rpc_url(chain);
    let provider = ProviderBuilder::new().connect_http(url.parse().map_err(|e| {
        anyhow::anyhow!("Invalid RPC URL for chain {chain}: {e}")
    })?);
    Ok(provider)
}

// ── Token address registry ─────────────────────────────────────────

/// Get the token address for a given (chain, symbol) pair.
/// Returns None for unknown tokens.
pub fn token_address(chain: &Chain, symbol: &str) -> Option<Address> {
    let key = (chain_id(chain), symbol.to_uppercase());
    TOKEN_REGISTRY.get(&key).copied()
}

/// Native token placeholder address used by LiFi and others.
pub const NATIVE_TOKEN: Address = Address::ZERO;

lazy_static_token_registry! {
    // ── Ethereum ──
    (1, "USDC") => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    (1, "USDT") => "0xdAC17F958D2ee523a2206206994597C13D831ec7",
    (1, "WETH") => "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    (1, "DAI") => "0x6B175474E89094C44Da98b954EedeAC495271d0F",
    (1, "WBTC") => "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599",

    // ── Base ──
    (8453, "USDC") => "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
    (8453, "WETH") => "0x4200000000000000000000000000000000000006",
    (8453, "CBBTC") => "0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf",
    (8453, "AERO") => "0x940181a94A35A4569E4529A3CDfB74e38FD98631",
    (8453, "DAI") => "0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb",

    // ── Arbitrum ──
    (42161, "USDC") => "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
    (42161, "USDT") => "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9",
    (42161, "WETH") => "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
    (42161, "WBTC") => "0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f",
    (42161, "ARB") => "0x912CE59144191C1204E64559FE8253a0e49E6548",

    // ── Optimism ──
    (10, "USDC") => "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
    (10, "WETH") => "0x4200000000000000000000000000000000000006",
    (10, "OP") => "0x4200000000000000000000000000000000000042",

    // ── Mantle ──
    (5000, "USDC") => "0x09Bc4E0D864854c6aFB6eB9A9cdF58aC190D0dF9",
    (5000, "USDT") => "0x201EBa5CC46D216Ce6DC03F6a759e8E766e956aE",
    (5000, "WETH") => "0xdEAddEaDdeadDEadDEADDEAddEADDEAddead1111",
    (5000, "USDE") => "0x5d3a1Ff2b6BAb83b63cd9AD0787074081a52ef34",

    // ── HyperEVM ──
    (999, "USDC") => "0xEB62eee3685fC5Eb20D2bDCd08B25014B8407492",
    (999, "USDE") => "0x5d3a1Ff2b6BAb83b63cd9AD0787074081a52ef34",
    (999, "WETH") => "0x2C63007E1a4dd672E55fE2A3F39e710260981FDA",
    (999, "HYPE") => "0x2Fc2C4E7a3BD6C9EE0C7a9f2C90ac109f93D7e3D",
    (999, "WHYPE") => "0x2Fc2C4E7a3BD6C9EE0C7a9f2C90ac109f93D7e3D",
}

// ── Utility functions ──────────────────────────────────────────────

/// Convert a USD amount to token units (with decimals).
pub fn to_token_units(amount_usd: f64, price: f64, decimals: u8) -> U256 {
    let token_amount = amount_usd / price;
    let scaled = token_amount * 10f64.powi(decimals as i32);
    U256::from(scaled as u128)
}

/// Convert token units back to human-readable amount.
pub fn from_token_units(units: U256, decimals: u8) -> f64 {
    let divisor = 10f64.powi(decimals as i32);
    units.to::<u128>() as f64 / divisor
}

/// Format an address for display (shortened).
pub fn short_addr(addr: &Address) -> String {
    let s = format!("{addr}");
    if s.len() > 10 {
        format!("{}...{}", &s[..6], &s[s.len() - 4..])
    } else {
        s
    }
}

// ── Token registry implementation ──────────────────────────────────

// Build a static HashMap from (chain_id, symbol) → Address
macro_rules! lazy_static_token_registry {
    ( $( ($chain:expr, $sym:expr) => $addr:expr ),* $(,)? ) => {
        fn build_token_registry() -> HashMap<(u64, String), Address> {
            let mut m = HashMap::new();
            $(
                m.insert(($chain, $sym.to_string()), $addr.parse::<Address>().unwrap());
            )*
            m
        }

        use std::sync::LazyLock;
        static TOKEN_REGISTRY: LazyLock<HashMap<(u64, String), Address>> =
            LazyLock::new(|| build_token_registry());
    };
}
use lazy_static_token_registry;

// ── Known contract addresses ───────────────────────────────────────

/// Aave V3 / HyperLend pool addresses by (chain_id, venue_name).
pub fn lending_pool_address(chain: &Chain, venue: &str) -> Option<Address> {
    match (chain_id(chain), venue.to_lowercase().as_str()) {
        (999, "hyperlend") => Some("0xC0EE4e7e60D0A1F9a9AfaE0706D1b5C5A7f5B9b4".parse().unwrap()),
        (8453, "aave") => Some("0xA238Dd80C7A0845DA4b9e9146FF76C97a7aEcE89".parse().unwrap()),
        (42161, "aave") => Some("0x794a61358D6845594F94dc1DB02A252b5b4814aD".parse().unwrap()),
        (1, "aave") => Some("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2".parse().unwrap()),
        _ => None,
    }
}

/// Aerodrome NonfungiblePositionManager on Base.
pub fn aerodrome_position_manager() -> Address {
    "0x827922686190790b37229fd06084350E74485b72".parse().unwrap()
}

/// Aerodrome CL Gauge Factory on Base.
pub fn aerodrome_gauge_factory() -> Address {
    "0x35f35cA5B132CaDf2916BaB57639128eAC5bbcb5".parse().unwrap()
}

/// Pendle Router on various chains.
pub fn pendle_router(chain: &Chain) -> Option<Address> {
    match chain_id(chain) {
        42161 => Some("0x00000000005BBB0EF59571E58418F9a4357b68A0".parse().unwrap()),
        1 => Some("0x00000000005BBB0EF59571E58418F9a4357b68A0".parse().unwrap()),
        999 => Some("0x00000000005BBB0EF59571E58418F9a4357b68A0".parse().unwrap()), // Pendle on HyperEVM
        _ => None,
    }
}

/// Lending rewards controller addresses.
pub fn rewards_controller_address(chain: &Chain, venue: &str) -> Option<Address> {
    match (chain_id(chain), venue.to_lowercase().as_str()) {
        (999, "hyperlend") => Some("0x54586bE62E3c3580375aE3723C145253060Ca0C2".parse().unwrap()),
        _ => None,
    }
}
