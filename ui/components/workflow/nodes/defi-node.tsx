"use client";

import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { CanvasNode, CanvasNodeData } from "@/lib/types/canvas";
import type { DefiNode as DefiNodeType } from "@/lib/types/defi-flow";
import { getNodeConfig } from "@/lib/node-registry";
import { cn } from "@/lib/utils";
import { AlertCircle, CheckCircle2 } from "lucide-react";

function NodeSummary({ node }: { node: DefiNodeType }) {
  switch (node.type) {
    case "wallet":
      return <>{node.token} on {node.chain.name}</>;
    case "perp": {
      const dir = node.direction ? ` ${node.direction}` : "";
      const lev = node.leverage ? ` ${node.leverage}x` : "";
      return <>{node.venue} · {node.action}{dir} {node.pair}{lev}</>;
    }
    case "spot":
      return <>{node.venue} · {node.side} {node.pair}</>;
    case "lending":
      return <>{node.archetype} · {node.action} {node.asset}</>;
    case "vault":
      return <>{node.archetype} · {node.action} {node.asset}</>;
    case "lp":
      return <>{node.venue} · {node.action} {node.pool}</>;
    case "options":
      return <>{node.venue} · {node.action} {node.asset}</>;
    case "pendle":
      return <>{node.action} · {node.market}</>;
    case "movement":
      return <>{node.movement_type} · {node.from_token}→{node.to_token}</>;
    case "optimizer":
      return <>Kelly {(node.kelly_fraction * 100).toFixed(0)}% · {node.allocations.length} venues</>;
  }
}

const COLOR_MAP: Record<string, string> = {
  blue: "border-blue-500/30 bg-blue-500/5 hover:border-blue-500/50 hover:bg-blue-500/10",
  purple: "border-purple-500/30 bg-purple-500/5 hover:border-purple-500/50 hover:bg-purple-500/10",
  green: "border-green-500/30 bg-green-500/5 hover:border-green-500/50 hover:bg-green-500/10",
  cyan: "border-cyan-500/30 bg-cyan-500/5 hover:border-cyan-500/50 hover:bg-cyan-500/10",
  indigo: "border-indigo-500/30 bg-indigo-500/5 hover:border-indigo-500/50 hover:bg-indigo-500/10",
  teal: "border-teal-500/30 bg-teal-500/5 hover:border-teal-500/50 hover:bg-teal-500/10",
  orange: "border-orange-500/30 bg-orange-500/5 hover:border-orange-500/50 hover:bg-orange-500/10",
  pink: "border-pink-500/30 bg-pink-500/5 hover:border-pink-500/50 hover:bg-pink-500/10",
  amber: "border-amber-500/30 bg-amber-500/5 hover:border-amber-500/50 hover:bg-amber-500/10",
  red: "border-red-500/30 bg-red-500/5 hover:border-red-500/50 hover:bg-red-500/10",
};

const ICON_BG_MAP: Record<string, string> = {
  blue: "bg-blue-500/10 text-blue-400",
  purple: "bg-purple-500/10 text-purple-400",
  green: "bg-green-500/10 text-green-400",
  cyan: "bg-cyan-500/10 text-cyan-400",
  indigo: "bg-indigo-500/10 text-indigo-400",
  teal: "bg-teal-500/10 text-teal-400",
  orange: "bg-orange-500/10 text-orange-400",
  pink: "bg-pink-500/10 text-pink-400",
  amber: "bg-amber-500/10 text-amber-400",
  red: "bg-red-500/10 text-red-400",
};

export const DefiNode = memo(function DefiNode(props: NodeProps<CanvasNode>) {
  const data = props.data as CanvasNodeData;
  const selected = props.selected;
  const config = getNodeConfig(data.defiNode.type);
  if (!config) return null;

  const Icon = config.icon;
  const hasErrors = data.validationErrors && data.validationErrors.length > 0;
  const isValid = data.status === "valid";

  return (
    <div
      className={cn(
        "w-56 rounded-xl border shadow-sm transition-all duration-150",
        COLOR_MAP[config.color] ?? "border-border bg-card hover:bg-accent/50",
        selected && "ring-2 ring-primary/70 ring-offset-1 ring-offset-background shadow-md",
        hasErrors && "!border-red-500/60 !shadow-red-500/10",
        isValid && !hasErrors && "!border-emerald-500/40",
      )}
    >
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2.5 !h-2.5 !bg-muted-foreground/60 !border-2 !border-background hover:!bg-primary !transition-colors"
      />

      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2">
        <div className={cn("w-6 h-6 rounded-md flex items-center justify-center shrink-0", ICON_BG_MAP[config.color])}>
          <Icon className="w-3.5 h-3.5" />
        </div>
        <div className="flex-1 min-w-0">
          <span className="text-xs font-semibold truncate block">{data.label || config.label}</span>
          <span className="text-[10px] text-muted-foreground leading-none">{config.label}</span>
        </div>
        {/* Status indicator */}
        {hasErrors && <AlertCircle className="w-3.5 h-3.5 text-red-400 shrink-0" />}
        {isValid && !hasErrors && <CheckCircle2 className="w-3.5 h-3.5 text-emerald-500 shrink-0" />}
      </div>

      {/* Summary */}
      <div className="px-3 pb-2 text-[11px] text-muted-foreground leading-relaxed">
        <NodeSummary node={data.defiNode} />
      </div>

      {/* Error detail */}
      {hasErrors && (
        <div className="mx-2 mb-2 px-2 py-1.5 rounded-md bg-red-500/10 text-[10px] text-red-400 leading-tight">
          {data.validationErrors![0]}
        </div>
      )}

      {/* Trigger badge */}
      {"trigger" in data.defiNode && data.defiNode.trigger && (
        <div className="px-3 pb-2">
          <span className="inline-flex items-center text-[10px] px-1.5 py-0.5 rounded-md bg-amber-500/10 text-amber-400 border border-amber-500/20 font-medium">
            {data.defiNode.trigger.type === "cron" ? data.defiNode.trigger.interval : "event"}
          </span>
        </div>
      )}

      <Handle
        type="source"
        position={Position.Right}
        className="!w-2.5 !h-2.5 !bg-muted-foreground/60 !border-2 !border-background hover:!bg-primary !transition-colors"
      />
    </div>
  );
});
