"use client";

import {
  ConnectionMode,
  MiniMap,
  type OnConnect,
  type OnConnectStartParams,
  useReactFlow,
  type Connection as XYFlowConnection,
  type Edge as XYFlowEdge,
} from "@xyflow/react";
import { useAtom, useSetAtom } from "jotai";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Canvas } from "@/components/ai-elements/canvas";
import { Connection } from "@/components/ai-elements/connection";
import { Controls } from "@/components/ai-elements/controls";
import { WorkflowToolbar } from "@/components/workflow/workflow-toolbar";
import "@xyflow/react/dist/style.css";

import { nanoid } from "nanoid";
import {
  addNodeAtom,
  autosaveAtom,
  deleteSelectedItemsAtom,
  edgesAtom,
  nodesAtom,
  onEdgesChangeAtom,
  onNodesChangeAtom,
  redoAtom,
  selectedEdgeAtom,
  selectedNodeAtom,
  showMinimapAtom,
  undoAtom,
} from "@/lib/workflow-store";
import { createDefaultNode, getNodeLabel, inferEdgeToken } from "@/lib/types/defi-flow";
import type { CanvasNode, CanvasEdge } from "@/lib/types/canvas";
import { DefiNode } from "./nodes/defi-node";
import { DefiEdge } from "./edges/defi-edge";
import { Panel } from "../ai-elements/panel";

const nodeTypes = { "defi-node": DefiNode } as any;
const edgeTypes = { "defi-edge": DefiEdge } as any;

export function WorkflowCanvas() {
  const [nodes, setNodes] = useAtom(nodesAtom);
  const [edges, setEdges] = useAtom(edgesAtom);
  const [showMinimap] = useAtom(showMinimapAtom);
  const onNodesChange = useSetAtom(onNodesChangeAtom);
  const onEdgesChange = useSetAtom(onEdgesChangeAtom);
  const setSelectedNode = useSetAtom(selectedNodeAtom);
  const setSelectedEdge = useSetAtom(selectedEdgeAtom);
  const addNode = useSetAtom(addNodeAtom);
  const triggerAutosave = useSetAtom(autosaveAtom);
  const undo = useSetAtom(undoAtom);
  const redo = useSetAtom(redoAtom);
  const deleteSelected = useSetAtom(deleteSelectedItemsAtom);
  const { screenToFlowPosition, fitView } = useReactFlow();

  // Auto-sync edge tokens whenever nodes change
  // Auto-sync edge tokens + sourceType whenever nodes change
  const prevNodeDataRef = useRef<string>("");
  useEffect(() => {
    const fingerprint = nodes.map((n) => `${n.id}:${JSON.stringify(n.data.defiNode)}`).join("|");
    if (fingerprint === prevNodeDataRef.current) return;
    prevNodeDataRef.current = fingerprint;

    let changed = false;
    const updated = edges.map((edge) => {
      const src = nodes.find((n) => n.id === edge.source);
      const tgt = nodes.find((n) => n.id === edge.target);
      if (!src || !tgt) return edge;
      const inferred = inferEdgeToken(src.data.defiNode, tgt.data.defiNode);
      const srcType = src.data.defiNode.type;
      if (edge.data?.token === inferred && edge.data?.sourceType === srcType) return edge;
      changed = true;
      return { ...edge, data: { ...edge.data!, token: inferred, sourceType: srcType } } as CanvasEdge;
    });
    if (changed) setEdges(updated);
  }, [nodes, edges, setEdges]);

  const connectingNodeId = useRef<string | null>(null);
  const connectingHandleType = useRef<"source" | "target" | null>(null);
  const justCreatedNode = useRef(false);

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      // Ignore if user is typing in an input
      const tag = (event.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (mod && event.key === "/") {
        event.preventDefault();
        fitView({ padding: 0.2, duration: 300 });
      } else if (mod && event.shiftKey && event.key === "z") {
        event.preventDefault();
        redo();
      } else if (mod && event.key === "z") {
        event.preventDefault();
        undo();
      } else if (mod && event.key === "s") {
        event.preventDefault();
        triggerAutosave({ immediate: true });
      } else if (event.key === "Delete" || event.key === "Backspace") {
        event.preventDefault();
        deleteSelected();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [fitView, undo, redo, triggerAutosave, deleteSelected]);

  const isValidConnection = useCallback(
    (connection: XYFlowConnection | XYFlowEdge) => {
      if (!(connection.source && connection.target)) return false;
      if (connection.source === connection.target) return false;
      return true;
    },
    []
  );

  const onConnect: OnConnect = useCallback(
    (connection: XYFlowConnection) => {
      // Auto-detect edge token from source/target nodes
      const srcNode = nodes.find((n) => n.id === connection.source);
      const tgtNode = nodes.find((n) => n.id === connection.target);
      const token =
        srcNode && tgtNode
          ? inferEdgeToken(srcNode.data.defiNode, tgtNode.data.defiNode)
          : "USDC";

      const newEdge: CanvasEdge = {
        id: nanoid(),
        ...connection,
        type: "defi-edge",
        data: {
          token,
          amount: { type: "all" },
          sourceType: srcNode?.data.defiNode.type,
        },
      };
      setEdges([...edges, newEdge]);
      triggerAutosave({ immediate: true });
    },
    [nodes, edges, setEdges, triggerAutosave]
  );

  const onConnectStart = useCallback(
    (_event: MouseEvent | TouchEvent, params: OnConnectStartParams) => {
      connectingNodeId.current = params.nodeId;
      connectingHandleType.current = params.handleType;
    },
    []
  );

  const onConnectEnd = useCallback(
    (event: MouseEvent | TouchEvent) => {
      if (!connectingNodeId.current) return;

      const clientX = "changedTouches" in event ? event.changedTouches[0].clientX : event.clientX;
      const clientY = "changedTouches" in event ? event.changedTouches[0].clientY : event.clientY;
      const target = "changedTouches" in event
        ? document.elementFromPoint(clientX, clientY)
        : (event.target as Element);

      if (!target) { connectingNodeId.current = null; return; }

      const nodeElement = target.closest(".react-flow__node");
      const isHandle = target.closest(".react-flow__handle");

      // Don't create new node if we landed on an existing node or handle
      if (nodeElement || isHandle) {
        connectingNodeId.current = null;
        connectingHandleType.current = null;
        return;
      }

      // Create new perp node at drop position
      const sourceId = connectingNodeId.current;
      const nodeId = `perp_${nanoid(4)}`;
      const defiNode = createDefaultNode("perp", nodeId);
      const position = screenToFlowPosition({ x: clientX, y: clientY - 50 });

      const newNode: CanvasNode = {
        id: nodeId,
        type: "defi-node",
        position,
        data: { defiNode, label: getNodeLabel(defiNode), status: "idle" },
        selected: true,
      };

      addNode(newNode);
      setSelectedNode(nodeId);

      const fromSource = connectingHandleType.current === "source";
      const edgeSource = fromSource ? sourceId : nodeId;
      const edgeTarget = fromSource ? nodeId : sourceId;
      const srcNode = nodes.find((n) => n.id === edgeSource);
      const tgtNode = edgeTarget === nodeId ? newNode : nodes.find((n) => n.id === edgeTarget);
      const edgeToken =
        srcNode && tgtNode
          ? inferEdgeToken(srcNode.data.defiNode, tgtNode.data.defiNode)
          : "USDC";

      const newEdge: CanvasEdge = {
        id: nanoid(),
        source: edgeSource,
        target: edgeTarget,
        type: "defi-edge",
        data: { token: edgeToken, amount: { type: "all" }, sourceType: srcNode?.data.defiNode.type },
      };
      setEdges([...edges, newEdge]);
      triggerAutosave({ immediate: true });

      justCreatedNode.current = true;
      setTimeout(() => { justCreatedNode.current = false; }, 100);

      connectingNodeId.current = null;
      connectingHandleType.current = null;
    },
    [screenToFlowPosition, addNode, edges, setEdges, setSelectedNode, triggerAutosave]
  );

  const onPaneClick = useCallback(() => {
    if (justCreatedNode.current) return;
    setSelectedNode(null);
    setSelectedEdge(null);
  }, [setSelectedNode, setSelectedEdge]);

  return (
    <div className="relative h-full w-full bg-background">
      <WorkflowToolbar />
      <Canvas
        className="bg-background"
        connectionLineComponent={Connection}
        connectionMode={ConnectionMode.Strict}
        edges={edges}
        edgeTypes={edgeTypes}
        isValidConnection={isValidConnection}
        nodes={nodes}
        nodeTypes={nodeTypes}
        onConnect={onConnect}
        onConnectEnd={onConnectEnd}
        onConnectStart={onConnectStart}
        onEdgesChange={onEdgesChange}
        onEdgeClick={(_, edge) => { setSelectedEdge(edge.id); setSelectedNode(null); }}
        onNodeClick={(_, node) => setSelectedNode(node.id)}
        onNodesChange={onNodesChange}
        onPaneClick={onPaneClick}
      >
        <Panel
          className="workflow-controls-panel border-none bg-transparent p-0"
          position="bottom-left"
        >
          <Controls />
        </Panel>
        {showMinimap && (
          <MiniMap bgColor="var(--sidebar)" nodeStrokeColor="var(--border)" />
        )}
      </Canvas>
    </div>
  );
}
