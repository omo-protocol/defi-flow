use std::collections::{HashMap, HashSet};

use crate::model::node::{MovementType, Node, PerpAction, TokenFlow};
use crate::model::Workflow;

use super::ValidationError;

/// Check token compatibility, chain flow, and node-specific constraints.
pub fn check_token_compatibility(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Movement-specific checks (bridge same-chain, etc.)
    errors.extend(check_movement_nodes(workflow));

    // Unified edge flow validation (token + chain)
    errors.extend(check_edge_flows(workflow));

    // Optimizer-specific constraints
    errors.extend(check_optimizer_nodes(workflow));

    // Perp-specific constraints
    errors.extend(check_perp_nodes(workflow));

    errors
}

// ── Edge flow validation ────────────────────────────────────────────

/// Validate every edge for token and chain compatibility.
/// Produces actionable error messages suggesting intermediate nodes to insert.
fn check_edge_flows(workflow: &Workflow) -> Vec<ValidationError> {
    let node_map: HashMap<&str, &Node> = workflow.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut errors = Vec::new();

    for edge in &workflow.edges {
        let from_node = match node_map.get(edge.from_node.as_str()) {
            Some(n) => n,
            None => continue, // caught by reference checks
        };
        let to_node = match node_map.get(edge.to_node.as_str()) {
            Some(n) => n,
            None => continue,
        };

        // What the source node outputs (fallback: edge token on source's chain)
        let source = from_node.output_token().unwrap_or_else(|| TokenFlow {
            token: edge.token.clone(),
            chain: from_node.chain(),
        });

        // What the dest node expects (fallback: edge token on dest's input chain)
        let dest = to_node.expected_input_token().unwrap_or_else(|| TokenFlow {
            token: edge.token.clone(),
            chain: to_node.input_chain(),
        });

        // Chain compatibility (skip if either is chain-agnostic)
        let chain_ok = match (&source.chain, &dest.chain) {
            (Some(sc), Some(dc)) => sc.name.eq_ignore_ascii_case(&dc.name),
            _ => true,
        };

        // Token compatibility (source output vs dest expectation)
        let token_ok = source.token.eq_ignore_ascii_case(&dest.token);

        // Edge token vs source output
        let edge_vs_source = source.token.eq_ignore_ascii_case(&edge.token);

        // Edge token vs dest expectation
        let edge_vs_dest = dest.token.eq_ignore_ascii_case(&edge.token);

        if chain_ok && token_ok && edge_vs_source && edge_vs_dest {
            continue;
        }

        let message = build_flow_suggestion(from_node, to_node, &edge.token, &source, &dest, chain_ok, token_ok, edge_vs_source);

        errors.push(ValidationError::FlowMismatch {
            from_node: edge.from_node.clone(),
            to_node: edge.to_node.clone(),
            message,
        });
    }

    errors
}

/// Build an actionable error message suggesting what nodes to insert.
fn build_flow_suggestion(
    from_node: &Node,
    to_node: &Node,
    edge_token: &str,
    source: &TokenFlow,
    dest: &TokenFlow,
    chain_ok: bool,
    token_ok: bool,
    edge_vs_source: bool,
) -> String {
    let from_id = from_node.id();
    let to_id = to_node.id();
    let sc = source
        .chain
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("?");
    let dc = dest
        .chain
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("?");

    // Special case: edge token doesn't match source output (but dest may be fine)
    if !edge_vs_source && chain_ok {
        return format!(
            "edge declares token {} but '{}' outputs {} on {}. \
             Insert a Movement(swap, from_token: {}, to_token: {}) between them",
            edge_token, from_id, source.token, sc, source.token, edge_token,
        );
    }

    match (chain_ok, token_ok) {
        (false, true) => {
            // Chain mismatch only
            format!(
                "chain mismatch: '{}' outputs {} on {}, but '{}' expects it on {}. \
                 Insert a Movement(bridge, from_chain: {}, to_chain: {}, token: {})",
                from_id, source.token, sc, to_id, dc, sc, dc, source.token,
            )
        }
        (true, false) => {
            // Token mismatch only (same chain)
            let chain_name = if sc != "?" { sc } else { dc };
            format!(
                "token mismatch: '{}' outputs {} but '{}' expects {} (both on {}). \
                 Insert a Movement(swap, from_token: {}, to_token: {})",
                from_id, source.token, to_id, dest.token, chain_name,
                source.token, dest.token,
            )
        }
        (false, false) => {
            // Both chain AND token mismatch
            let bridge_tok = "USDC";

            // If only token differs, can use a single swap_bridge Movement
            if source.token.eq_ignore_ascii_case(bridge_tok) || dest.token.eq_ignore_ascii_case(bridge_tok) {
                // One side is already USDC — suggest swap_bridge or bridge+swap
                let mut steps = Vec::new();

                if !source.token.eq_ignore_ascii_case(bridge_tok) {
                    steps.push(format!(
                        "Movement(swap, from_token: {}, to_token: {})",
                        source.token, bridge_tok,
                    ));
                }

                steps.push(format!(
                    "Movement(bridge, from_chain: {}, to_chain: {}, token: {})",
                    sc, dc, bridge_tok,
                ));

                if !dest.token.eq_ignore_ascii_case(bridge_tok) {
                    steps.push(format!(
                        "Movement(swap, from_token: {}, to_token: {})",
                        bridge_tok, dest.token,
                    ));
                }

                let numbered: Vec<String> = steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("({}) {s}", i + 1))
                    .collect();

                format!(
                    "chain+token mismatch: '{}' outputs {} on {}, but '{}' expects {} on {}. \
                     Insert: {}",
                    from_id, source.token, sc, to_id, dest.token, dc,
                    numbered.join(", then "),
                )
            } else {
                // Both tokens differ from USDC — suggest swap_bridge (atomic)
                format!(
                    "chain+token mismatch: '{}' outputs {} on {}, but '{}' expects {} on {}. \
                     Insert a Movement(swap_bridge, from_token: {}, to_token: {}, from_chain: {}, to_chain: {})",
                    from_id, source.token, sc, to_id, dest.token, dc,
                    source.token, dest.token, sc, dc,
                )
            }
        }
        (true, true) => unreachable!(),
    }
}

// ── Movement checks ────────────────────────────────────────────────

fn check_movement_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if let Node::Movement {
            id,
            movement_type,
            from_chain,
            to_chain,
            ..
        } = node
        {
            match movement_type {
                MovementType::Bridge | MovementType::SwapBridge => {
                    // Bridge / swap_bridge require both chains and they must differ
                    match (from_chain, to_chain) {
                        (Some(fc), Some(tc)) if fc.name.eq_ignore_ascii_case(&tc.name) => {
                            errors.push(ValidationError::BridgeSameChain {
                                node_id: id.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                MovementType::Swap => {}
            }
        }
    }

    errors
}

// ── Optimizer checks (unchanged) ────────────────────────────────────

fn check_optimizer_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

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
            if allocations.is_empty() {
                errors.push(ValidationError::OptimizerNoAllocations {
                    node_id: id.clone(),
                });
            }

            if !(*kelly_fraction >= 0.0 && *kelly_fraction <= 1.0) {
                errors.push(ValidationError::OptimizerInvalidFraction {
                    node_id: id.clone(),
                    value: *kelly_fraction,
                });
            }

            if let Some(max_alloc) = max_allocation {
                if !(*max_alloc >= 0.0 && *max_alloc <= 1.0) {
                    errors.push(ValidationError::OptimizerInvalidMaxAllocation {
                        node_id: id.clone(),
                        value: *max_alloc,
                    });
                }
            }

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

// ── Perp checks (unchanged) ────────────────────────────────────────

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
