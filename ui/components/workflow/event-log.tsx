"use client";

import { useState, useEffect, useRef } from "react";
import { subscribeEvents } from "@/lib/api";
import { Separator } from "@/components/ui/separator";
import { Button } from "@/components/ui/button";
import { Trash2, Pause, Play } from "lucide-react";

type EngineEvent = {
  type: string;
  [key: string]: unknown;
};

export function EventLog({ sessionId }: { sessionId: string | null }) {
  const [events, setEvents] = useState<EngineEvent[]>([]);
  const [paused, setPaused] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (!sessionId) {
      setEvents([]);
      return;
    }

    const es = subscribeEvents(sessionId);
    eventSourceRef.current = es;

    es.onmessage = (e) => {
      try {
        const event: EngineEvent = JSON.parse(e.data);
        setEvents((prev) => [...prev.slice(-500), event]);
      } catch {
        // ignore malformed events
      }
    };

    es.onerror = () => {
      es.close();
    };

    return () => {
      es.close();
      eventSourceRef.current = null;
    };
  }, [sessionId]);

  // Auto-scroll
  useEffect(() => {
    if (!paused && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [events, paused]);

  if (!sessionId) {
    return (
      <div className="p-4 text-sm text-muted-foreground">
        Select an active session to view live events.
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-4 py-2 border-b">
        <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground">
          Event Log ({events.length})
        </h4>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            onClick={() => setPaused(!paused)}
          >
            {paused ? (
              <Play className="w-3 h-3" />
            ) : (
              <Pause className="w-3 h-3" />
            )}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            onClick={() => setEvents([])}
          >
            <Trash2 className="w-3 h-3" />
          </Button>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto p-2 space-y-0.5 font-mono text-[11px]"
      >
        {events.map((event, i) => (
          <EventLine key={i} event={event} />
        ))}
        {events.length === 0 && (
          <div className="text-muted-foreground text-center py-8">
            Waiting for events...
          </div>
        )}
      </div>
    </div>
  );
}

function EventLine({ event }: { event: EngineEvent }) {
  const colorMap: Record<string, string> = {
    Deployed: "text-blue-400",
    TickCompleted: "text-muted-foreground",
    NodeExecuted: "text-emerald-400",
    Rebalanced: "text-amber-400",
    MarginTopUp: "text-orange-400",
    ReserveAction: "text-purple-400",
    HotReloaded: "text-cyan-400",
    Error: "text-red-400",
    Stopped: "text-red-300",
  };

  const color = colorMap[event.type] || "text-foreground";

  const formatEvent = (e: EngineEvent): string => {
    switch (e.type) {
      case "Deployed":
        return `deployed ${(e.nodes as string[])?.length ?? 0} nodes, TVL=$${(e.tvl as number)?.toFixed(2)}`;
      case "TickCompleted":
        return `tick ts=${e.timestamp} TVL=$${(e.tvl as number)?.toFixed(2)}`;
      case "NodeExecuted":
        return `${e.node_id} → ${e.action} $${(e.amount as number)?.toFixed(2)}`;
      case "Rebalanced":
        return `rebalance ${e.group} drift=${(e.drift as number)?.toFixed(4)}`;
      case "MarginTopUp":
        return `margin ${e.perp_node} +$${(e.amount as number)?.toFixed(2)} from ${e.from_donor}`;
      case "Error":
        return `ERROR${e.node_id ? ` [${e.node_id}]` : ""}: ${e.message}`;
      case "Stopped":
        return `stopped: ${e.reason}`;
      default:
        return JSON.stringify(e);
    }
  };

  return (
    <div className={`${color} leading-tight`}>
      <span className="text-muted-foreground mr-1">[{event.type}]</span>
      {formatEvent(event)}
    </div>
  );
}
