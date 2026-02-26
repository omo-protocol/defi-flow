use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "full")]
use super::chain::Chain;
use super::edge::Edge;
use super::node::Node;
use super::reserve::ReserveConfig;
use super::valuer::ValuerConfig;

/// A named workflow: a directed acyclic graph of DeFi operation nodes
/// connected by token-flow edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workflow {
    /// Human-readable name for this workflow.
    pub name: String,
    /// Optional description of the strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Token manifest: maps token symbols to contract addresses per chain.
    /// Example: `{"USDC": {"hyperevm": "0x...", "base": "0x..."}}`.
    /// Used by the live executor to resolve ERC20 addresses.
    /// When present, the validator checks that all tokens used in edges/nodes
    /// have entries for the relevant chains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<HashMap<String, HashMap<String, String>>>,
    /// Protocol contract addresses per chain.
    /// Example: `{"pendle_router": {"hyperevm": "0x..."}}`.
    /// Used by live executors to resolve protocol-specific contract addresses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contracts: Option<HashMap<String, HashMap<String, String>>>,
    /// Optional vault reserve management configuration.
    /// When present, the daemon monitors the vault's reserve ratio and
    /// unwinds venue positions to replenish it when depleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserve: Option<ReserveConfig>,
    /// Optional onchain valuer configuration.
    /// When present, the daemon pushes TVL to a Morpho v2 valuer contract
    /// after each tick, subject to `push_interval` throttling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valuer: Option<ValuerConfig>,
    /// The nodes (operations) in this workflow.
    pub nodes: Vec<Node>,
    /// The edges (token flows) connecting nodes.
    pub edges: Vec<Edge>,
}

#[cfg(feature = "full")]
#[allow(dead_code)]
impl Workflow {
    /// Resolve a token symbol to its contract address for a given chain.
    /// Looks up the workflow's `tokens` manifest.
    pub fn resolve_token_address(
        &self,
        chain: &Chain,
        symbol: &str,
    ) -> Option<alloy::primitives::Address> {
        let manifest = self.tokens.as_ref()?;
        crate::venues::evm::resolve_token(manifest, chain, symbol)
    }

    /// Return the token manifest (empty if not set).
    pub fn token_manifest(&self) -> crate::venues::evm::TokenManifest {
        self.tokens.clone().unwrap_or_default()
    }

    /// Return the contracts manifest (empty if not set).
    pub fn contract_manifest(&self) -> crate::venues::evm::ContractManifest {
        self.contracts.clone().unwrap_or_default()
    }

    /// Resolve a protocol contract address by name and chain.
    /// Looks up the workflow's `contracts` manifest.
    pub fn resolve_contract(
        &self,
        name: &str,
        chain: &Chain,
    ) -> Option<alloy::primitives::Address> {
        let manifest = self.contract_manifest();
        crate::venues::evm::resolve_contract(&manifest, name, chain)
    }
}
