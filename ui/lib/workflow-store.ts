import type { EdgeChange, NodeChange } from "@xyflow/react";
import { applyEdgeChanges, applyNodeChanges } from "@xyflow/react";
import { atom } from "jotai";
import type { CanvasNode, CanvasEdge } from "./types/canvas";

// ── Core canvas state ────────────────────────────────────────────────

export const nodesAtom = atom<CanvasNode[]>([]);
export const edgesAtom = atom<CanvasEdge[]>([]);
export const selectedNodeAtom = atom<string | null>(null);
export const selectedEdgeAtom = atom<string | null>(null);

// ── Workflow metadata ────────────────────────────────────────────────

export const workflowNameAtom = atom<string>("Untitled Strategy");
export const workflowDescriptionAtom = atom<string>("");

// ── Manifests (tokens & contracts address maps) ─────────────────────

type Manifest = Record<string, Record<string, string>>;

export const tokensManifestAtom = atom<Manifest | undefined>(undefined);
export const contractsManifestAtom = atom<Manifest | undefined>(undefined);

// ── User settings (in-memory only) ──────────────────────────────────

export const privateKeyAtom = atom<string>("");
export const walletAddressAtom = atom<string>("");

// ── UI state ─────────────────────────────────────────────────────────

export const rightPanelWidthAtom = atom<string | null>("30%");
export const isPanelAnimatingAtom = atom<boolean>(false);
export const showMinimapAtom = atom(false);

// ── Autosave to localStorage ─────────────────────────────────────────

let autosaveTimer: ReturnType<typeof setTimeout> | null = null;

export const autosaveAtom = atom(
  null,
  (get, _set, options?: { immediate?: boolean }) => {
    const doSave = () => {
      const nodes = get(nodesAtom);
      const edges = get(edgesAtom);
      const name = get(workflowNameAtom);
      const desc = get(workflowDescriptionAtom);
      const tokens = get(tokensManifestAtom);
      const contracts = get(contractsManifestAtom);
      try {
        localStorage.setItem(
          "defi-flow-current",
          JSON.stringify({ name, description: desc, nodes, edges, tokens, contracts })
        );
      } catch {
        // quota exceeded — ignore
      }
    };

    if (options?.immediate) {
      doSave();
    } else {
      if (autosaveTimer) clearTimeout(autosaveTimer);
      autosaveTimer = setTimeout(doSave, 1000);
    }
  }
);

// ── Undo / Redo ──────────────────────────────────────────────────────

type HistoryState = { nodes: CanvasNode[]; edges: CanvasEdge[] };

const historyAtom = atom<HistoryState[]>([]);
const futureAtom = atom<HistoryState[]>([]);
export const canUndoAtom = atom((get) => get(historyAtom).length > 0);
export const canRedoAtom = atom((get) => get(futureAtom).length > 0);

function pushHistory(get: (a: typeof nodesAtom) => CanvasNode[], set: Function) {
  const nodes = get(nodesAtom as any) as CanvasNode[];
  const edges = (get as any)(edgesAtom) as CanvasEdge[];
  const history = (get as any)(historyAtom) as HistoryState[];
  (set as any)(historyAtom, [...history.slice(-50), { nodes, edges }]);
  (set as any)(futureAtom, []);
}

export const undoAtom = atom(null, (get, set) => {
  const history = get(historyAtom);
  if (history.length === 0) return;
  const current = { nodes: get(nodesAtom), edges: get(edgesAtom) };
  set(futureAtom, [...get(futureAtom), current]);
  const prev = history[history.length - 1];
  set(historyAtom, history.slice(0, -1));
  set(nodesAtom, prev.nodes);
  set(edgesAtom, prev.edges);
});

export const redoAtom = atom(null, (get, set) => {
  const future = get(futureAtom);
  if (future.length === 0) return;
  const current = { nodes: get(nodesAtom), edges: get(edgesAtom) };
  set(historyAtom, [...get(historyAtom), current]);
  const next = future[future.length - 1];
  set(futureAtom, future.slice(0, -1));
  set(nodesAtom, next.nodes);
  set(edgesAtom, next.edges);
});

// ── Node CRUD ────────────────────────────────────────────────────────

export const addNodeAtom = atom(null, (get, set, node: CanvasNode) => {
  const current = get(nodesAtom);
  const edges = get(edgesAtom);
  set(historyAtom, [...get(historyAtom).slice(-50), { nodes: current, edges }]);
  set(futureAtom, []);

  const updated = current.map((n) => ({ ...n, selected: false }));
  set(nodesAtom, [...updated, { ...node, selected: true }]);
  set(selectedNodeAtom, node.id);
  set(autosaveAtom, { immediate: true });
});

export const updateNodeDataAtom = atom(
  null,
  (get, set, { id, data }: { id: string; data: Partial<CanvasNode["data"]> }) => {
    const nodes = get(nodesAtom);
    set(
      nodesAtom,
      nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...data } } : n))
    );
    set(autosaveAtom);
  }
);

export const deleteNodeAtom = atom(null, (get, set, nodeId: string) => {
  const nodes = get(nodesAtom);
  const edges = get(edgesAtom);
  set(historyAtom, [...get(historyAtom).slice(-50), { nodes, edges }]);
  set(futureAtom, []);
  set(
    nodesAtom,
    nodes.filter((n) => n.id !== nodeId)
  );
  set(
    edgesAtom,
    edges.filter((e) => e.source !== nodeId && e.target !== nodeId)
  );
  if (get(selectedNodeAtom) === nodeId) set(selectedNodeAtom, null);
  set(autosaveAtom, { immediate: true });
});

export const deleteEdgeAtom = atom(null, (get, set, edgeId: string) => {
  const nodes = get(nodesAtom);
  const edges = get(edgesAtom);
  set(historyAtom, [...get(historyAtom).slice(-50), { nodes, edges }]);
  set(futureAtom, []);
  set(
    edgesAtom,
    edges.filter((e) => e.id !== edgeId)
  );
  if (get(selectedEdgeAtom) === edgeId) set(selectedEdgeAtom, null);
  set(autosaveAtom, { immediate: true });
});

export const deleteSelectedItemsAtom = atom(null, (get, set) => {
  const nodes = get(nodesAtom);
  const edges = get(edgesAtom);
  set(historyAtom, [...get(historyAtom).slice(-50), { nodes, edges }]);
  set(futureAtom, []);

  const selectedNodeIds = nodes.filter((n) => n.selected).map((n) => n.id);
  set(
    nodesAtom,
    nodes.filter((n) => !n.selected)
  );
  set(
    edgesAtom,
    edges.filter(
      (e) =>
        !e.selected &&
        !selectedNodeIds.includes(e.source) &&
        !selectedNodeIds.includes(e.target)
    )
  );
  set(selectedNodeAtom, null);
  set(selectedEdgeAtom, null);
  set(autosaveAtom, { immediate: true });
});

// ── React Flow change handlers ───────────────────────────────────────

export const onNodesChangeAtom = atom(null, (get, set, changes: NodeChange[]) => {
  const current = get(nodesAtom);
  const newNodes = applyNodeChanges(changes, current) as CanvasNode[];
  set(nodesAtom, newNodes);

  // Sync selection
  const selected = newNodes.find((n) => n.selected);
  if (selected) {
    set(selectedNodeAtom, selected.id);
    set(selectedEdgeAtom, null);
  } else {
    const prev = get(selectedNodeAtom);
    if (prev && !newNodes.find((n) => n.id === prev)) {
      set(selectedNodeAtom, null);
    }
  }

  // Autosave on deletions/position changes
  if (changes.some((c) => c.type === "remove")) {
    set(autosaveAtom, { immediate: true });
  } else if (changes.some((c) => c.type === "position" && !("dragging" in c && c.dragging))) {
    set(autosaveAtom);
  }
});

export const onEdgesChangeAtom = atom(null, (get, set, changes: EdgeChange[]) => {
  const current = get(edgesAtom);
  const newEdges = applyEdgeChanges(changes, current) as CanvasEdge[];
  set(edgesAtom, newEdges);

  const selected = newEdges.find((e) => e.selected);
  if (selected) {
    set(selectedEdgeAtom, selected.id);
    set(selectedNodeAtom, null);
  } else {
    const prev = get(selectedEdgeAtom);
    if (prev && !newEdges.find((e) => e.id === prev)) {
      set(selectedEdgeAtom, null);
    }
  }

  if (changes.some((c) => c.type === "remove")) {
    set(autosaveAtom, { immediate: true });
  }
});
