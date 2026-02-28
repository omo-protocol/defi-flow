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
import {
  isAuthenticatedAtom,
  walletsAtom,
  selectedWalletIdAtom,
  userConfigAtom,
} from "@/lib/auth-store";
import { startRun as startRunAuth } from "@/lib/auth-api";
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
import { Play, Square, Radio, Wallet } from "lucide-react";
import { toast } from "sonner";

export function RunControls({
  onSessionSelect,
}: {
  onSessionSelect?: (id: string | null) => void;
}) {
  const nodes = useAtomValue(nodesAtom);
  const edges = useAtomValue(edgesAtom);
  const name = useAtomValue(workflowNameAtom);
  const tokens = useAtomValue(tokensManifestAtom);
  const contracts = useAtomValue(contractsManifestAtom);

  const isAuth = useAtomValue(isAuthenticatedAtom);
  const wallets = useAtomValue(walletsAtom);
  const savedWalletId = useAtomValue(selectedWalletIdAtom);
  const config = useAtomValue(userConfigAtom);

  const [network, setNetwork] = useState(config.default_network || "testnet");
  const [dryRun, setDryRun] = useState(true);
  const [walletId, setWalletId] = useState<string>("");
  const [starting, setStarting] = useState(false);
  const [sessions, setSessions] = useState<RunListEntry[]>([]);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);
  const [status, setStatus] = useState<RunStatusResponse | null>(null);

  // Pre-select wallet if one is bound to the strategy
  useEffect(() => {
    if (savedWalletId && !walletId) setWalletId(savedWalletId);
  }, [savedWalletId]); // eslint-disable-line react-hooks/exhaustive-deps

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

    if (!dryRun && !walletId) {
      toast.error("Select a wallet for live trading");
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

      let res;
      if (walletId && walletId !== "none" && isAuth) {
        // Auth API — PK decrypted server-side, never leaves backend
        res = await startRunAuth(walletId, workflow, {
          network,
          dry_run: dryRun,
        });
      } else {
        // Dry run without wallet
        res = await startDaemon(workflow, {
          network,
          dry_run: dryRun,
        });
      }
      toast.success(`Daemon started: ${res.session_id.slice(0, 8)}...`);
      setSelectedSession(res.session_id);
      onSessionSelect?.(res.session_id);
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
      setTimeout(() => {
        listRuns().then(setSessions).catch(() => {});
        if (selectedSession === sessionId) {
          setSelectedSession(null);
          onSessionSelect?.(null);
        }
      }, 1000);
    } catch (err) {
      toast.error(
        "Stop failed: " +
          (err instanceof Error ? err.message : "Unknown error")
      );
    }
  };

  const selectedWallet = wallets.find((w) => w.id === walletId);

  return (
    <div className="p-4 space-y-4 text-sm">
      <h3 className="font-semibold text-base">Live Run</h3>

      {/* Start controls */}
      <div className="space-y-3">
        <div>
          <Label className="text-xs text-muted-foreground">Network</Label>
          <Select value={network} onValueChange={setNetwork}>
            <SelectTrigger className="h-8 text-xs mt-1">
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

        {/* Wallet selector */}
        <div>
          <Label className="text-xs text-muted-foreground">
            Wallet {!dryRun && <span className="text-destructive">*</span>}
          </Label>
          {isAuth && wallets.length > 0 ? (
            <Select value={walletId} onValueChange={setWalletId}>
              <SelectTrigger className="h-8 text-xs mt-1">
                <SelectValue placeholder="Select wallet" />
              </SelectTrigger>
              <SelectContent>
                {dryRun && (
                  <SelectItem value="none" className="text-xs">
                    No wallet (dry run only)
                  </SelectItem>
                )}
                {wallets.map((w) => (
                  <SelectItem key={w.id} value={w.id} className="text-xs">
                    <span className="font-medium">{w.label}</span>
                    <span className="ml-2 text-muted-foreground font-mono">
                      {w.address.slice(0, 6)}...{w.address.slice(-4)}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : isAuth ? (
            <p className="text-xs text-muted-foreground mt-1">
              No wallets. Add one via the user menu.
            </p>
          ) : (
            <p className="text-xs text-muted-foreground mt-1">
              Sign in to use saved wallets.
            </p>
          )}
          {selectedWallet && (
            <p className="text-[10px] text-muted-foreground mt-1 font-mono">
              {selectedWallet.address}
            </p>
          )}
        </div>

        <Button
          onClick={handleStart}
          disabled={starting || nodes.length === 0 || (!dryRun && !walletId)}
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
                onClick={() => {
                  setSelectedSession(s.session_id);
                  onSessionSelect?.(s.session_id);
                }}
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
