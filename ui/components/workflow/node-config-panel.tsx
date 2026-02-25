"use client";

import { useAtomValue, useSetAtom, useAtom } from "jotai";
import {
  nodesAtom,
  edgesAtom,
  selectedNodeAtom,
  selectedEdgeAtom,
  deleteNodeAtom,
  deleteEdgeAtom,
  updateNodeDataAtom,
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

  const token = edge.data?.token ?? "";
  const amount = edge.data?.amount ?? { type: "all" as const };

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

      <div className="space-y-1.5">
        <Label className="text-xs">Token</Label>
        <Input
          className="h-8 text-xs"
          value={token}
          onChange={(e) => updateEdge({ token: e.target.value })}
          placeholder="USDC"
        />
      </div>

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
    </div>
  );
}

function EmptyState() {
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
