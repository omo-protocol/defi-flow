"use client";

import { useState, useEffect } from "react";
import { useAtomValue } from "jotai";
import {
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import { convertCanvasToDefiFlow } from "@/lib/converters/canvas-defi-flow";
import {
  startDaemon,
  stopDaemon,
  listRuns,
  getRunStatus,
  type RunListEntry,
  type RunStatusResponse,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Play, Square, RefreshCw, Radio } from "lucide-react";
import { toast } from "sonner";

export function RunControls() {
  const nodes = useAtomValue(nodesAtom);
  const edges = useAtomValue(edgesAtom);
  const name = useAtomValue(workflowNameAtom);
  const tokens = useAtomValue(tokensManifestAtom);
  const contracts = useAtomValue(contractsManifestAtom);

  const [network, setNetwork] = useState("testnet");
  const [dryRun, setDryRun] = useState(true);
  const [starting, setStarting] = useState(false);
  const [sessions, setSessions] = useState<RunListEntry[]>([]);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [status, setStatus] = useState<RunStatusResponse | null>(null);

  // Poll active runs
  useEffect(() => {
    const poll = () => {
      listRuns().then(setSessions).catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 5000);
    return () => clearInterval(interval);
  }, []);

  // Poll selected session status
  useEffect(() => {
    if (!selectedSession) {
      setStatus(null);
      return;
    }
    const poll = () => {
      getRunStatus(selectedSession).then(setStatus).catch(() => setStatus(null));
    };
    poll();
    const interval = setInterval(poll, 3000);
    return () => clearInterval(interval);
  }, [selectedSession]);

  const handleStart = async () => {
    if (nodes.length === 0) {
      toast.error("No nodes to run");
      return;
    }

    setStarting(true);
    try {
      const workflow = convertCanvasToDefiFlow(
        nodes,
        edges,
        name,
        undefined,
        tokens,
        contracts
      );
      const res = await startDaemon(workflow, {
        network,
        dry_run: dryRun,
      });
      toast.success(`Daemon started: ${res.session_id.slice(0, 8)}...`);
      setSelectedSession(res.session_id);
      // Refresh list
      listRuns().then(setSessions).catch(() => {});
    } catch (err) {
      toast.error(
        "Start failed: " +
          (err instanceof Error ? err.message : "Unknown error")
      );
    } finally {
      setStarting(false);
    }
  };

  const handleStop = async (sessionId: string) => {
    try {
      await stopDaemon(sessionId);
      toast.success("Stopping daemon...");
      // Refresh list after a moment
      setTimeout(() => {
        listRuns().then(setSessions).catch(() => {});
        if (selectedSession === sessionId) {
          setSelectedSession(null);
        }
      }, 1000);
    } catch (err) {
      toast.error(
        "Stop failed: " +
          (err instanceof Error ? err.message : "Unknown error")
      );
    }
  };

  return (
    <div className="p-4 space-y-4 text-sm">
      <h3 className="font-semibold text-base">Live Run</h3>

      {/* Start controls */}
      <div className="space-y-2">
        <div>
          <Label className="text-xs text-muted-foreground">Network</Label>
          <Select value={network} onValueChange={setNetwork}>
            <SelectTrigger className="h-7 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="testnet">Testnet</SelectItem>
              <SelectItem value="mainnet">Mainnet</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="flex items-center space-x-2">
          <Checkbox
            id="dry-run"
            checked={dryRun}
            onCheckedChange={(v) => setDryRun(v === true)}
          />
          <Label htmlFor="dry-run" className="text-xs">
            Dry run (paper trading)
          </Label>
        </div>

        <Button
          onClick={handleStart}
          disabled={starting || nodes.length === 0}
          size="sm"
          className="w-full"
        >
          <Play className="w-3.5 h-3.5 mr-1" />
          {starting ? "Starting..." : "Start Daemon"}
        </Button>
      </div>

      {/* Active sessions */}
      {sessions.length > 0 && (
        <>
          <Separator />
          <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground flex items-center gap-1">
            <Radio className="w-3 h-3 text-emerald-500 animate-pulse" />
            Active Sessions ({sessions.length})
          </h4>
          <div className="space-y-1">
            {sessions.map((s) => (
              <div
                key={s.session_id}
                className={`flex items-center justify-between px-2 py-1.5 rounded text-xs cursor-pointer transition-colors ${
                  selectedSession === s.session_id
                    ? "bg-accent"
                    : "hover:bg-accent/50"
                }`}
                onClick={() => setSelectedSession(s.session_id)}
              >
                <div className="flex-1 min-w-0">
                  <div className="font-medium truncate">
                    {s.workflow_name}
                  </div>
                  <div className="text-muted-foreground">
                    {s.network} &middot; {s.session_id.slice(0, 8)}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0 ml-2"
                  onClick={(e) => {
                    e.stopPropagation();
                    handleStop(s.session_id);
                  }}
                >
                  <Square className="w-3 h-3 text-red-500" />
                </Button>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Selected session status */}
      {status && (
        <>
          <Separator />
          <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground">
            Status
          </h4>
          <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
            <div className="flex justify-between">
              <span className="text-muted-foreground">TVL</span>
              <span className="font-mono">${status.tvl.toFixed(2)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Network</span>
              <span>{status.network}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Dry Run</span>
              <span>{status.dry_run ? "Yes" : "No"}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Status</span>
              <span className="text-emerald-500">{status.status}</span>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
