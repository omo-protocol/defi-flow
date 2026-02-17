use std::collections::{HashMap, HashSet};

use crate::model::node::PerpAction;
use crate::model::{Node, Workflow};

use super::ValidationError;

/// Check token compatibility and node-specific constraints.
pub fn check_token_compatibility(workflow: &Workflow) -> Vec<ValidationError> {
    let node_map: HashMap<&str, &Node> = workflow.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut errors = Vec::new();

    // Check bridge nodes don't bridge to same chain
    for node in &workflow.nodes {
        if let Node::Bridge {
            id,
            from_chain,
            to_chain,
            ..
        } = node
        {
            if from_chain == to_chain {
                errors.push(ValidationError::BridgeSameChain {
                    node_id: id.clone(),
                });
            }
        }
    }

    // Check edge token compatibility with destination node's expected input
    for edge in &workflow.edges {
        if let Some(node) = node_map.get(edge.to_node.as_str()) {
            if let Some(expected) = expected_input_token(node) {
                if !tokens_compatible(&edge.token, expected) {
                    errors.push(ValidationError::TokenMismatch {
                        edge_token: edge.token.clone(),
                        node_id: edge.to_node.clone(),
                        node_token: expected.to_string(),
                    });
                }
            }
        }
    }

    // Check cross-chain edges (must go through a bridge)
    errors.extend(check_cross_chain_edges(workflow, &node_map));

    // Check optimizer-specific constraints
    errors.extend(check_optimizer_nodes(workflow));

    // Check perp-specific constraints
    errors.extend(check_perp_nodes(workflow));

    errors
}

/// Check that edges don't cross chains without a bridge node.
///
/// Each node has an output chain (`node.chain()`) and an input chain (`node.input_chain()`).
/// For bridges, input = from_chain and output = to_chain; for all other nodes they're the same.
/// If the source node's output chain differs from the destination node's input chain,
/// that edge crosses chains without a bridge and is invalid.
///
/// Nodes returning `None` for their chain (e.g. Optimizer, Swap without an explicit chain)
/// are chain-agnostic and skip this check.
fn check_cross_chain_edges(
    workflow: &Workflow,
    node_map: &HashMap<&str, &Node>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for edge in &workflow.edges {
        let from_node = match node_map.get(edge.from_node.as_str()) {
            Some(n) => n,
            None => continue, // unknown-node error caught elsewhere
        };
        let to_node = match node_map.get(edge.to_node.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let from_chain = from_node.chain(); // output chain of source
        let to_chain = to_node.input_chain(); // input chain of destination

        if let (Some(fc), Some(tc)) = (&from_chain, &to_chain) {
            // Compare by name only — chain_id/rpc_url may differ between convenience
            // constructors and user-supplied values for the same logical chain.
            if fc.name != tc.name {
                errors.push(ValidationError::CrossChainEdge {
                    from_node: edge.from_node.clone(),
                    to_node: edge.to_node.clone(),
                    from_chain: fc.name.clone(),
                    to_chain: tc.name.clone(),
                });
            }
        }
    }

    errors
}

/// Validate optimizer nodes: fractions, allocations, and connectivity.
fn check_optimizer_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Build set of outgoing edges per node
    let mut outgoing_targets: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &workflow.edges {
        outgoing_targets
            .entry(edge.from_node.as_str())
            .or_default()
            .insert(edge.to_node.as_str());
    }

    for node in &workflow.nodes {
        if let Node::Optimizer {
            id,
            kelly_fraction,
            max_allocation,
            allocations,
            ..
        } = node
        {
            // Must have at least 1 allocation
            if allocations.is_empty() {
                errors.push(ValidationError::OptimizerNoAllocations {
                    node_id: id.clone(),
                });
            }

            // kelly_fraction must be in 0.0..=1.0
            if !(*kelly_fraction >= 0.0 && *kelly_fraction <= 1.0) {
                errors.push(ValidationError::OptimizerInvalidFraction {
                    node_id: id.clone(),
                    value: *kelly_fraction,
                });
            }

            // max_allocation (if present) must be in 0.0..=1.0
            if let Some(max_alloc) = max_allocation {
                if !(*max_alloc >= 0.0 && *max_alloc <= 1.0) {
                    errors.push(ValidationError::OptimizerInvalidMaxAllocation {
                        node_id: id.clone(),
                        value: *max_alloc,
                    });
                }
            }

            // Each allocation target must have an outgoing edge from this optimizer
            let targets = outgoing_targets.get(id.as_str());
            for alloc in allocations {
                let connected = targets
                    .map(|t| t.contains(alloc.target_node.as_str()))
                    .unwrap_or(false);
                if !connected {
                    errors.push(ValidationError::OptimizerTargetNotConnected {
                        node_id: id.clone(),
                        target_node: alloc.target_node.clone(),
                    });
                }
            }
        }
    }

    errors
}

/// Validate perp nodes: open/adjust require direction + leverage.
fn check_perp_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if let Node::Perp {
            id,
            action,
            direction,
            leverage,
            ..
        } = node
        {
            if matches!(action, PerpAction::Open | PerpAction::Adjust) {
                if direction.is_none() {
                    errors.push(ValidationError::PerpMissingDirection {
                        node_id: id.clone(),
                        action: format!("{action:?}"),
                    });
                }
                if leverage.is_none() {
                    errors.push(ValidationError::PerpMissingLeverage {
                        node_id: id.clone(),
                        action: format!("{action:?}"),
                    });
                }
            }
        }
    }

    errors
}

/// Determine the expected input token for a node, if it has a specific one.
fn expected_input_token(node: &Node) -> Option<&str> {
    match node {
        Node::Swap { from_token, .. } => Some(from_token),
        Node::Bridge { token, .. } => Some(token),
        Node::Perp { .. } => node.margin_token(),
        // Wallet, Spot, Lp, Optimizer accept tokens contextually
        _ => None,
    }
}

/// Case-insensitive token comparison.
fn tokens_compatible(edge_token: &str, node_token: &str) -> bool {
    edge_token.eq_ignore_ascii_case(node_token)
}
