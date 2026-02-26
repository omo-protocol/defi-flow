use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A blockchain specification.
///
/// In JSON, chains are always objects:
/// - EVM chain: `{"name": "ethereum", "chain_id": 1, "rpc_url": "https://eth.llamarpc.com"}`
/// - Named chain: `{"name": "hyperliquid"}` (chain_id/rpc_url filled from registry)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Chain {
    /// Human-readable chain name (e.g. "ethereum", "base", "hyperliquid").
    pub name: String,
    /// EVM chain ID. Required for EVM chains that need on-chain interaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    /// JSON-RPC endpoint URL. Required for on-chain interactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
}

// ── Methods ──────────────────────────────────────────────────────────

impl Chain {
    /// EVM chain ID, if this chain has one.
    pub fn chain_id(&self) -> Option<u64> {
        self.chain_id
    }

    /// JSON-RPC URL, if this chain has one.
    pub fn rpc_url(&self) -> Option<&str> {
        self.rpc_url.as_deref()
    }

    /// Human-readable name.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ── Convenience constructors ─────────────────────────────────────────

#[allow(dead_code)]
impl Chain {
    pub fn ethereum() -> Self {
        Chain {
            name: "ethereum".into(),
            chain_id: Some(1),
            rpc_url: Some("https://eth.llamarpc.com".into()),
        }
    }
    pub fn arbitrum() -> Self {
        Chain {
            name: "arbitrum".into(),
            chain_id: Some(42161),
            rpc_url: Some("https://arb1.arbitrum.io/rpc".into()),
        }
    }
    pub fn optimism() -> Self {
        Chain {
            name: "optimism".into(),
            chain_id: Some(10),
            rpc_url: Some("https://mainnet.optimism.io".into()),
        }
    }
    pub fn base() -> Self {
        Chain {
            name: "base".into(),
            chain_id: Some(8453),
            rpc_url: Some("https://mainnet.base.org".into()),
        }
    }
    pub fn mantle() -> Self {
        Chain {
            name: "mantle".into(),
            chain_id: Some(5000),
            rpc_url: Some("https://rpc.mantle.xyz".into()),
        }
    }
    pub fn hyperevm() -> Self {
        Chain {
            name: "hyperevm".into(),
            chain_id: Some(999),
            rpc_url: Some("https://rpc.hyperliquid.xyz/evm".into()),
        }
    }
    /// HyperCore (Hyperliquid L1) — chain 1337 on LiFi.
    /// Perps/spot use the Hyperliquid API (not EVM RPC), but LiFi
    /// exposes it as chain 1337 for bridging/routing.
    pub fn hyperliquid() -> Self {
        Chain {
            name: "hyperliquid".into(),
            chain_id: Some(1337),
            rpc_url: None,
        }
    }

    /// Construct a chain from its name, matching known chains.
    /// Falls back to a name-only chain (no chain_id/rpc_url) for unknown names.
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "ethereum" => Self::ethereum(),
            "arbitrum" => Self::arbitrum(),
            "optimism" => Self::optimism(),
            "base" => Self::base(),
            "mantle" => Self::mantle(),
            "hyperevm" => Self::hyperevm(),
            "hyperliquid" => Self::hyperliquid(),
            _ => Self::named(name),
        }
    }

    /// Custom EVM chain with chain_id + rpc_url.
    pub fn custom(name: impl Into<String>, chain_id: u64, rpc_url: impl Into<String>) -> Self {
        Chain {
            name: name.into(),
            chain_id: Some(chain_id),
            rpc_url: Some(rpc_url.into()),
        }
    }

    /// Named non-EVM chain (no chain_id, no rpc_url).
    pub fn named(name: impl Into<String>) -> Self {
        Chain {
            name: name.into(),
            chain_id: None,
            rpc_url: None,
        }
    }
}

// ── Display ──────────────────────────────────────────────────────────

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}
