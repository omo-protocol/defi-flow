use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use crate::model::amount::Amount;
use crate::model::edge::Edge;
use crate::model::node::Node;
use crate::model::workflow::Workflow;
use crate::validate;

/// Entry point for the `visualize` command.
pub fn run(
    path: &Path,
    format: &str,
    scope: Option<&str>,
    output: Option<&Path>,
) -> anyhow::Result<()> {
    let workflow = validate::load_and_validate(path).map_err(|errs| {
        anyhow::anyhow!(
            "Cannot visualize invalid workflow:\n{}",
            errs.iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    // Apply scoping if requested
    let workflow = if let Some(scope_str) = scope {
        let (from, to) = parse_scope(scope_str)?;
        scope_workflow(&workflow, &from, &to)?
    } else {
        workflow
    };

    match format {
        "ascii" => {
            render_ascii(&workflow);
            Ok(())
        }
        "dot" => {
            let dot = render_dot(&workflow);
            write_output(&dot, output)
        }
        "svg" | "png" => {
            let dot = render_dot(&workflow);
            render_graphviz(&dot, format, output)
        }
        other => anyhow::bail!("Unknown format '{other}'. Use: ascii, dot, svg, png"),
    }
}

// ── Scoping ──────────────────────────────────────────────────────────

/// Parse "node_a:node_b" into (from, to).
fn parse_scope(scope: &str) -> anyhow::Result<(String, String)> {
    let parts: Vec<&str> = scope.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        anyhow::bail!("Scope must be 'from_node:to_node', got '{scope}'");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Extract the subgraph containing all nodes on any path from `from_id` to `to_id`.
fn scope_workflow(
    workflow: &Workflow,
    from_id: &str,
    to_id: &str,
) -> anyhow::Result<Workflow> {
    let node_ids: HashSet<&str> = workflow.nodes.iter().map(|n| n.id()).collect();
    if !node_ids.contains(from_id) {
        anyhow::bail!("Scope start node '{from_id}' not found in workflow");
    }
    if !node_ids.contains(to_id) {
        anyhow::bail!("Scope end node '{to_id}' not found in workflow");
    }

    // Build forward and backward adjacency lists
    let mut forward: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut backward: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &workflow.edges {
        forward
            .entry(edge.from_node.as_str())
            .or_default()
            .push(edge.to_node.as_str());
        backward
            .entry(edge.to_node.as_str())
            .or_default()
            .push(edge.from_node.as_str());
    }

    let reachable_forward = bfs_reachable(&forward, from_id);
    let reachable_backward = bfs_reachable(&backward, to_id);

    // Intersection: nodes on any path from from_id to to_id
    let on_path: HashSet<&str> = reachable_forward
        .intersection(&reachable_backward)
        .copied()
        .collect();

    if on_path.is_empty() {
        anyhow::bail!("No path found from '{from_id}' to '{to_id}'");
    }

    let nodes: Vec<Node> = workflow
        .nodes
        .iter()
        .filter(|n| on_path.contains(n.id()))
        .cloned()
        .collect();
    let edges: Vec<Edge> = workflow
        .edges
        .iter()
        .filter(|e| {
            on_path.contains(e.from_node.as_str()) && on_path.contains(e.to_node.as_str())
        })
        .cloned()
        .collect();

    Ok(Workflow {
        name: format!("{} (scope: {from_id}:{to_id})", workflow.name),
        description: workflow.description.clone(),
        nodes,
        edges,
    })
}

/// BFS from a start node, returning all reachable node IDs.
fn bfs_reachable<'a>(adj: &HashMap<&'a str, Vec<&'a str>>, start: &'a str) -> HashSet<&'a str> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);
    while let Some(node) = queue.pop_front() {
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }
    visited
}

// ── DOT Rendering ────────────────────────────────────────────────────

/// Render the workflow as a DOT language string.
fn render_dot(workflow: &Workflow) -> String {
    let mut dot = String::new();
    dot.push_str("digraph workflow {\n");
    dot.push_str("    rankdir=TB;\n");
    dot.push_str("    bgcolor=\"#1a1a2e\";\n");
    dot.push_str("    node [fontname=\"Helvetica\", fontsize=10];\n");
    dot.push_str("    edge [fontname=\"Helvetica\", fontsize=9, color=\"#888888\", fontcolor=\"#cccccc\"];\n");
    dot.push_str(&format!(
        "    labelloc=t;\n    label=\"{}\";\n    fontname=\"Helvetica\";\n    fontsize=14;\n    fontcolor=\"#ffffff\";\n\n",
        escape_dot(&workflow.name)
    ));

    // Render nodes
    for node in &workflow.nodes {
        let id = node.id();
        let label = node_dot_label(node);
        let style = node_dot_style(node);
        dot.push_str(&format!(
            "    \"{}\" [label=\"{}\"{style}];\n",
            escape_dot(id),
            label
        ));
    }

    dot.push_str("\n");

    // Render edges
    for edge in &workflow.edges {
        let label = edge_dot_label(edge);
        dot.push_str(&format!(
            "    \"{}\" -> \"{}\" [label=\"{}\"];\n",
            escape_dot(&edge.from_node),
            escape_dot(&edge.to_node),
            escape_dot(&label)
        ));
    }

    dot.push_str("}\n");
    dot
}

/// Build a multiline DOT label for a node.
fn node_dot_label(node: &Node) -> String {
    let type_name = node.type_name().to_uppercase();
    let id = node.id();
    let detail = node_detail(node);
    // Use DOT record-style newline
    format!("{type_name} | {id}\\n{detail}")
}

/// Extract a short human-readable detail string for a node.
fn node_detail(node: &Node) -> String {
    match node {
        Node::Wallet { chain, address, .. } => {
            let addr = if address.len() > 10 {
                format!("{}...{}", &address[..6], &address[address.len() - 4..])
            } else {
                address.clone()
            };
            format!("{} {}", chain, addr)
        }
        Node::Perp {
            venue,
            pair,
            action,
            leverage,
            ..
        } => {
            let lev = leverage
                .map(|l| format!(", {l:.1}x"))
                .unwrap_or_default();
            format!("{venue:?} {action:?} {pair}{lev}")
        }
        Node::Options {
            venue,
            asset,
            action,
            delta_target,
            ..
        } => {
            let delta = delta_target
                .map(|d| format!(", {:.0}d", d * 100.0))
                .unwrap_or_default();
            format!("{venue:?} {action:?} {asset:?}{delta}")
        }
        Node::Spot {
            venue, pair, side, ..
        } => format!("{venue:?} {side:?} {pair}"),
        Node::Lp {
            venue,
            pool,
            action,
            ..
        } => format!("{venue:?} {action:?} {pool}"),
        Node::Swap {
            provider,
            from_token,
            to_token,
            ..
        } => format!("{provider:?} {from_token} -> {to_token}"),
        Node::Bridge {
            provider,
            from_chain,
            to_chain,
            token,
            ..
        } => format!("{provider:?} {token} {from_chain} -> {to_chain}"),
        Node::Lending {
            archetype,
            chain,
            asset,
            action,
            ..
        } => format!("{archetype:?} {action:?} {asset} on {chain}"),
        Node::Vault {
            archetype,
            chain,
            asset,
            action,
            ..
        } => format!("{archetype:?} {action:?} {asset} on {chain}"),
        Node::Pendle {
            market, action, ..
        } => format!("{action:?} {market}"),
        Node::Optimizer {
            strategy,
            kelly_fraction,
            allocations,
            ..
        } => format!(
            "{strategy:?} {:.0}% kelly, {} venues",
            kelly_fraction * 100.0,
            allocations.len()
        ),
    }
}

/// Return DOT style attributes for a node based on its type.
fn node_dot_style(node: &Node) -> String {
    let (shape, color) = match node {
        Node::Wallet { .. } => ("house", "#2e7d32"),
        Node::Perp { .. } => ("box", "#c62828"),
        Node::Options { .. } => ("box", "#ad1457"),
        Node::Spot { .. } => ("box", "#1565c0"),
        Node::Lp { .. } => ("box", "#6a1b9a"),
        Node::Swap { .. } => ("parallelogram", "#e65100"),
        Node::Bridge { .. } => ("parallelogram", "#4e342e"),
        Node::Lending { .. } => ("box", "#00838f"),
        Node::Vault { .. } => ("box", "#00695c"),
        Node::Pendle { .. } => ("box", "#37474f"),
        Node::Optimizer { .. } => ("diamond", "#f9a825"),
    };

    let triggered_style = if node.is_triggered() {
        "dashed,filled"
    } else {
        "filled"
    };

    let font_color = match node {
        Node::Optimizer { .. } => "#000000",
        _ => "#ffffff",
    };

    format!(
        ", shape={shape}, fillcolor=\"{color}\", fontcolor=\"{font_color}\", style=\"{triggered_style}\""
    )
}

/// Build edge label showing token and amount.
fn edge_dot_label(edge: &Edge) -> String {
    let amount_str = match &edge.amount {
        Amount::Fixed { value } => value.clone(),
        Amount::Percentage { value } => format!("{value}%"),
        Amount::All => "all".to_string(),
    };
    format!("{} ({})", edge.token, amount_str)
}

/// Escape special characters for DOT format.
fn escape_dot(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ── Output ───────────────────────────────────────────────────────────

/// Write text output to file or stdout.
fn write_output(content: &str, output: Option<&Path>) -> anyhow::Result<()> {
    if let Some(path) = output {
        std::fs::write(path, content)?;
        eprintln!("Written to {}", path.display());
    } else {
        print!("{content}");
    }
    Ok(())
}

/// Invoke the system `dot` command to render DOT to SVG/PNG.
fn render_graphviz(dot: &str, format: &str, output: Option<&Path>) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let output_path = output.ok_or_else(|| {
        anyhow::anyhow!("--output is required for {format} format (binary output)")
    })?;

    let mut child = Command::new("dot")
        .args([
            &format!("-T{format}"),
            "-o",
            &output_path.display().to_string(),
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to run 'dot' command. Is Graphviz installed?\n  \
                 Install: sudo apt install graphviz  (or: brew install graphviz)\n  \
                 Error: {e}"
            )
        })?;

    child.stdin.as_mut().unwrap().write_all(dot.as_bytes())?;

    let result = child.wait_with_output()?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("dot command failed: {stderr}");
    }

    eprintln!("Rendered {format} to {}", output_path.display());
    Ok(())
}

// ── ASCII Rendering (existing) ───────────────────────────────────────

fn render_ascii(workflow: &Workflow) {
    let node_ids: Vec<&str> = workflow.nodes.iter().map(|n| n.id()).collect();
    let node_set: HashSet<&str> = node_ids.iter().copied().collect();

    let mut in_degree: HashMap<&str, usize> = node_ids.iter().map(|&id| (id, 0)).collect();
    let mut successors: HashMap<&str, Vec<&str>> =
        node_ids.iter().map(|&id| (id, vec![])).collect();

    for edge in &workflow.edges {
        if node_set.contains(edge.from_node.as_str())
            && node_set.contains(edge.to_node.as_str())
        {
            *in_degree.get_mut(edge.to_node.as_str()).unwrap() += 1;
            successors
                .get_mut(edge.from_node.as_str())
                .unwrap()
                .push(&edge.to_node);
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
    println!("  {}", "\u{2500}".repeat(60));
    println!();

    for (i, layer) in layers.iter().enumerate() {
        if i > 0 {
            for &node_id in layer {
                if let Some(preds) = predecessors.get(node_id) {
                    for &(from, token) in preds {
                        println!("      {from} \u{2500}\u{2500}({token})\u{2500}\u{2500}\u{25b6} {node_id}");
                    }
                }
            }
            println!();
        }

        let labels: Vec<String> = layer
            .iter()
            .map(|id| node_label.get(id).cloned().unwrap_or_default())
            .collect();
        println!("  Layer {i}: {}", labels.join("  "));
        println!();
    }

    println!("  {}", "\u{2500}".repeat(60));
    println!(
        "  {} nodes, {} edges",
        workflow.nodes.len(),
        workflow.edges.len()
    );
    println!();
}
