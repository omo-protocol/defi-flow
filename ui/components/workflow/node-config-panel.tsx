"use client";

import { useAtom, useAtomValue, useSetAtom } from "jotai";
import {
  nodesAtom,
  edgesAtom,
  selectedNodeAtom,
  selectedEdgeAtom,
  deleteNodeAtom,
  deleteEdgeAtom,
  updateNodeDataAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import type { CanvasEdge, CanvasEdgeData } from "@/lib/types/canvas";
import { getNodeConfig } from "@/lib/node-registry";
import { NodeConfigForm } from "./config/node-configs";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Trash2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

// ── Edge config ──────────────────────────────────────────────────────

function EdgeConfig({ edge }: { edge: CanvasEdge }) {
  const nodes = useAtomValue(nodesAtom);
  const [edges, setEdges] = useAtom(edgesAtom);
  const deleteEdge = useSetAtom(deleteEdgeAtom);

  const updateEdge = (partial: Partial<CanvasEdgeData>) => {
    setEdges(
      edges.map((e) =>
        e.id === edge.id
          ? ({ ...e, data: { ...e.data, ...partial } } as CanvasEdge)
          : e
      )
    );
  };

  const srcNode = nodes.find((n) => n.id === edge.source);
  const tgtNode = nodes.find((n) => n.id === edge.target);
  const isFromOptimizer = srcNode?.data.defiNode.type === "optimizer";

  const token = edge.data?.token ?? "";
  const amount = edge.data?.amount ?? { type: "all" as const };

  const srcLabel = srcNode?.data.label ?? edge.source;
  const tgtLabel = tgtNode?.data.label ?? edge.target;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">Edge Configuration</h3>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 w-7 p-0 text-destructive"
          onClick={() => deleteEdge(edge.id)}
        >
          <Trash2 className="w-3.5 h-3.5" />
        </Button>
      </div>

      <div className="text-xs text-muted-foreground">
        <span className="font-medium text-foreground">{srcLabel}</span>
        {" → "}
        <span className="font-medium text-foreground">{tgtLabel}</span>
      </div>

      <div className="space-y-1.5">
        <Label className="text-xs">Token</Label>
        <Input
          className="h-8 text-xs"
          value={token}
          onChange={(e) => updateEdge({ token: e.target.value })}
          placeholder="USDC"
        />
        <p className="text-[10px] text-muted-foreground">Auto-detected from nodes. Override if needed.</p>
      </div>

      {isFromOptimizer && (
        <>
          <div className="space-y-1.5">
            <Label className="text-xs">Amount Type</Label>
            <Select
              value={amount.type}
              onValueChange={(type) => {
                if (type === "all") updateEdge({ amount: { type: "all" } });
                else if (type === "percentage")
                  updateEdge({ amount: { type: "percentage", value: 100 } });
                else updateEdge({ amount: { type: "fixed", value: "1000.0" } });
              }}
            >
              <SelectTrigger className="h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All Available</SelectItem>
                <SelectItem value="percentage">Percentage</SelectItem>
                <SelectItem value="fixed">Fixed Amount</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {amount.type === "percentage" && (
            <div className="space-y-1.5">
              <Label className="text-xs">Percentage (%)</Label>
              <Input
                className="h-8 text-xs"
                type="number"
                value={amount.value}
                onChange={(e) =>
                  updateEdge({
                    amount: { type: "percentage", value: Number(e.target.value) },
                  })
                }
                min={0}
                max={100}
                step={1}
              />
            </div>
          )}

          {amount.type === "fixed" && (
            <div className="space-y-1.5">
              <Label className="text-xs">Amount</Label>
              <Input
                className="h-8 text-xs"
                value={amount.value}
                onChange={(e) =>
                  updateEdge({ amount: { type: "fixed", value: e.target.value } })
                }
                placeholder="1000.0"
              />
            </div>
          )}
        </>
      )}
    </div>
  );
}

function ManifestEditor({
  title,
  manifest,
  setManifest,
}: {
  title: string;
  manifest: Record<string, Record<string, string>> | undefined;
  setManifest: (m: Record<string, Record<string, string>> | undefined) => void;
}) {
  if (!manifest || Object.keys(manifest).length === 0) return null;

  const updateAddress = (key: string, chain: string, value: string) => {
    const updated = structuredClone(manifest);
    updated[key][chain] = value;
    setManifest(updated);
  };

  const renameKey = (oldKey: string, newKey: string) => {
    if (!newKey || newKey === oldKey) return;
    if (manifest[newKey]) return; // don't overwrite existing
    const updated: Record<string, Record<string, string>> = {};
    for (const [k, v] of Object.entries(manifest)) {
      updated[k === oldKey ? newKey : k] = v;
    }
    setManifest(updated);
  };

  const renameChain = (key: string, oldChain: string, newChain: string) => {
    if (!newChain || newChain === oldChain) return;
    const updated = structuredClone(manifest);
    const addr = updated[key][oldChain];
    delete updated[key][oldChain];
    updated[key][newChain] = addr;
    setManifest(updated);
  };

  const addEntry = () => {
    const updated = structuredClone(manifest) ?? {};
    const name = `new_${Object.keys(updated).length}`;
    updated[name] = { hyperevm: "" };
    setManifest(updated);
  };

  const addChain = (key: string) => {
    const updated = structuredClone(manifest);
    const existing = Object.keys(updated[key]);
    const newChain = existing.includes("hyperevm") ? "base" : "hyperevm";
    updated[key][newChain] = "";
    setManifest(updated);
  };

  const removeChain = (key: string, chain: string) => {
    const updated = structuredClone(manifest);
    delete updated[key][chain];
    if (Object.keys(updated[key]).length === 0) {
      delete updated[key];
    }
    setManifest(Object.keys(updated).length > 0 ? updated : undefined);
  };

  const removeEntry = (key: string) => {
    const updated = structuredClone(manifest);
    delete updated[key];
    setManifest(Object.keys(updated).length > 0 ? updated : undefined);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h4 className="text-xs font-semibold">{title}</h4>
        <Button variant="ghost" size="sm" className="h-6 px-1.5 text-[10px]" onClick={addEntry}>
          + Add
        </Button>
      </div>
      {Object.entries(manifest).map(([key, chains]) => (
        <div key={key} className="rounded border p-2 space-y-1.5">
          <div className="flex items-center justify-between gap-1">
            <Input
              className="h-6 text-xs font-mono font-medium border-none bg-transparent p-0 focus-visible:ring-1"
              value={key}
              onBlur={(e) => renameKey(key, e.target.value.trim())}
              onKeyDown={(e) => {
                if (e.key === "Enter") renameKey(key, (e.target as HTMLInputElement).value.trim());
              }}
            />
            <div className="flex items-center gap-0.5 shrink-0">
              <Button
                variant="ghost"
                size="sm"
                className="h-5 px-1 text-[10px] text-muted-foreground"
                onClick={() => addChain(key)}
              >
                +chain
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-5 w-5 p-0 text-destructive"
                onClick={() => removeEntry(key)}
              >
                <Trash2 className="w-3 h-3" />
              </Button>
            </div>
          </div>
          {Object.entries(chains).map(([chain, addr]) => (
            <div key={chain} className="flex items-center gap-1.5">
              <Input
                className="h-6 text-[10px] text-muted-foreground w-20 shrink-0 border-none bg-transparent p-0 focus-visible:ring-1"
                value={chain}
                onBlur={(e) => renameChain(key, chain, e.target.value.trim())}
                onKeyDown={(e) => {
                  if (e.key === "Enter") renameChain(key, chain, (e.target as HTMLInputElement).value.trim());
                }}
              />
              <Input
                className="h-6 text-[10px] font-mono"
                value={addr}
                onChange={(e) => updateAddress(key, chain, e.target.value)}
                placeholder="0x..."
              />
              <Button
                variant="ghost"
                size="sm"
                className="h-5 w-5 p-0 shrink-0 text-muted-foreground hover:text-destructive"
                onClick={() => removeChain(key, chain)}
              >
                <Trash2 className="w-2.5 h-2.5" />
              </Button>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function EmptyState() {
  const [tokens, setTokens] = useAtom(tokensManifestAtom);
  const [contracts, setContracts] = useAtom(contractsManifestAtom);

  const hasManifests =
    (tokens && Object.keys(tokens).length > 0) ||
    (contracts && Object.keys(contracts).length > 0);

  if (hasManifests) {
    return (
      <div className="h-full overflow-y-auto p-4 space-y-4">
        <p className="text-xs text-muted-foreground">
          Select a node or edge to configure it. Manifests below:
        </p>
        <ManifestEditor title="Token Addresses" manifest={tokens} setManifest={setTokens} />
        <Separator />
        <ManifestEditor title="Contract Addresses" manifest={contracts} setManifest={setContracts} />
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center h-full text-center p-6">
      <p className="text-sm text-muted-foreground">
        Select a node or edge to configure it
      </p>
    </div>
  );
}

function JsonPreview({ data }: { data: unknown }) {
  return (
    <pre className="text-[11px] font-mono bg-muted/50 p-3 rounded-md overflow-auto max-h-96 text-muted-foreground whitespace-pre-wrap">
      {JSON.stringify(data, null, 2)}
    </pre>
  );
}

// ── Main panel ───────────────────────────────────────────────────────

export function NodeConfigPanel() {
  const nodes = useAtomValue(nodesAtom);
  const edges = useAtomValue(edgesAtom);
  const selectedNodeId = useAtomValue(selectedNodeAtom);
  const selectedEdgeId = useAtomValue(selectedEdgeAtom);
  const deleteNode = useSetAtom(deleteNodeAtom);
  const updateNodeData = useSetAtom(updateNodeDataAtom);

  if (selectedEdgeId) {
    const edge = edges.find((e) => e.id === selectedEdgeId);
    if (edge) {
      return (
        <div className="h-full overflow-y-auto p-4">
          <EdgeConfig edge={edge as CanvasEdge} />
        </div>
      );
    }
  }

  if (selectedNodeId) {
    const node = nodes.find((n) => n.id === selectedNodeId);
    if (!node) return <EmptyState />;

    const config = getNodeConfig(node.data.defiNode.type);
    const Icon = config?.icon;

    return (
      <div className="h-full flex flex-col">
        <div className="px-4 py-3 border-b flex items-center gap-2">
          {Icon && <Icon className="w-4 h-4 text-muted-foreground" />}
          <div className="flex-1 min-w-0">
            <Input
              className="h-7 text-sm font-semibold border-none bg-transparent p-0 focus-visible:ring-0"
              value={node.data.label}
              onChange={(e) =>
                updateNodeData({
                  id: node.id,
                  data: { label: e.target.value },
                })
              }
              placeholder="Node label"
            />
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0 text-destructive"
            onClick={() => deleteNode(node.id)}
          >
            <Trash2 className="w-3.5 h-3.5" />
          </Button>
        </div>

        <div className="px-4 py-2 border-b">
          <label className="text-[10px] text-muted-foreground">Node ID</label>
          <Input
            className="h-7 text-xs font-mono border-none bg-transparent p-0 focus-visible:ring-0"
            value={node.data.defiNode.id}
            onChange={(e) => {
              const updatedDefi = { ...node.data.defiNode, id: e.target.value };
              updateNodeData({
                id: node.id,
                data: { defiNode: updatedDefi },
              });
            }}
            placeholder="node_id"
          />
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          <NodeConfigForm node={node as any} />
          <Separator className="my-4" />
          <details className="text-xs">
            <summary className="text-muted-foreground cursor-pointer hover:text-foreground mb-2">
              JSON Preview
            </summary>
            <JsonPreview data={node.data.defiNode} />
          </details>
        </div>
      </div>
    );
  }

  return <EmptyState />;
}
