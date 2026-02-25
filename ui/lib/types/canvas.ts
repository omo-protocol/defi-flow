import type { Node as ReactFlowNode, Edge as ReactFlowEdge } from "@xyflow/react";
import type { DefiNode, Amount } from "./defi-flow";

// ── Canvas node data ─────────────────────────────────────────────────

export type CanvasNodeData = {
  defiNode: DefiNode;
  label: string;
  status?: "idle" | "valid" | "error";
  validationErrors?: string[];
  [key: string]: unknown;
};

export type CanvasNode = ReactFlowNode<CanvasNodeData, "defi-node">;

// ── Canvas edge data ─────────────────────────────────────────────────

export type CanvasEdgeData = {
  token: string;
  amount: Amount;
  status?: "valid" | "error";
  validationError?: string;
  [key: string]: unknown;
};

export type CanvasEdge = ReactFlowEdge<CanvasEdgeData>;
