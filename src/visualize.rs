use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::Workflow;
use crate::validate;

/// Render an ASCII DAG of the workflow.
pub fn run(path: &Path) -> anyhow::Result<()> {
    let workflow = validate::load_and_validate(path).map_err(|errs| {
        anyhow::anyhow!(
            "Cannot visualize invalid workflow:\n{}",
            errs.iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    render_ascii(&workflow);
    Ok(())
}

fn render_ascii(workflow: &Workflow) {
    let node_ids: Vec<&str> = workflow.nodes.iter().map(|n| n.id()).collect();
    let node_set: HashSet<&str> = node_ids.iter().copied().collect();

    let mut in_degree: HashMap<&str, usize> = node_ids.iter().map(|&id| (id, 0)).collect();
    let mut successors: HashMap<&str, Vec<&str>> =
        node_ids.iter().map(|&id| (id, vec![])).collect();
    let mut edge_labels: HashMap<(&str, &str), String> = HashMap::new();

    for edge in &workflow.edges {
        if node_set.contains(edge.from_node.as_str()) && node_set.contains(edge.to_node.as_str()) {
            *in_degree.get_mut(edge.to_node.as_str()).unwrap() += 1;
            successors
                .get_mut(edge.from_node.as_str())
                .unwrap()
                .push(&edge.to_node);
            edge_labels.insert(
                (edge.from_node.as_str(), edge.to_node.as_str()),
                edge.token.clone(),
            );
        }
    }

    // Kahn's algorithm for topological layers
    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_id, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    queue.sort();

    let mut layers: Vec<Vec<&str>> = Vec::new();

    while !queue.is_empty() {
        layers.push(queue.clone());
        let mut next_queue = Vec::new();
        for &node_id in &queue {
            for &succ in &successors[node_id] {
                let deg = in_degree.get_mut(succ).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    next_queue.push(succ);
                }
            }
        }
        next_queue.sort();
        next_queue.dedup();
        queue = next_queue;
    }

    // Build node labels
    let node_label: HashMap<&str, String> = workflow
        .nodes
        .iter()
        .map(|n| (n.id(), format!("[{}: {}]", n.id(), n.label())))
        .collect();

    // Build predecessor map for printing incoming edges
    let mut predecessors: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for edge in &workflow.edges {
        predecessors
            .entry(edge.to_node.as_str())
            .or_default()
            .push((edge.from_node.as_str(), edge.token.as_str()));
    }

    println!();
    println!("  Workflow: {}", workflow.name);
    if let Some(desc) = &workflow.description {
        println!("  {desc}");
    }
    println!("  {}", "─".repeat(60));
    println!();

    for (i, layer) in layers.iter().enumerate() {
        // Print incoming edges for this layer
        if i > 0 {
            for &node_id in layer {
                if let Some(preds) = predecessors.get(node_id) {
                    for &(from, token) in preds {
                        println!("      {from} ──({token})──▶ {node_id}");
                    }
                }
            }
            println!();
        }

        // Print nodes in this layer
        let labels: Vec<String> = layer
            .iter()
            .map(|id| node_label.get(id).cloned().unwrap_or_default())
            .collect();
        println!("  Layer {i}: {}", labels.join("  "));
        println!();
    }

    println!("  {}", "─".repeat(60));
    println!(
        "  {} nodes, {} edges",
        workflow.nodes.len(),
        workflow.edges.len()
    );
    println!();
}
