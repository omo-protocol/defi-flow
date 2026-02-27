"use client";

import { memo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import type { CanvasEdge, CanvasEdgeData } from "@/lib/types/canvas";
import { cn } from "@/lib/utils";

export const DefiEdge = memo(function DefiEdge(props: EdgeProps<CanvasEdge>) {
  const {
    id,
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    selected,
  } = props;

  const data = props.data as CanvasEdgeData | undefined;
  const srcType = data?.sourceType as string | undefined;

  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });

  // Only show label on edges from wallet or optimizer nodes
  const showLabel = srcType === "wallet" || srcType === "optimizer";

  const token = data?.token ?? "?";
  const amount = data?.amount;
  const amountText = !amount
    ? ""
    : amount.type === "all"
      ? "all"
      : amount.type === "percentage"
        ? `${amount.value}%`
        : amount.value;

  const isError = data?.status === "error";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          strokeWidth: selected ? 2.5 : 1.5,
          stroke: isError ? "#ef4444" : selected ? "#818cf8" : "#64748b",
          strokeDasharray: "5 4",
          animation: "dashdraw 0.8s linear infinite",
          opacity: selected ? 1 : 0.6,
          transition: "stroke 0.15s, stroke-width 0.15s, opacity 0.15s",
        }}
      />
      {showLabel && (
        <EdgeLabelRenderer>
          <div
            style={{
              position: "absolute",
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
              pointerEvents: "all",
            }}
            className="nodrag nopan"
          >
            <div
              className={cn(
                "flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border cursor-pointer transition-all shadow-sm",
                selected
                  ? "bg-indigo-500 text-white border-indigo-400 shadow-indigo-500/20"
                  : "bg-card/95 backdrop-blur-sm text-foreground border-border/60 hover:border-border hover:shadow-md"
              )}
            >
              <span className="font-semibold">{token}</span>
              {amountText && (
                <>
                  <span className={selected ? "text-white/50" : "text-muted-foreground"}>·</span>
                  <span className={selected ? "text-white/80" : "text-muted-foreground"}>{amountText}</span>
                </>
              )}
            </div>
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
});
