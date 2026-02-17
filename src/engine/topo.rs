use std::collections::{HashMap, HashSet};

use petgraph::graph::DiGraph;
use petgraph::visit::Topo;

use crate::model::node::NodeId;
use crate::model::workflow::Workflow;

/// Compute the topological execution order for deploy-phase (non-triggered) nodes.
/// Triggered nodes and their edges are excluded — they run on the periodic schedule.
pub fn deploy_order(workflow: &Workflow) -> Vec<NodeId> {
    let triggered_ids: HashSet<&str> = workflow
        .nodes
        .iter()
        .filter(|n| n.is_triggered())
        .map(|n| n.id())
        .collect();

    // Build a petgraph with only non-triggered nodes/edges
    let mut graph = DiGraph::<&str, ()>::new();
    let mut node_indices: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();

    for node in &workflow.nodes {
        if !triggered_ids.contains(node.id()) {
            let idx = graph.add_node(node.id());
            node_indices.insert(node.id(), idx);
        }
    }

    for edge in &workflow.edges {
        let from_triggered = triggered_ids.contains(edge.from_node.as_str());
        let to_triggered = triggered_ids.contains(edge.to_node.as_str());
        if !from_triggered && !to_triggered {
            if let (Some(&from_idx), Some(&to_idx)) = (
                node_indices.get(edge.from_node.as_str()),
                node_indices.get(edge.to_node.as_str()),
            ) {
                graph.add_edge(from_idx, to_idx, ());
            }
        }
    }

    let mut topo = Topo::new(&graph);
    let mut order = Vec::new();
    while let Some(idx) = topo.next(&graph) {
        order.push(graph[idx].to_string());
    }
    order
}
