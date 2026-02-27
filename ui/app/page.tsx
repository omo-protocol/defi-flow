"use client";

import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { atom } from "jotai";
import {
  rightPanelWidthAtom,
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  selectedNodeAtom,
  selectedEdgeAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import {
  authUserAtom,
  authLoadingAtom,
  isAuthenticatedAtom,
  selectedWalletIdAtom,
  tokenAtom,
  setTokenGetter,
  type StrategyInfo,
} from "@/lib/auth-store";
import {
  register,
  login,
  listStrategies,
  getStrategy,
} from "@/lib/auth-api";
import { WorkflowCanvas } from "@/components/workflow/workflow-canvas";
import { NodeConfigPanel } from "@/components/workflow/node-config-panel";
import { StatusDashboard } from "@/components/workflow/status-dashboard";
import { AgentPanel } from "@/components/ai-agent/agent-panel";
import { useEffect, useState } from "react";
import { convertDefiFlowToCanvas } from "@/lib/converters/canvas-defi-flow";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";
import { useReactFlow } from "@xyflow/react";
import { toast } from "sonner";

// Right panel mode: "config" when a node/edge is selected, "engine" for backtest/run, "agent" for AI builder
export const panelModeAtom = atom<"config" | "engine" | "agent">("config");

const EXAMPLES = [
  { name: "Delta Neutral v1", file: "delta_neutral.json" },
  { name: "Delta Neutral v2", file: "delta_neutral_v2.json" },
];

function WelcomeOverlay({ onClose }: { onClose: () => void }) {
  const setNodes = useSetAtom(nodesAtom);
  const setEdges = useSetAtom(edgesAtom);
  const setName = useSetAtom(workflowNameAtom);
  const setTokens = useSetAtom(tokensManifestAtom);
  const setContracts = useSetAtom(contractsManifestAtom);
  const { fitView } = useReactFlow();

  const authLoading = useAtomValue(authLoadingAtom);
  const isAuth = useAtomValue(isAuthenticatedAtom);
  const user = useAtomValue(authUserAtom);
  const setSelectedWalletId = useSetAtom(selectedWalletIdAtom);

  const [tab, setTab] = useState<"login" | "register">("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [authError, setAuthError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [savedStrategies, setSavedStrategies] = useState<StrategyInfo[]>([]);

  // Load saved strategies when authenticated
  useEffect(() => {
    if (isAuth) {
      listStrategies().then(setSavedStrategies).catch(() => {});
    }
  }, [isAuth]);

  const setUser = useSetAtom(authUserAtom);
  const setToken = useSetAtom(tokenAtom);
  const setAuthLoading = useSetAtom(authLoadingAtom);

  const handleAuth = async (e: React.FormEvent) => {
    e.preventDefault();
    setAuthError("");
    setSubmitting(true);
    try {
      if (tab === "register") {
        if (password !== confirmPassword) {
          setAuthError("Passwords do not match");
          setSubmitting(false);
          return;
        }
        await register(username, password);
      }

      const result = await login(username, password);
      setToken(result.token);
      setUser(result.user);
      toast.success(tab === "register" ? `Welcome, ${username}!` : `Welcome back, ${username}!`);
    } catch (err) {
      setAuthError(err instanceof Error ? err.message : "Authentication failed");
    } finally {
      setSubmitting(false);
    }
  };

  const loadExample = async (file: string) => {
    try {
      const res = await fetch(`/examples/${file}`);
      const workflow: DefiFlowWorkflow = await res.json();
      const { nodes, edges, tokens, contracts } = convertDefiFlowToCanvas(workflow);
      setNodes(nodes);
      setEdges(edges);
      setName(workflow.name);
      setTokens(tokens);
      setContracts(contracts);
      onClose();
      setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 200);
    } catch {
      onClose();
    }
  };

  const loadStrategy = async (id: string) => {
    try {
      const strat = await getStrategy(id);
      const workflow: DefiFlowWorkflow = JSON.parse(strat.workflow_json);
      const { nodes, edges, tokens, contracts } = convertDefiFlowToCanvas(workflow);
      setNodes(nodes);
      setEdges(edges);
      setName(workflow.name || strat.name);
      setTokens(tokens);
      setContracts(contracts);
      if (strat.wallet_id) setSelectedWalletId(strat.wallet_id);
      onClose();
      toast.success(`Loaded "${strat.name}"`);
      setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 200);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to load");
    }
  };

  if (authLoading) {
    return (
      <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-md">
        <div className="flex flex-col items-center gap-3">
          <div className="w-8 h-8 border-2 border-primary/30 border-t-primary rounded-full animate-spin" />
          <span className="text-sm text-muted-foreground">Loading...</span>
        </div>
      </div>
    );
  }

  // Not authenticated — show login/register
  if (!isAuth) {
    return (
      <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-md">
        <div className="bg-card border rounded-2xl shadow-2xl p-8 max-w-sm w-full mx-4">
          <div className="flex items-center gap-2 mb-1">
            <div className="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="text-primary"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
            </div>
            <h1 className="text-lg font-bold tracking-tight">DeFi Flow</h1>
          </div>
          <p className="text-sm text-muted-foreground mb-6">
            Sign in to build and manage your strategies.
          </p>

          {/* Tabs */}
          <div className="flex mb-4 border rounded-lg overflow-hidden">
            <button
              onClick={() => { setTab("login"); setAuthError(""); }}
              className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
                tab === "login"
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              Sign In
            </button>
            <button
              onClick={() => { setTab("register"); setAuthError(""); }}
              className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors ${
                tab === "register"
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              Register
            </button>
          </div>

          <form onSubmit={handleAuth} className="space-y-3">
            <div>
              <label className="text-xs text-muted-foreground mb-1 block">Username</label>
              <input
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                className="w-full h-9 px-3 text-sm rounded-lg border bg-background focus:outline-none focus:ring-2 focus:ring-primary/30 transition-shadow"
                required
              />
            </div>
            <div>
              <label className="text-xs text-muted-foreground mb-1 block">Password</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete={tab === "register" ? "new-password" : "current-password"}
                className="w-full h-9 px-3 text-sm rounded-lg border bg-background focus:outline-none focus:ring-2 focus:ring-primary/30 transition-shadow"
                required
              />
            </div>
            {tab === "register" && (
              <div>
                <label className="text-xs text-muted-foreground mb-1 block">Confirm Password</label>
                <input
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  autoComplete="new-password"
                  className="w-full h-9 px-3 text-sm rounded-lg border bg-background focus:outline-none focus:ring-2 focus:ring-primary/30 transition-shadow"
                  required
                />
              </div>
            )}
            {authError && (
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-destructive/10 text-destructive text-xs">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
                {authError}
              </div>
            )}
            <button
              type="submit"
              disabled={submitting}
              className="w-full px-4 py-2.5 rounded-lg bg-primary text-primary-foreground font-medium text-sm hover:bg-primary/90 transition-all disabled:opacity-50"
            >
              {submitting ? (
                <span className="flex items-center justify-center gap-2">
                  <span className="w-3.5 h-3.5 border-2 border-primary-foreground/30 border-t-primary-foreground rounded-full animate-spin" />
                  {tab === "register" ? "Creating..." : "Signing in..."}
                </span>
              ) : tab === "register" ? "Create Account" : "Sign In"}
            </button>
          </form>
        </div>
      </div>
    );
  }

  // Authenticated — show strategies + examples
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-md">
      <div className="bg-card border rounded-2xl shadow-2xl p-8 max-w-md w-full mx-4">
        <h1 className="text-lg font-bold mb-1">Welcome back, {user?.username}</h1>
        <p className="text-sm text-muted-foreground mb-6">
          Load a strategy or start fresh.
        </p>

        {/* Saved strategies */}
        {savedStrategies.length > 0 && (
          <div className="space-y-2 mb-6">
            <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
              Your Strategies
            </p>
            {savedStrategies.map((s) => (
              <button
                key={s.id}
                onClick={() => loadStrategy(s.id)}
                className="w-full text-left px-4 py-3 rounded-lg border hover:bg-accent transition-colors"
              >
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium">{s.name}</span>
                  <span className="text-[10px] text-muted-foreground">
                    {new Date(s.updated_at * 1000).toLocaleDateString()}
                  </span>
                </div>
                {(s.wallet_label || s.wallet_address) && (
                  <div className="text-[10px] text-muted-foreground mt-0.5 font-mono">
                    {s.wallet_label && <span>{s.wallet_label} </span>}
                    {s.wallet_address && (
                      <span>{s.wallet_address.slice(0, 6)}...{s.wallet_address.slice(-4)}</span>
                    )}
                  </div>
                )}
              </button>
            ))}
          </div>
        )}

        {/* Examples */}
        <div className="space-y-2 mb-6">
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">
            Examples
          </p>
          {EXAMPLES.map((ex) => (
            <button
              key={ex.file}
              onClick={() => loadExample(ex.file)}
              className="w-full text-left px-4 py-3 rounded-lg border hover:bg-accent transition-colors text-sm"
            >
              {ex.name}
            </button>
          ))}
        </div>

        <button
          onClick={onClose}
          className="w-full px-4 py-2.5 rounded-lg bg-primary text-primary-foreground font-medium text-sm hover:bg-primary/90 transition-all"
        >
          Start from Scratch
        </button>

        <p className="text-[10px] text-muted-foreground/50 text-center mt-4">
          Cmd+S to save · Cmd+Z to undo · Delete to remove
        </p>
      </div>
    </div>
  );
}

function RightPanel() {
  const [panelMode, setPanelMode] = useAtom(panelModeAtom);
  const selectedNodeId = useAtomValue(selectedNodeAtom);
  const selectedEdgeId = useAtomValue(selectedEdgeAtom);

  // Auto-switch to config when a node/edge is selected
  useEffect(() => {
    if (selectedNodeId || selectedEdgeId) {
      setPanelMode("config");
    }
  }, [selectedNodeId, selectedEdgeId, setPanelMode]);

  return (
    <div className="h-full flex flex-col">
      {/* Tab bar */}
      <div className="flex border-b bg-card px-1 pt-1">
        {(["config", "engine", "agent"] as const).map((mode) => (
          <button
            key={mode}
            onClick={() => setPanelMode(mode)}
            className={`flex-1 px-3 py-1.5 text-xs font-medium transition-all rounded-t-md ${
              panelMode === mode
                ? "text-foreground bg-background border border-b-0 border-border -mb-px"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            {mode === "config" ? "Config" : mode === "engine" ? "Engine" : "Agent"}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {panelMode === "config" ? (
          <NodeConfigPanel />
        ) : panelMode === "engine" ? (
          <StatusDashboard />
        ) : (
          <AgentPanel />
        )}
      </div>
    </div>
  );
}

export default function Home() {
  const rightPanelWidth = useAtomValue(rightPanelWidthAtom);
  const [nodes, setNodes] = useAtom(nodesAtom);
  const setEdges = useSetAtom(edgesAtom);
  const setName = useSetAtom(workflowNameAtom);
  const setTokensManifest = useSetAtom(tokensManifestAtom);
  const setContractsManifest = useSetAtom(contractsManifestAtom);
  const isAuth = useAtomValue(isAuthenticatedAtom);
  const [authLoading, setAuthLoading] = useAtom(authLoadingAtom);
  const token = useAtomValue(tokenAtom);
  const [showWelcome, setShowWelcome] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Wire up token getter for auth-api and mark loading done
  useEffect(() => {
    setTokenGetter(() => token);
  }, [token]);

  useEffect(() => {
    setAuthLoading(false);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Restore from localStorage on mount (only when authenticated)
  useEffect(() => {
    if (authLoading) return;
    if (!isAuth) {
      setShowWelcome(true);
      setLoaded(true);
      return;
    }
    try {
      const saved = localStorage.getItem("defi-flow-current");
      if (saved) {
        const { name, nodes: n, edges: e, tokens, contracts } = JSON.parse(saved);
        if (n && n.length > 0) {
          setNodes(n);
          setEdges(e);
          if (name) setName(name);
          if (tokens) setTokensManifest(tokens);
          if (contracts) setContractsManifest(contracts);
          setLoaded(true);
          return;
        }
      }
    } catch {
      // corrupt data — ignore
    }
    // No saved data — show welcome
    setShowWelcome(true);
    setLoaded(true);
  }, [authLoading, isAuth]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!loaded) return null;

  return (
    <div className="h-dvh w-full flex">
      {/* Canvas */}
      <div className="flex-1 relative">
        <WorkflowCanvas />
      </div>

      {/* Right panel — Config / Engine tabs */}
      {rightPanelWidth && (
        <div
          className="border-l bg-card h-full overflow-hidden"
          style={{ width: rightPanelWidth }}
        >
          <RightPanel />
        </div>
      )}

      {/* Welcome overlay — shown on first visit OR when not authenticated */}
      {(showWelcome || !isAuth) && (
        <WelcomeOverlay onClose={() => setShowWelcome(false)} />
      )}
    </div>
  );
}
