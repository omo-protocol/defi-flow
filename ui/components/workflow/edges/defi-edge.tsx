"use client";

import { memo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
  type Edge,
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

  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });

  const token = data?.token ?? "?";
  const amount = data?.amount;
  const amountText = !amount
    ? ""
    : amount.type === "all"
      ? "all"
      : amount.type === "percentage"
        ? `${amount.value}%`
        : amount.value;

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          strokeWidth: selected ? 3 : 2,
          stroke: data?.status === "error" ? "#ef4444" : selected ? "#818cf8" : "#6366f1",
          strokeDasharray: "6 3",
          animation: "dashdraw 0.5s linear infinite",
        }}
      />
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
              "flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium border cursor-pointer transition-colors",
              selected
                ? "bg-indigo-500 text-white border-indigo-400"
                : "bg-card text-foreground border-border hover:bg-accent"
            )}
          >
            <span>{token}</span>
            {amountText && (
              <>
                <span className="text-muted-foreground">·</span>
                <span className="text-muted-foreground">{amountText}</span>
              </>
            )}
          </div>
        </div>
      </EdgeLabelRenderer>
    </>
  );
});
