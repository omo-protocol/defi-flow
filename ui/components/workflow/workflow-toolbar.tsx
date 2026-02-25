"use client";

import { useAtomValue, useSetAtom, useAtom } from "jotai";
import {
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  canUndoAtom,
  canRedoAtom,
  undoAtom,
  redoAtom,
  addNodeAtom,
  autosaveAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import { NODE_REGISTRY, CATEGORIES, type NodeTypeConfig } from "@/lib/node-registry";
import { createDefaultNode, getNodeLabel, type DefiNodeType } from "@/lib/types/defi-flow";
import type { CanvasNode } from "@/lib/types/canvas";
import { convertCanvasToDefiFlow, convertDefiFlowToCanvas } from "@/lib/converters/canvas-defi-flow";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Download,
  Upload,
  Plus,
  Undo2,
  Redo2,
  CheckCircle,
  Save,
} from "lucide-react";
import { useReactFlow } from "@xyflow/react";
import { nanoid } from "nanoid";
import { toast } from "sonner";
import { validateWorkflow } from "@/lib/wasm";
import { validateWorkflow as validateWorkflowApi } from "@/lib/api";
import { useState } from "react";

export function WorkflowToolbar() {
  const [name, setName] = useAtom(workflowNameAtom);
  const [nodes, setNodes] = useAtom(nodesAtom);
  const [edges, setEdges] = useAtom(edgesAtom);
  const canUndo = useAtomValue(canUndoAtom);
  const canRedo = useAtomValue(canRedoAtom);
  const undo = useSetAtom(undoAtom);
  const redo = useSetAtom(redoAtom);
  const addNode = useSetAtom(addNodeAtom);
  const save = useSetAtom(autosaveAtom);
  const [tokensManifest, setTokensManifest] = useAtom(tokensManifestAtom);
  const [contractsManifest, setContractsManifest] = useAtom(contractsManifestAtom);
  const { screenToFlowPosition, fitView } = useReactFlow();

  // ── Add node ───────────────────────────────────────────────────────

  const handleAddNode = (config: NodeTypeConfig) => {
    const nodeId = `${config.type}_${nanoid(4)}`;
    const defiNode = createDefaultNode(config.type, nodeId);

    const position = screenToFlowPosition({
      x: window.innerWidth / 2 - 120,
      y: window.innerHeight / 2 - 50,
    });

    // Offset to avoid overlapping
    const offset = nodes.length * 20;
    position.x += offset;
    position.y += offset;

    const newNode: CanvasNode = {
      id: nodeId,
      type: "defi-node",
      position,
      data: {
        defiNode,
        label: getNodeLabel(defiNode),
        status: "idle",
      },
    };

    addNode(newNode);
  };

  // ── Import ─────────────────────────────────────────────────────────

  const handleImport = () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json";
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        // Validate with WASM first
        const result = await validateWorkflow(text);
        if (!result.valid) {
          toast.warning("Imported with warnings", {
            description: (result.errors ?? [])[0],
          });
        }
        const workflow: DefiFlowWorkflow = JSON.parse(text);
        const { nodes: newNodes, edges: newEdges, tokens, contracts } = convertDefiFlowToCanvas(workflow);
        setNodes(newNodes);
        setEdges(newEdges);
        setName(workflow.name || "Imported Strategy");
        setTokensManifest(tokens);
        setContractsManifest(contracts);
        toast.success(`Imported "${workflow.name}" (${workflow.nodes.length} nodes)`);
        setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 100);
      } catch (err) {
        toast.error("Failed to import: " + (err instanceof Error ? err.message : "Invalid JSON"));
      }
    };
    input.click();
  };

  // ── Export ─────────────────────────────────────────────────────────

  const handleExport = () => {
    const workflow = convertCanvasToDefiFlow(nodes, edges, name, undefined, tokensManifest, contractsManifest);
    const json = JSON.stringify(workflow, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${name.toLowerCase().replace(/\s+/g, "_")}.json`;
    a.click();
    URL.revokeObjectURL(url);
    toast.success("Strategy exported");
  };

  // ── Validate ───────────────────────────────────────────────────────

  const [validating, setValidating] = useState(false);
  const [validatingOnchain, setValidatingOnchain] = useState(false);

  const handleValidateOnchain = async () => {
    setValidatingOnchain(true);
    try {
      const workflow = convertCanvasToDefiFlow(nodes, edges, name, undefined, tokensManifest, contractsManifest);
      const result = await validateWorkflowApi(workflow, true);

      if (result.valid) {
        setNodes(nodes.map((n) => ({
          ...n,
          data: { ...n.data, status: "valid" as const, validationErrors: undefined },
        })));
        toast.success("On-chain validation passed");
        if (result.warnings?.length) {
          for (const w of result.warnings) {
            toast.warning(w);
          }
        }
      } else {
        const nodeErrorMap = new Map<string, string[]>();
        const globalErrors: string[] = [];
        for (const err of result.errors ?? []) {
          const nodeMatch = err.match(/node [`"]([^`"]+)[`"]/);
          if (nodeMatch) {
            const id = nodeMatch[1];
            const existing = nodeErrorMap.get(id) ?? [];
            existing.push(err);
            nodeErrorMap.set(id, existing);
          } else {
            globalErrors.push(err);
          }
        }
        setNodes(nodes.map((n) => {
          const errs = nodeErrorMap.get(n.data.defiNode.id);
          return {
            ...n,
            data: {
              ...n.data,
              status: errs ? ("error" as const) : ("valid" as const),
              validationErrors: errs,
            },
          };
        }));
        toast.error(`${(result.errors ?? []).length} error(s)`, {
          description: globalErrors[0],
        });
      }
    } catch (err) {
      toast.error("On-chain validation failed: " + (err instanceof Error ? err.message : "API error"));
    } finally {
      setValidatingOnchain(false);
    }
  };

  const handleValidate = async () => {
    setValidating(true);
    try {
      // Convert canvas → defi-flow JSON, then run Rust validator via WASM
      const workflow = convertCanvasToDefiFlow(nodes, edges, name, undefined, tokensManifest, contractsManifest);
      const json = JSON.stringify(workflow);
      const result = await validateWorkflow(json);

      if (result.valid) {
        // Mark all nodes as valid
        setNodes(nodes.map((n) => ({
          ...n,
          data: { ...n.data, status: "valid" as const, validationErrors: undefined },
        })));
        toast.success(`Strategy is valid (${nodes.length} nodes, ${edges.length} edges)`);
      } else {
        // Try to map errors to specific nodes
        const nodeErrorMap = new Map<string, string[]>();
        const globalErrors: string[] = [];

        for (const err of result.errors ?? []) {
          // Try to extract node ID from error message patterns like `node "xyz"` or `node_id`
          const nodeMatch = err.match(/node [`"]([^`"]+)[`"]/);
          if (nodeMatch) {
            const id = nodeMatch[1];
            const existing = nodeErrorMap.get(id) ?? [];
            existing.push(err);
            nodeErrorMap.set(id, existing);
          } else {
            globalErrors.push(err);
          }
        }

        setNodes(nodes.map((n) => {
          const errs = nodeErrorMap.get(n.data.defiNode.id);
          return {
            ...n,
            data: {
              ...n.data,
              status: errs ? ("error" as const) : ("valid" as const),
              validationErrors: errs,
            },
          };
        }));

        const totalErrors = (result.errors ?? []).length;
        toast.error(`${totalErrors} validation error(s) found`, {
          description: globalErrors.length > 0 ? globalErrors[0] : undefined,
        });
      }
    } catch (err) {
      toast.error("Validation failed: " + (err instanceof Error ? err.message : "WASM error"));
    } finally {
      setValidating(false);
    }
  };

  return (
    <div className="pointer-events-auto absolute top-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-1.5 bg-card/95 backdrop-blur border rounded-lg px-3 py-1.5 shadow-lg">
      {/* Name */}
      <Input
        className="h-7 w-44 text-xs font-medium border-none bg-transparent p-0 focus-visible:ring-0"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="Strategy Name"
      />

      <Separator orientation="vertical" className="h-5" />

      {/* Add Node */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="h-7 px-2 text-xs">
            <Plus className="w-3.5 h-3.5 mr-1" />
            Add Node
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="w-52" align="center">
          {CATEGORIES.map((cat) => (
            <div key={cat.key}>
              <DropdownMenuLabel className="text-[10px] uppercase tracking-wider text-muted-foreground">
                {cat.label}
              </DropdownMenuLabel>
              {NODE_REGISTRY.filter((n) => n.category === cat.key).map((config) => {
                const Icon = config.icon;
                return (
                  <DropdownMenuItem
                    key={config.type}
                    onClick={() => handleAddNode(config)}
                    className="text-xs"
                  >
                    <Icon className="w-3.5 h-3.5 mr-2" />
                    {config.label}
                    <span className="ml-auto text-[10px] text-muted-foreground">
                      {config.description.slice(0, 30)}
                    </span>
                  </DropdownMenuItem>
                );
              })}
              <DropdownMenuSeparator />
            </div>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <Separator orientation="vertical" className="h-5" />

      {/* Import / Export */}
      <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={handleImport}>
        <Upload className="w-3.5 h-3.5 mr-1" />
        Import
      </Button>
      <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={handleExport}>
        <Download className="w-3.5 h-3.5 mr-1" />
        Export
      </Button>
      <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={handleValidate} disabled={validating}>
        <CheckCircle className="w-3.5 h-3.5 mr-1" />
        {validating ? "Validating..." : "Validate"}
      </Button>
      <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={handleValidateOnchain} disabled={validatingOnchain}>
        <CheckCircle className="w-3.5 h-3.5 mr-1" />
        {validatingOnchain ? "Checking..." : "On-Chain"}
      </Button>

      <Separator orientation="vertical" className="h-5" />

      {/* Undo / Redo */}
      <Button variant="ghost" size="sm" className="h-7 w-7 p-0" disabled={!canUndo} onClick={() => undo()}>
        <Undo2 className="w-3.5 h-3.5" />
      </Button>
      <Button variant="ghost" size="sm" className="h-7 w-7 p-0" disabled={!canRedo} onClick={() => redo()}>
        <Redo2 className="w-3.5 h-3.5" />
      </Button>

      <Separator orientation="vertical" className="h-5" />

      {/* Save */}
      <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => save({ immediate: true })}>
        <Save className="w-3.5 h-3.5 mr-1" />
        Save
      </Button>
    </div>
  );
}
