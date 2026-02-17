mod graph;
mod references;
mod tokens;

use std::path::Path;

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

    #[error("Token mismatch: edge carries `{edge_token}` but node `{node_id}` expects `{node_token}`")]
    TokenMismatch {
        edge_token: String,
        node_id: String,
        node_token: String,
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
}

/// Load and fully validate a workflow from a JSON file.
pub fn load_and_validate(path: &Path) -> Result<Workflow, Vec<ValidationError>> {
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

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// CLI entry point for the `validate` subcommand.
pub fn run(path: &Path) -> anyhow::Result<()> {
    match load_and_validate(path) {
        Ok(wf) => {
            println!(
                "Workflow '{}' is valid. {} nodes, {} edges.",
                wf.name,
                wf.nodes.len(),
                wf.edges.len()
            );
            Ok(())
        }
        Err(errors) => {
            eprintln!("Validation failed with {} error(s):", errors.len());
            for (i, e) in errors.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, e);
            }
            std::process::exit(1);
        }
    }
}
