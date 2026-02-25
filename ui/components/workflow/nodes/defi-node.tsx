"use client";

import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import type { CanvasNode, CanvasNodeData } from "@/lib/types/canvas";
import type { DefiNode as DefiNodeType } from "@/lib/types/defi-flow";
import { getNodeConfig } from "@/lib/node-registry";
import { cn } from "@/lib/utils";
import { AlertCircle } from "lucide-react";

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
  blue: "border-blue-500/40 bg-blue-500/5",
  purple: "border-purple-500/40 bg-purple-500/5",
  green: "border-green-500/40 bg-green-500/5",
  cyan: "border-cyan-500/40 bg-cyan-500/5",
  indigo: "border-indigo-500/40 bg-indigo-500/5",
  teal: "border-teal-500/40 bg-teal-500/5",
  orange: "border-orange-500/40 bg-orange-500/5",
  pink: "border-pink-500/40 bg-pink-500/5",
  amber: "border-amber-500/40 bg-amber-500/5",
  red: "border-red-500/40 bg-red-500/5",
};

const ICON_COLOR_MAP: Record<string, string> = {
  blue: "text-blue-400",
  purple: "text-purple-400",
  green: "text-green-400",
  cyan: "text-cyan-400",
  indigo: "text-indigo-400",
  teal: "text-teal-400",
  orange: "text-orange-400",
  pink: "text-pink-400",
  amber: "text-amber-400",
  red: "text-red-400",
};

export const DefiNode = memo(function DefiNode(props: NodeProps<CanvasNode>) {
  const data = props.data as CanvasNodeData;
  const selected = props.selected;
  const config = getNodeConfig(data.defiNode.type);
  if (!config) return null;

  const Icon = config.icon;
  const hasErrors = data.validationErrors && data.validationErrors.length > 0;

  return (
    <div
      className={cn(
        "w-60 rounded-lg border-2 shadow-md transition-all",
        COLOR_MAP[config.color] ?? "border-border bg-card",
        selected && "ring-2 ring-primary ring-offset-2 ring-offset-background",
        hasErrors && "!border-red-500",
        data.status === "valid" && "!border-green-500"
      )}
    >
      <Handle type="target" position={Position.Left} className="!w-3 !h-3 !bg-muted-foreground !border-background" />

      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/50">
        <Icon className={cn("w-4 h-4 shrink-0", ICON_COLOR_MAP[config.color])} />
        <span className="text-xs font-semibold truncate">{data.label || config.label}</span>
        <span className="ml-auto text-[10px] text-muted-foreground uppercase tracking-wide">
          {config.label}
        </span>
      </div>

      <div className="px-3 py-2 text-xs text-muted-foreground leading-relaxed">
        <NodeSummary node={data.defiNode} />
      </div>

      {hasErrors && (
        <div className="px-3 pb-2 flex items-start gap-1.5 text-xs text-red-400">
          <AlertCircle className="w-3 h-3 mt-0.5 shrink-0" />
          <span className="line-clamp-2">{data.validationErrors![0]}</span>
        </div>
      )}

      {"trigger" in data.defiNode && data.defiNode.trigger && (
        <div className="px-3 pb-2">
          <span className="inline-block text-[10px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">
            {data.defiNode.trigger.type === "cron" ? data.defiNode.trigger.interval : "event"}
          </span>
        </div>
      )}

      <Handle type="source" position={Position.Right} className="!w-3 !h-3 !bg-muted-foreground !border-background" />
    </div>
  );
});
