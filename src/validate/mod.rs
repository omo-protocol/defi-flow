mod contracts;
mod graph;
#[cfg(feature = "full")]
pub mod onchain;
mod references;
mod reserve;
mod tokens;

use thiserror::Error;

use crate::model::Workflow;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Edge references unknown node `{node_id}`")]
    UnknownNode { node_id: String },

    #[error("Workflow contains a cycle")]
    CycleDetected,

    #[error("Self-loop on node `{node_id}`")]
    SelfLoop { node_id: String },

    #[error("Duplicate node ID `{node_id}`")]
    DuplicateNodeId { node_id: String },

    #[error("Edge {from_node}->{to_node}: {message}")]
    FlowMismatch {
        from_node: String,
        to_node: String,
        message: String,
    },

    #[error("Bridge node `{node_id}` has identical from_chain and to_chain")]
    BridgeSameChain { node_id: String },

    #[error("Optimizer `{node_id}` has no allocations (needs at least 1 venue)")]
    OptimizerNoAllocations { node_id: String },

    #[error("Optimizer `{node_id}` has kelly_fraction {value} outside valid range 0.0..=1.0")]
    OptimizerInvalidFraction { node_id: String, value: f64 },

    #[error("Optimizer `{node_id}` has max_allocation {value} outside valid range 0.0..=1.0")]
    OptimizerInvalidMaxAllocation { node_id: String, value: f64 },

    #[error("Optimizer `{node_id}` allocation target `{target_node}` has no outgoing edge from optimizer")]
    OptimizerTargetNotConnected {
        node_id: String,
        target_node: String,
    },

    #[error("Perp node `{node_id}` with action {action} requires `direction` field")]
    PerpMissingDirection { node_id: String, action: String },

    #[error("Perp node `{node_id}` with action {action} requires `leverage` field")]
    PerpMissingLeverage { node_id: String, action: String },

    #[error("Token `{token}` on chain `{chain}` has no address in the tokens manifest")]
    TokenNotInManifest { token: String, chain: String },

    #[error("Contract `{contract}` on {chain} not in contracts manifest (node `{node_id}`)")]
    ContractNotInManifest {
        contract: String,
        chain: String,
        node_id: String,
    },

    // ── Reserve config validation errors ────────────────────────────

    #[error("Reserve config: `{field}` has invalid value {value}")]
    ReserveInvalidThreshold { field: String, value: f64 },

    #[error("Reserve config: trigger_threshold ({trigger}) must be less than target_ratio ({target})")]
    ReserveTriggerAboveTarget { trigger: f64, target: f64 },

    #[error("Reserve config: vault_chain `{chain}` has no rpc_url (needed for on-chain vault reads)")]
    ReserveMissingRpc { chain: String },

    #[error("Reserve config: vault `{vault}` not found in contracts manifest for chain `{chain}`")]
    ReserveVaultNotInManifest { vault: String, chain: String },

    #[error("Reserve config: token `{token}` not found in tokens manifest for chain `{chain}`")]
    ReserveTokenNotInManifest { token: String, chain: String },

    // ── On-chain validation errors ──────────────────────────────────

    #[error("RPC unreachable for chain `{chain}` at {url}: {reason}")]
    RpcUnreachable {
        chain: String,
        url: String,
        reason: String,
    },

    #[error("Chain ID mismatch for `{chain}`: expected {expected}, RPC returned {actual}")]
    ChainIdMismatch {
        chain: String,
        expected: u64,
        actual: u64,
    },

    #[error("Contract `{contract}` on {chain} at {address} has no deployed code")]
    ContractNoCode {
        contract: String,
        chain: String,
        address: String,
    },

    #[error("Token `{token}` on {chain} at {address} has no deployed code")]
    TokenNoCode {
        token: String,
        chain: String,
        address: String,
    },

    #[error("Contract `{contract}` on {chain} at {address} does not implement expected interface ({expected})")]
    WrongInterface {
        contract: String,
        chain: String,
        address: String,
        expected: String,
    },
}

#[cfg(feature = "full")]
/// Load and fully validate a workflow from a JSON file.
pub fn load_and_validate(path: &std::path::Path) -> Result<Workflow, Vec<ValidationError>> {
    let contents = std::fs::read_to_string(path).map_err(|e| vec![ValidationError::Io(e)])?;
    let workflow: Workflow =
        serde_json::from_str(&contents).map_err(|e| vec![ValidationError::Json(e)])?;
    validate(&workflow)?;
    Ok(workflow)
}

/// Validate a workflow, collecting all errors.
pub fn validate(workflow: &Workflow) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    errors.extend(references::check_duplicate_ids(workflow));
    errors.extend(references::check_edge_references(workflow));
    errors.extend(graph::check_dag(workflow));
    errors.extend(tokens::check_token_compatibility(workflow));
    errors.extend(contracts::check_contract_manifest(workflow));
    errors.extend(reserve::check_reserve_config(workflow));

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(feature = "full")]
/// CLI entry point for the `validate` subcommand.
pub fn run(path: &std::path::Path) -> anyhow::Result<()> {
    let wf = match load_and_validate(path) {
        Ok(wf) => {
            println!(
                "Workflow '{}' is valid (offline). {} nodes, {} edges.",
                wf.name,
                wf.nodes.len(),
                wf.edges.len()
            );
            wf
        }
        Err(errors) => {
            eprintln!("Validation failed with {} error(s):", errors.len());
            for (i, e) in errors.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, e);
            }
            std::process::exit(1);
        }
    };

    // On-chain validation: check RPC connectivity, chain IDs, deployed code
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = tokio::runtime::Runtime::new()?;
    let errors = rt.block_on(onchain::validate_onchain(&wf));

    let (warnings, hard): (Vec<_>, Vec<_>) = errors
        .into_iter()
        .partition(|e| matches!(e, ValidationError::RpcUnreachable { .. }));

    for w in &warnings {
        eprintln!("  warning: {}", w);
    }

    if hard.is_empty() {
        println!("On-chain validation passed.");
        Ok(())
    } else {
        eprintln!(
            "On-chain validation failed with {} error(s):",
            hard.len()
        );
        for (i, e) in hard.iter().enumerate() {
            eprintln!("  {}. {}", i + 1, e);
        }
        std::process::exit(1);
    }
}
