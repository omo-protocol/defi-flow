import type { CanvasNode, CanvasEdge } from "../types/canvas";
import type { DefiFlowWorkflow, DefiNode, DefiEdge, Amount } from "../types/defi-flow";
import { getNodeLabel } from "../types/defi-flow";

/**
 * Convert canvas state → defi-flow JSON for export.
 */
export function convertCanvasToDefiFlow(
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  name: string,
  description?: string
): DefiFlowWorkflow {
  const defiNodes: DefiNode[] = nodes.map((n) => n.data.defiNode);
  const defiEdges: DefiEdge[] = edges.map((e) => ({
    from_node: e.source,
    to_node: e.target,
    token: e.data?.token ?? "USDC",
    amount: e.data?.amount ?? { type: "all" as const },
  }));

  // Build tokens manifest from wallet nodes
  const tokens: Record<string, Record<string, string>> = {};
  for (const node of defiNodes) {
    if (node.type === "wallet" && node.token && node.chain) {
      if (!tokens[node.token]) tokens[node.token] = {};
      // Address placeholder — user fills in real token contract addresses via manifest
    }
  }

  // Build contracts manifest from lending/vault nodes
  const contracts: Record<string, Record<string, string>> = {};
  for (const node of defiNodes) {
    if (node.type === "lending" && node.pool_address) {
      if (!contracts[node.pool_address]) contracts[node.pool_address] = {};
    }
    if (node.type === "vault" && node.vault_address) {
      if (!contracts[node.vault_address]) contracts[node.vault_address] = {};
    }
  }

  return {
    name,
    description: description || undefined,
    tokens: Object.keys(tokens).length > 0 ? tokens : undefined,
    contracts: Object.keys(contracts).length > 0 ? contracts : undefined,
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

  return { nodes, edges };
}
