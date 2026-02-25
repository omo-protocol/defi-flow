"use client";

import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { atom } from "jotai";
import {
  rightPanelWidthAtom,
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  selectedNodeAtom,
  selectedEdgeAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import { WorkflowCanvas } from "@/components/workflow/workflow-canvas";
import { NodeConfigPanel } from "@/components/workflow/node-config-panel";
import { StatusDashboard } from "@/components/workflow/status-dashboard";
import { AgentPanel } from "@/components/ai-agent/agent-panel";
import { useEffect, useState } from "react";
import { convertDefiFlowToCanvas } from "@/lib/converters/canvas-defi-flow";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";
import { useReactFlow } from "@xyflow/react";

// Right panel mode: "config" when a node/edge is selected, "engine" for backtest/run, "agent" for AI builder
export const panelModeAtom = atom<"config" | "engine" | "agent">("config");

const EXAMPLES = [
  { name: "Delta Neutral v1", file: "delta_neutral.json" },
  { name: "Delta Neutral v2", file: "delta_neutral_v2.json" },
];

function WelcomeOverlay({ onClose }: { onClose: () => void }) {
  const setNodes = useSetAtom(nodesAtom);
  const setEdges = useSetAtom(edgesAtom);
  const setName = useSetAtom(workflowNameAtom);
  const setTokens = useSetAtom(tokensManifestAtom);
  const setContracts = useSetAtom(contractsManifestAtom);
  const { fitView } = useReactFlow();

  const loadExample = async (file: string) => {
    try {
      const res = await fetch(`/examples/${file}`);
      const workflow: DefiFlowWorkflow = await res.json();
      const { nodes, edges, tokens, contracts } = convertDefiFlowToCanvas(workflow);
      setNodes(nodes);
      setEdges(edges);
      setName(workflow.name);
      setTokens(tokens);
      setContracts(contracts);
      onClose();
      setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 200);
    } catch {
      onClose();
    }
  };

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm">
      <div className="bg-card border rounded-xl shadow-2xl p-8 max-w-md w-full mx-4">
        <h1 className="text-xl font-bold mb-1">DeFi Flow</h1>
        <p className="text-sm text-muted-foreground mb-6">
          Visual strategy builder for DeFi workflows. Add nodes, connect them
          with token flows, configure parameters, and export valid strategy JSON.
        </p>

        <div className="space-y-2 mb-6">
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
            Load Example
          </p>
          {EXAMPLES.map((ex) => (
            <button
              key={ex.file}
              onClick={() => loadExample(ex.file)}
              className="w-full text-left px-4 py-3 rounded-lg border hover:bg-accent transition-colors text-sm"
            >
              {ex.name}
            </button>
          ))}
        </div>

        <button
          onClick={onClose}
          className="w-full px-4 py-2.5 rounded-lg bg-primary text-primary-foreground font-medium text-sm hover:bg-primary/90 transition-colors"
        >
          Start from Scratch
        </button>
      </div>
    </div>
  );
}

function RightPanel() {
  const [panelMode, setPanelMode] = useAtom(panelModeAtom);
  const selectedNodeId = useAtomValue(selectedNodeAtom);
  const selectedEdgeId = useAtomValue(selectedEdgeAtom);

  // Auto-switch to config when a node/edge is selected
  useEffect(() => {
    if (selectedNodeId || selectedEdgeId) {
      setPanelMode("config");
    }
  }, [selectedNodeId, selectedEdgeId, setPanelMode]);

  return (
    <div className="h-full flex flex-col">
      {/* Tab bar */}
      <div className="flex border-b bg-card">
        <button
          onClick={() => setPanelMode("config")}
          className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
            panelMode === "config"
              ? "text-foreground border-b-2 border-primary"
              : "text-muted-foreground hover:text-foreground"
          }`}
        >
          Config
        </button>
        <button
          onClick={() => setPanelMode("engine")}
          className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
            panelMode === "engine"
              ? "text-foreground border-b-2 border-primary"
              : "text-muted-foreground hover:text-foreground"
          }`}
        >
          Engine
        </button>
        <button
          onClick={() => setPanelMode("agent")}
          className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
            panelMode === "agent"
              ? "text-foreground border-b-2 border-primary"
              : "text-muted-foreground hover:text-foreground"
          }`}
        >
          Agent
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {panelMode === "config" ? (
          <NodeConfigPanel />
        ) : panelMode === "engine" ? (
          <StatusDashboard />
        ) : (
          <AgentPanel />
        )}
      </div>
    </div>
  );
}

export default function Home() {
  const rightPanelWidth = useAtomValue(rightPanelWidthAtom);
  const [nodes, setNodes] = useAtom(nodesAtom);
  const setEdges = useSetAtom(edgesAtom);
  const setName = useSetAtom(workflowNameAtom);
  const setTokensManifest = useSetAtom(tokensManifestAtom);
  const setContractsManifest = useSetAtom(contractsManifestAtom);
  const [showWelcome, setShowWelcome] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Restore from localStorage on mount
  useEffect(() => {
    try {
      const saved = localStorage.getItem("defi-flow-current");
      if (saved) {
        const { name, nodes: n, edges: e, tokens, contracts } = JSON.parse(saved);
        if (n && n.length > 0) {
          setNodes(n);
          setEdges(e);
          if (name) setName(name);
          if (tokens) setTokensManifest(tokens);
          if (contracts) setContractsManifest(contracts);
          setLoaded(true);
          return;
        }
      }
    } catch {
      // corrupt data — ignore
    }
    // No saved data — show welcome
    setShowWelcome(true);
    setLoaded(true);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (!loaded) return null;

  return (
    <div className="h-dvh w-full flex">
      {/* Canvas */}
      <div className="flex-1 relative">
        <WorkflowCanvas />
      </div>

      {/* Right panel — Config / Engine tabs */}
      {rightPanelWidth && (
        <div
          className="border-l bg-card h-full overflow-hidden"
          style={{ width: rightPanelWidth }}
        >
          <RightPanel />
        </div>
      )}

      {/* Welcome overlay */}
      {showWelcome && <WelcomeOverlay onClose={() => setShowWelcome(false)} />}
    </div>
  );
}
