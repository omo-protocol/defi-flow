use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// EVM-compatible chains supported by the workflow engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    Ethereum,
    Arbitrum,
    Optimism,
    Base,
    Mantle,
    #[serde(rename = "hyperevm")]
    HyperEvm,
    #[serde(rename = "hypercore")]
    HyperCore,
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Chain::Ethereum => write!(f, "ethereum"),
            Chain::Arbitrum => write!(f, "arbitrum"),
            Chain::Optimism => write!(f, "optimism"),
            Chain::Base => write!(f, "base"),
            Chain::Mantle => write!(f, "mantle"),
            Chain::HyperEvm => write!(f, "hyperevm"),
            Chain::HyperCore => write!(f, "hypercore"),
        }
    }
}
