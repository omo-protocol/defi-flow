use crate::model::node::{LpVenue, Node};
use crate::model::Workflow;
use crate::venues::yield_tokens::pendle::pendle_contract_key;

use super::ValidationError;

/// Check that all contract references in nodes have matching entries
/// in the workflow's `contracts` manifest.
pub fn check_contract_manifest(workflow: &Workflow) -> Vec<ValidationError> {
    // Collect all required (contract_name, chain_name, node_id) tuples
    let mut required: Vec<(String, String, String)> = Vec::new();

    for node in &workflow.nodes {
        match node {
            Node::Lending {
                id,
                chain,
                pool_address,
                rewards_controller,
                ..
            } => {
                required.push((pool_address.clone(), chain.name.clone(), id.clone()));
                if let Some(rc) = rewards_controller {
                    required.push((rc.clone(), chain.name.clone(), id.clone()));
                }
            }
            Node::Vault {
                id,
                chain,
                vault_address,
                ..
            } => {
                required.push((vault_address.clone(), chain.name.clone(), id.clone()));
            }
            Node::Pendle { id, market, .. } => {
                // Pendle nodes need market, sy, yt, and router contracts
                for suffix in &["market", "sy", "yt"] {
                    let key = pendle_contract_key(market, suffix);
                    // We don't know the chain statically — infer from manifest or use a
                    // convention. For now, check that the key exists for ANY chain.
                    required.push((key, "*".to_string(), id.clone()));
                }
                required.push(("pendle_router".to_string(), "*".to_string(), id.clone()));
            }
            Node::Lp {
                id,
                venue: LpVenue::Aerodrome,
                ..
            } => {
                required.push((
                    "aerodrome_position_manager".to_string(),
                    "base".to_string(),
                    id.clone(),
                ));
            }
            _ => {}
        }
    }

    if required.is_empty() {
        return Vec::new();
    }

    let manifest = match &workflow.contracts {
        Some(m) => m,
        None => {
            // No manifest but nodes reference contracts — warn to stderr
            // (don't fail validation so backtest-only workflows still work)
            eprintln!(
                "warning: workflow has nodes that reference contracts but no `contracts` manifest. \
                 Add a `contracts` section for live execution."
            );
            return Vec::new();
        }
    };

    let mut errors = Vec::new();

    for (contract, chain, node_id) in &required {
        let missing = match manifest.get(contract) {
            Some(chains) => {
                if chain == "*" {
                    // Just need the key to exist for any chain
                    chains.is_empty()
                } else {
                    !chains.keys().any(|c| c.eq_ignore_ascii_case(chain))
                }
            }
            None => true,
        };
        if missing {
            let chain_display = if chain == "*" {
                "any chain".to_string()
            } else {
                chain.clone()
            };
            errors.push(ValidationError::ContractNotInManifest {
                contract: contract.clone(),
                chain: chain_display,
                node_id: node_id.clone(),
            });
        }
    }

    errors
}
