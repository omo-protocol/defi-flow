use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::chain::Chain;

/// Configuration for pushing strategy NAV to an onchain Morpho v2 valuer
/// contract.
///
/// When present in a workflow, the daemon signs and pushes the current TVL
/// after each tick, subject to `push_interval` throttling.
/// The strategy wallet (from `DEFI_FLOW_PRIVATE_KEY`) is the signer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ValuerConfig {
    /// Contracts manifest key for the valuer contract address.
    /// Must have a matching entry in the workflow `contracts` manifest.
    pub contract: String,

    /// Text strategy identifier. Hashed with `keccak256(abi.encodePacked(...))`
    /// to produce the `bytes32` strategy ID used on-chain.
    pub strategy_id: String,

    /// Chain where the valuer contract is deployed.
    pub chain: Chain,

    /// Confidence level (0–100) included in the signed message.
    #[serde(default = "default_confidence")]
    pub confidence: u64,

    /// Decimals of the underlying token (e.g. 6 for USDC, 18 for ETH).
    /// Used to scale the f64 TVL to a uint256 on-chain value.
    #[serde(default = "default_underlying_decimals")]
    pub underlying_decimals: u8,

    /// Minimum seconds between value pushes. Default: 3600 (1 hour).
    #[serde(default = "default_push_interval")]
    pub push_interval: u64,

    /// TTL in seconds for the signature expiry. Default: 7200 (2 hours).
    #[serde(default = "default_ttl")]
    pub ttl: u64,
}

fn default_confidence() -> u64 {
    90
}
fn default_underlying_decimals() -> u8 {
    6
}
fn default_push_interval() -> u64 {
    3600
}
fn default_ttl() -> u64 {
    7200
}
