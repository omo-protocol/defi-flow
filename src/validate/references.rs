use std::collections::HashSet;

use crate::model::Workflow;

use super::ValidationError;

/// Check that all node IDs are unique.
pub fn check_duplicate_ids(workflow: &Workflow) -> Vec<ValidationError> {
    let mut seen = HashSet::new();
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        if !seen.insert(node.id()) {
            errors.push(ValidationError::DuplicateNodeId {
                node_id: node.id().to_string(),
            });
        }
    }

    errors
}

/// Check that every edge references existing nodes.
pub fn check_edge_references(workflow: &Workflow) -> Vec<ValidationError> {
    let node_ids: HashSet<&str> = workflow.nodes.iter().map(|n| n.id()).collect();
    let mut errors = Vec::new();

    for edge in &workflow.edges {
        if !node_ids.contains(edge.from_node.as_str()) {
            errors.push(ValidationError::UnknownNode {
                node_id: edge.from_node.clone(),
            });
        }
        if !node_ids.contains(edge.to_node.as_str()) {
            errors.push(ValidationError::UnknownNode {
                node_id: edge.to_node.clone(),
            });
        }
    }

    errors
}
