import type { CanvasNode, CanvasEdge } from "../types/canvas";
import type { DefiFlowWorkflow, DefiNode, DefiEdge } from "../types/defi-flow";
import { getNodeLabel } from "../types/defi-flow";

type Manifest = Record<string, Record<string, string>>;

/**
 * Convert canvas state → defi-flow JSON for export.
 *
 * `existingTokens` / `existingContracts` are the manifests from the last
 * import or manual edit — we preserve them and merge in any new entries
 * discovered from the canvas nodes.
 */
export function convertCanvasToDefiFlow(
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  name: string,
  description?: string,
  existingTokens?: Manifest,
  existingContracts?: Manifest,
): DefiFlowWorkflow {
  const defiNodes: DefiNode[] = nodes.map((n) => n.data.defiNode);
  const defiEdges: DefiEdge[] = edges.map((e) => ({
    from_node: e.source,
    to_node: e.target,
    token: e.data?.token ?? "USDC",
    amount: e.data?.amount ?? { type: "all" as const },
  }));

  // Start from existing manifests (preserves user-entered addresses)
  const tokens: Manifest = existingTokens ? structuredClone(existingTokens) : {};
  const contracts: Manifest = existingContracts ? structuredClone(existingContracts) : {};

  // Discover tokens from edges + wallet nodes so user knows what to fill in
  const edgeTokens = new Set(defiEdges.map((e) => e.token));
  for (const node of defiNodes) {
    if (node.type === "wallet" && node.token) edgeTokens.add(node.token);
  }

  // Collect all chains referenced in nodes
  const chainsByToken = new Map<string, Set<string>>();
  for (const tok of edgeTokens) {
    chainsByToken.set(tok, new Set());
  }
  for (const node of defiNodes) {
    if ("chain" in node && node.chain) {
      const chainName = node.chain.name;
      // Each edge token could appear on this chain
      for (const tok of edgeTokens) {
        chainsByToken.get(tok)?.add(chainName);
      }
    }
  }

  // Ensure every token+chain combo has an entry (even if empty string = "fill me in")
  for (const [tok, chains] of chainsByToken) {
    if (!tokens[tok]) tokens[tok] = {};
    for (const chain of chains) {
      if (!(chain in tokens[tok])) {
        tokens[tok][chain] = ""; // placeholder for user to fill
      }
    }
  }

  // Merge contract references from lending/vault nodes
  for (const node of defiNodes) {
    if (node.type === "lending") {
      if (node.pool_address && !contracts[node.pool_address]) {
        contracts[node.pool_address] = {};
      }
      if (node.pool_address && node.chain) {
        if (!contracts[node.pool_address]) contracts[node.pool_address] = {};
        if (!(node.chain.name in contracts[node.pool_address])) {
          contracts[node.pool_address][node.chain.name] = "";
        }
      }
      if (node.rewards_controller) {
        if (!contracts[node.rewards_controller]) contracts[node.rewards_controller] = {};
        if (node.chain && !(node.chain.name in contracts[node.rewards_controller])) {
          contracts[node.rewards_controller][node.chain.name] = "";
        }
      }
    }
    if (node.type === "vault") {
      if (node.vault_address && !contracts[node.vault_address]) {
        contracts[node.vault_address] = {};
      }
      if (node.vault_address && node.chain) {
        if (!contracts[node.vault_address]) contracts[node.vault_address] = {};
        if (!(node.chain.name in contracts[node.vault_address])) {
          contracts[node.vault_address][node.chain.name] = "";
        }
      }
    }
  }

  // Clean out empty-string-only manifests — keep entries that have real addresses
  const cleanManifest = (m: Manifest): Manifest | undefined => {
    const result: Manifest = {};
    for (const [key, chains] of Object.entries(m)) {
      const nonEmpty: Record<string, string> = {};
      for (const [chain, addr] of Object.entries(chains)) {
        if (addr) nonEmpty[chain] = addr;
      }
      if (Object.keys(nonEmpty).length > 0) result[key] = nonEmpty;
    }
    return Object.keys(result).length > 0 ? result : undefined;
  };

  return {
    name,
    description: description || undefined,
    tokens: cleanManifest(tokens),
    contracts: cleanManifest(contracts),
    nodes: defiNodes,
    edges: defiEdges,
  };
}

/**
 * Convert defi-flow JSON → canvas state for import.
 */
export function convertDefiFlowToCanvas(workflow: DefiFlowWorkflow): {
  nodes: CanvasNode[];
  edges: CanvasEdge[];
  tokens?: Manifest;
  contracts?: Manifest;
} {
  // Simple auto-layout: arrange nodes in rows
  const SPACING_X = 320;
  const SPACING_Y = 180;
  const COLS = 4;

  const nodes: CanvasNode[] = workflow.nodes.map((defiNode, i) => ({
    id: defiNode.id,
    type: "defi-node",
    position: {
      x: (i % COLS) * SPACING_X,
      y: Math.floor(i / COLS) * SPACING_Y,
    },
    data: {
      defiNode,
      label: getNodeLabel(defiNode),
      status: "idle" as const,
    },
  }));

  const edges: CanvasEdge[] = workflow.edges.map((defiEdge, i) => ({
    id: `edge-${i}`,
    source: defiEdge.from_node,
    target: defiEdge.to_node,
    type: "defi-edge",
    data: {
      token: defiEdge.token,
      amount: defiEdge.amount,
      status: "valid" as const,
    },
  }));

  return {
    nodes,
    edges,
    tokens: workflow.tokens,
    contracts: workflow.contracts,
  };
}
