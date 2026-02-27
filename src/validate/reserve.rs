use std::collections::HashSet;

use crate::model::Workflow;
use crate::model::node::Node;

use super::ValidationError;

/// Validate the optional reserve management configuration.
pub fn check_reserve_config(workflow: &Workflow) -> Vec<ValidationError> {
    let rc = match &workflow.reserve {
        Some(rc) => rc,
        None => return Vec::new(),
    };

    let mut errors = Vec::new();

    // Threshold ordering: 0 < trigger < target <= 1
    if rc.trigger_threshold <= 0.0 || rc.trigger_threshold >= 1.0 {
        errors.push(ValidationError::ReserveInvalidThreshold {
            field: "trigger_threshold".into(),
            value: rc.trigger_threshold,
        });
    }
    if rc.target_ratio <= 0.0 || rc.target_ratio > 1.0 {
        errors.push(ValidationError::ReserveInvalidThreshold {
            field: "target_ratio".into(),
            value: rc.target_ratio,
        });
    }
    if rc.trigger_threshold >= rc.target_ratio {
        errors.push(ValidationError::ReserveTriggerAboveTarget {
            trigger: rc.trigger_threshold,
            target: rc.target_ratio,
        });
    }

    // min_unwind must be non-negative
    if rc.min_unwind < 0.0 {
        errors.push(ValidationError::ReserveInvalidThreshold {
            field: "min_unwind".into(),
            value: rc.min_unwind,
        });
    }

    // vault_chain must have an rpc_url (needed for on-chain reads)
    if rc.vault_chain.rpc_url().is_none() {
        errors.push(ValidationError::ReserveMissingRpc {
            chain: rc.vault_chain.name.clone(),
        });
    }

    // vault_address must exist in contracts manifest for vault_chain
    if let Some(manifest) = &workflow.contracts {
        match manifest.get(&rc.vault_address) {
            Some(chains) => {
                if !chains
                    .keys()
                    .any(|c| c.eq_ignore_ascii_case(&rc.vault_chain.name))
                {
                    errors.push(ValidationError::ReserveVaultNotInManifest {
                        vault: rc.vault_address.clone(),
                        chain: rc.vault_chain.name.clone(),
                    });
                }
            }
            None => {
                errors.push(ValidationError::ReserveVaultNotInManifest {
                    vault: rc.vault_address.clone(),
                    chain: rc.vault_chain.name.clone(),
                });
            }
        }
    } else {
        // No contracts manifest at all — vault can't be resolved
        errors.push(ValidationError::ReserveVaultNotInManifest {
            vault: rc.vault_address.clone(),
            chain: rc.vault_chain.name.clone(),
        });
    }

    // vault_token must exist in tokens manifest for vault_chain
    if let Some(manifest) = &workflow.tokens {
        match manifest.get(&rc.vault_token) {
            Some(chains) => {
                if !chains
                    .keys()
                    .any(|c| c.eq_ignore_ascii_case(&rc.vault_chain.name))
                {
                    errors.push(ValidationError::ReserveTokenNotInManifest {
                        token: rc.vault_token.clone(),
                        chain: rc.vault_chain.name.clone(),
                    });
                }
            }
            None => {
                errors.push(ValidationError::ReserveTokenNotInManifest {
                    token: rc.vault_token.clone(),
                    chain: rc.vault_chain.name.clone(),
                });
            }
        }
    } else {
        errors.push(ValidationError::ReserveTokenNotInManifest {
            token: rc.vault_token.clone(),
            chain: rc.vault_chain.name.clone(),
        });
    }

    // adapter_address (if set) must exist in contracts manifest for vault_chain
    if let Some(ref adapter_key) = rc.adapter_address {
        if let Some(manifest) = &workflow.contracts {
            match manifest.get(adapter_key) {
                Some(chains) => {
                    if !chains
                        .keys()
                        .any(|c| c.eq_ignore_ascii_case(&rc.vault_chain.name))
                    {
                        errors.push(ValidationError::ReserveVaultNotInManifest {
                            vault: adapter_key.clone(),
                            chain: rc.vault_chain.name.clone(),
                        });
                    }
                }
                None => {
                    errors.push(ValidationError::ReserveVaultNotInManifest {
                        vault: adapter_key.clone(),
                        chain: rc.vault_chain.name.clone(),
                    });
                }
            }
        } else {
            errors.push(ValidationError::ReserveVaultNotInManifest {
                vault: adapter_key.clone(),
                chain: rc.vault_chain.name.clone(),
            });
        }
    }

    // Cross-chain warning: if vault is on a different chain than venues,
    // warn if no bridge/movement node exists to route capital.
    let vault_chain = rc.vault_chain.name.to_lowercase();
    let venue_chains: HashSet<String> = workflow
        .nodes
        .iter()
        .filter_map(|n| n.chain().map(|c| c.name.to_lowercase()))
        .collect();
    let has_bridge = workflow
        .nodes
        .iter()
        .any(|n| matches!(n, Node::Movement { .. }));

    for vc in &venue_chains {
        if vc != &vault_chain && !has_bridge {
            eprintln!(
                "warning: reserve vault on {} but venues on {} — no bridge/movement node found. \
                 Add a movement node or handle cross-chain transfers externally.",
                rc.vault_chain.name, vc,
            );
        }
    }

    errors
}
