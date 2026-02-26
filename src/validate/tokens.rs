use std::collections::{HashMap, HashSet};

use crate::model::chain::Chain;
use crate::model::node::{MovementProvider, MovementType, Node, PerpAction, TokenFlow};
use crate::model::Workflow;

use super::ValidationError;

/// Check token compatibility, chain flow, and node-specific constraints.
pub fn check_token_compatibility(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Wallet address validation
    errors.extend(check_wallet_nodes(workflow));

    // Movement-specific checks (bridge same-chain, etc.)
    errors.extend(check_movement_nodes(workflow));

    // Unified edge flow validation (token + chain)
    errors.extend(check_edge_flows(workflow));

    // Token manifest validation (when manifest is present)
    errors.extend(check_token_manifest(workflow));

    // Optimizer-specific constraints
    errors.extend(check_optimizer_nodes(workflow));

    // Perp-specific constraints
    errors.extend(check_perp_nodes(workflow));

    errors
}

// ── Wallet validation ───────────────────────────────────────────────

/// Validate wallet node addresses:
/// - Address must be non-empty.
/// - On EVM chains (chain_id present): must be 0x-prefixed, 42-char hex.
fn check_wallet_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if let Node::Wallet {
            id,
            chain,
            address,
            ..
        } = node
        {
            if address.is_empty() {
                errors.push(ValidationError::WalletEmptyAddress {
                    node_id: id.clone(),
                });
                continue;
            }

            // EVM chains (have chain_id) require a valid 0x-prefixed hex address
            if chain.chain_id.is_some() && !is_valid_evm_address(address) {
                errors.push(ValidationError::WalletInvalidAddress {
                    node_id: id.clone(),
                    chain: chain.name.clone(),
                    address: address.clone(),
                });
            }
        }
    }

    errors
}

/// Check if a string is a valid EVM address: 0x-prefixed, 42 chars total, hex digits.
fn is_valid_evm_address(addr: &str) -> bool {
    addr.len() == 42
        && addr.starts_with("0x")
        && addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}

// ── Edge flow validation ────────────────────────────────────────────

/// Trace back through chain-agnostic nodes to find the origin chain of tokens
/// arriving at a given node. Follows incoming edges through agnostic nodes
/// (like Optimizer) until hitting a chain-aware node.
fn trace_origin_chain<'a>(
    node_id: &str,
    node_map: &HashMap<&str, &'a Node>,
    incoming: &HashMap<&str, Vec<&str>>,
    visited: &mut HashSet<String>,
) -> Option<Chain> {
    if !visited.insert(node_id.to_string()) {
        return None; // cycle guard
    }

    // If this node has a chain, that's the origin
    if let Some(node) = node_map.get(node_id) {
        if let Some(chain) = node.chain() {
            return Some(chain);
        }
    }

    // Otherwise trace back through incoming edges
    if let Some(sources) = incoming.get(node_id) {
        for src in sources {
            if let Some(chain) = trace_origin_chain(src, node_map, incoming, visited) {
                return Some(chain);
            }
        }
    }

    None
}

/// Validate every edge for token and chain compatibility.
/// For edges FROM chain-agnostic nodes (like Optimizer), traces back to find
/// the actual origin chain so mismatches aren't hidden.
fn check_edge_flows(workflow: &Workflow) -> Vec<ValidationError> {
    let node_map: HashMap<&str, &Node> = workflow.nodes.iter().map(|n| (n.id(), n)).collect();

    // Build incoming-edge map for back-tracing
    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &workflow.edges {
        incoming
            .entry(edge.to_node.as_str())
            .or_default()
            .push(edge.from_node.as_str());
    }

    let mut errors = Vec::new();

    for edge in &workflow.edges {
        let from_node = match node_map.get(edge.from_node.as_str()) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match node_map.get(edge.to_node.as_str()) {
            Some(n) => n,
            None => continue,
        };

        // What the source node outputs (fallback: edge token on source's chain)
        let mut source = from_node.output_token().unwrap_or_else(|| TokenFlow {
            token: edge.token.clone(),
            chain: from_node.chain(),
        });

        // What the dest node expects (fallback: edge token on dest's input chain)
        let dest = to_node.expected_input_token().unwrap_or_else(|| TokenFlow {
            token: edge.token.clone(),
            chain: to_node.input_chain(),
        });

        // If the source is chain-agnostic, trace back to find actual origin chain.
        // This catches: bridge(→hyperevm) → optimizer → perp(hyperliquid)
        // The optimizer has no chain, but the tokens actually came from hyperevm.
        if source.chain.is_none() {
            let mut visited = HashSet::new();
            source.chain = trace_origin_chain(
                edge.from_node.as_str(),
                &node_map,
                &incoming,
                &mut visited,
            );
        }

        // Chain compatibility (skip if either side is truly unknown)
        let chain_ok = match (&source.chain, &dest.chain) {
            (Some(sc), Some(dc)) => sc.name.eq_ignore_ascii_case(&dc.name),
            _ => true,
        };

        let token_ok = source.token.eq_ignore_ascii_case(&dest.token);
        let edge_vs_source = source.token.eq_ignore_ascii_case(&edge.token);
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

// ── Token manifest validation ──────────────────────────────────────

/// When a `tokens` manifest is present, verify that every token used in
/// edges and wallet nodes has a contract address for the relevant chain.
fn check_token_manifest(workflow: &Workflow) -> Vec<ValidationError> {
    let manifest = match &workflow.tokens {
        Some(m) => m,
        None => return Vec::new(),
    };

    // Collect namespace-only chains (no rpc_url → no on-chain contracts).
    // Tokens on these chains are identified by name, not address.
    // e.g. Hyperliquid L1 has chain_id 1337 (for LiFi routing) but no RPC —
    // perps/spot use the Hyperliquid API, not EVM contracts.
    let namespace_chains: HashSet<String> = {
        let mut set = HashSet::new();
        for node in &workflow.nodes {
            if let Some(chain) = node.chain() {
                if chain.rpc_url.is_none() {
                    set.insert(chain.name.to_lowercase());
                }
            }
            if let Some(chain) = node.input_chain() {
                if chain.rpc_url.is_none() {
                    set.insert(chain.name.to_lowercase());
                }
            }
        }
        set
    };

    let node_map: HashMap<&str, &Node> = workflow.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut errors = Vec::new();

    let mut check = |token: &str, chain: &str| {
        // Skip namespace-only chains — tokens are names, not addresses
        if namespace_chains.contains(&chain.to_lowercase()) {
            return;
        }

        let key = (token.to_string(), chain.to_lowercase());
        if seen.contains(&key) {
            return;
        }
        seen.insert(key);

        let missing = match manifest.get(token) {
            Some(chains) => !chains.keys().any(|c| c.eq_ignore_ascii_case(chain)),
            None => true,
        };
        if missing {
            errors.push(ValidationError::TokenNotInManifest {
                token: token.to_string(),
                chain: chain.to_string(),
            });
        }
    };

    // Check all node tokens (skip chains without RPC — no on-chain contracts)
    for node in &workflow.nodes {
        match node {
            Node::Wallet { token, chain, .. } => {
                if chain.rpc_url.is_some() {
                    check(token, &chain.name);
                }
            }
            Node::Movement {
                from_token,
                to_token,
                from_chain,
                to_chain,
                ..
            } => {
                if let Some(fc) = from_chain {
                    if fc.rpc_url.is_some() {
                        check(from_token, &fc.name);
                    }
                }
                if let Some(tc) = to_chain {
                    if tc.rpc_url.is_some() {
                        check(to_token, &tc.name);
                    }
                }
            }
            Node::Lending { asset, chain, .. } => {
                if chain.rpc_url.is_some() {
                    check(asset, &chain.name);
                }
            }
            Node::Vault { asset, chain, .. } => {
                if chain.rpc_url.is_some() {
                    check(asset, &chain.name);
                }
            }
            Node::Perp { margin_token, .. } => {
                // Both Hyperliquid and Hyena margin lives on HyperCore
                // — uses HL API, not EVM contracts.
                let _ = margin_token;
            }
            Node::Lp { pool, chain, .. } => {
                // Pool is "TOKEN0/TOKEN1" — validate both tokens on the LP chain
                // Skip chains without RPC (no on-chain contracts)
                let lp_chain = chain.as_ref();
                if lp_chain.map(|c| c.rpc_url.is_some()).unwrap_or(true) {
                    let chain_name = lp_chain
                        .map(|c| c.name.as_str())
                        .unwrap_or("base");
                    for token in pool.split('/') {
                        check(token.trim(), chain_name);
                    }
                }
            }
            _ => {}
        }
    }

    // Check edge tokens against the chain of the source/dest node
    // (skip chains without RPC — no contract addresses to verify)
    for edge in &workflow.edges {
        if let Some(from_node) = node_map.get(edge.from_node.as_str()) {
            if let Some(chain) = from_node.chain() {
                if chain.rpc_url.is_some() {
                    check(&edge.token, &chain.name);
                }
            }
        }
        if let Some(to_node) = node_map.get(edge.to_node.as_str()) {
            if let Some(chain) = to_node.input_chain() {
                if chain.rpc_url.is_some() {
                    check(&edge.token, &chain.name);
                }
            }
        }
    }

    errors
}

// ── Movement checks ────────────────────────────────────────────────

fn check_movement_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if let Node::Movement {
            id,
            movement_type,
            provider,
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

            // HyperliquidNative provider constraints
            if matches!(provider, MovementProvider::HyperliquidNative) {
                // Only bridge transfers — no swaps
                if matches!(movement_type, MovementType::Swap | MovementType::SwapBridge) {
                    errors.push(ValidationError::HyperliquidNativeSwapNotSupported {
                        node_id: id.clone(),
                    });
                }

                // Must be between hyperliquid and hyperevm (either direction)
                if matches!(movement_type, MovementType::Bridge) {
                    let fc = from_chain.as_ref().map(|c| c.name.to_lowercase());
                    let tc = to_chain.as_ref().map(|c| c.name.to_lowercase());
                    let valid = matches!(
                        (fc.as_deref(), tc.as_deref()),
                        (Some("hyperliquid"), Some("hyperevm"))
                            | (Some("hyperevm"), Some("hyperliquid"))
                    );
                    if !valid {
                        errors.push(ValidationError::HyperliquidNativeWrongChains {
                            node_id: id.clone(),
                            from_chain: from_chain
                                .as_ref()
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| "?".into()),
                            to_chain: to_chain
                                .as_ref()
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| "?".into()),
                        });
                    }
                }
            }
        }
    }

    errors
}

// ── Optimizer checks (unchanged) ────────────────────────────────────

fn check_optimizer_nodes(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Build adjacency list for reachability check
    let mut adj: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &workflow.edges {
        adj.entry(edge.from_node.as_str())
            .or_default()
            .insert(edge.to_node.as_str());
    }

    /// BFS reachability from `start` to `target` through the edge graph.
    fn is_reachable(start: &str, target: &str, adj: &HashMap<&str, HashSet<&str>>) -> bool {
        let mut queue = std::collections::VecDeque::new();
        let mut visited = HashSet::new();
        queue.push_back(start);
        visited.insert(start);
        while let Some(node) = queue.pop_front() {
            if let Some(neighbors) = adj.get(node) {
                for &next in neighbors {
                    if next == target {
                        return true;
                    }
                    if visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        false
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

            for alloc in allocations {
                for target in alloc.targets() {
                    if !is_reachable(id.as_str(), target, &adj) {
                        errors.push(ValidationError::OptimizerTargetNotConnected {
                            node_id: id.clone(),
                            target_node: target.to_string(),
                        });
                    }
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
