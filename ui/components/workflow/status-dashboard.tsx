"use client";

import { useState } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BacktestPanel } from "./backtest-panel";
import { RunControls } from "./run-controls";
import { EventLog } from "./event-log";
import { checkHealth } from "@/lib/api";
import { useEffect } from "react";

export function StatusDashboard() {
  const [tab, setTab] = useState("backtest");
  const [apiOnline, setApiOnline] = useState<boolean | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  // Check API health on mount
  useEffect(() => {
    checkHealth().then(setApiOnline);
    const interval = setInterval(() => {
      checkHealth().then(setApiOnline);
    }, 10000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="h-full flex flex-col">
      {/* API status indicator */}
      <div className="flex items-center gap-1.5 px-4 py-2 border-b text-xs">
        <div
          className={`w-1.5 h-1.5 rounded-full ${
            apiOnline === true
              ? "bg-emerald-500"
              : apiOnline === false
                ? "bg-red-500"
                : "bg-yellow-500"
          }`}
        />
        <span className="text-muted-foreground">
          API {apiOnline === true ? "Connected" : apiOnline === false ? "Offline" : "Checking..."}
        </span>
        {apiOnline === false && (
          <span className="text-muted-foreground ml-auto">
            Run: defi-flow api -p 8080
          </span>
        )}
      </div>

      <Tabs value={tab} onValueChange={setTab} className="flex-1 flex flex-col">
        <TabsList className="w-full justify-start rounded-none border-b px-4 h-8">
          <TabsTrigger value="backtest" className="text-xs h-6">
            Backtest
          </TabsTrigger>
          <TabsTrigger value="run" className="text-xs h-6">
            Run
          </TabsTrigger>
          <TabsTrigger value="events" className="text-xs h-6">
            Events
          </TabsTrigger>
        </TabsList>

        <TabsContent value="backtest" className="flex-1 overflow-y-auto mt-0">
          <BacktestPanel />
        </TabsContent>

        <TabsContent value="run" className="flex-1 overflow-y-auto mt-0">
          <RunControls />
        </TabsContent>

        <TabsContent value="events" className="flex-1 overflow-hidden mt-0">
          <EventLog sessionId={activeSessionId} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
