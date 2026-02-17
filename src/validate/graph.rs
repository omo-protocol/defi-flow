use std::collections::{HashMap, HashSet};

use petgraph::algo::is_cyclic_directed;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::model::Workflow;

use super::ValidationError;

/// Build a petgraph DiGraph from the workflow and check for cycles and self-loops.
///
/// Triggered (periodic) nodes are excluded from the DAG cycle check because
/// their edges represent periodic re-entry flows (e.g. claim_rewards -> swap -> optimizer)
/// which naturally form cycles but are not executed as a single pass.
pub fn check_dag(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Collect IDs of triggered nodes — their edges are allowed to form cycles
    let triggered_ids: HashSet<&str> = workflow
        .nodes
        .iter()
        .filter(|n| n.is_triggered())
        .map(|n| n.id())
        .collect();

    let mut graph = DiGraph::<&str, ()>::new();
    let mut index_map: HashMap<&str, NodeIndex> = HashMap::new();

    // Only add non-triggered nodes to the DAG
    for node in &workflow.nodes {
        if !node.is_triggered() {
            let id = node.id();
            let idx = graph.add_node(id);
            index_map.insert(id, idx);
        }
    }

    for edge in &workflow.edges {
        if edge.from_node == edge.to_node {
            errors.push(ValidationError::SelfLoop {
                node_id: edge.from_node.clone(),
            });
            continue;
        }

        // Skip edges involving triggered nodes for DAG validation
        if triggered_ids.contains(edge.from_node.as_str())
            || triggered_ids.contains(edge.to_node.as_str())
        {
            continue;
        }

        if let (Some(&from_idx), Some(&to_idx)) = (
            index_map.get(edge.from_node.as_str()),
            index_map.get(edge.to_node.as_str()),
        ) {
            graph.add_edge(from_idx, to_idx, ());
        }
        // Missing node references are caught by references.rs
    }

    if is_cyclic_directed(&graph) {
        errors.push(ValidationError::CycleDetected);
    }

    errors
}
