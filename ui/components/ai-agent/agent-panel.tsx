"use client";

import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useCallback, useEffect, useRef, useState } from "react";
import { nanoid } from "nanoid";
import { toast } from "sonner";
import { Bot, Send, Square, Wrench, Search, CheckCircle, Play, CircleStop, Database, Trash2, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  openaiKeyAtom,
  openaiBaseUrlAtom,
  openaiModelAtom,
  messagesAtom,
  generatingAtom,
  type Message,
} from "@/lib/ai-agent/store";
import { agentLoop, type ToolHandlers } from "@/lib/ai-agent/client";
import { buildSystemPrompt } from "@/lib/ai-agent/prompts";
import {
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  tokensManifestAtom,
  contractsManifestAtom,
  addNodeAtom,
  autosaveAtom,
} from "@/lib/workflow-store";
import {
  getNodeLabel,
  inferEdgeToken,
  type DefiNode,
} from "@/lib/types/defi-flow";
import { convertCanvasToDefiFlow } from "@/lib/converters/canvas-defi-flow";
import type { CanvasNode, CanvasEdge } from "@/lib/types/canvas";
import { validateWorkflow } from "@/lib/wasm";
import {
  runBacktest,
  fetchData,
  startDaemon,
  stopDaemon,
  listRuns,
  getRunStatus,
  getDataManifest,
} from "@/lib/api";

// ── Think-tag parser ────────────────────────────────────────────────

/** Split raw streamed text into { thinking, content } handling partial tags */
function parseThinkContent(raw: string): { thinking: string; content: string } {
  let thinking = "";
  let content = "";
  let remaining = raw;

  while (remaining.length > 0) {
    const openIdx = remaining.indexOf("<think>");
    if (openIdx === -1) {
      // No more think tags — rest is content
      content += remaining;
      break;
    }

    // Everything before <think> is content
    content += remaining.slice(0, openIdx);
    remaining = remaining.slice(openIdx + 7); // skip "<think>"

    const closeIdx = remaining.indexOf("</think>");
    if (closeIdx === -1) {
      // Unclosed think tag — everything remaining is thinking (still streaming)
      thinking += remaining;
      break;
    }

    thinking += remaining.slice(0, closeIdx);
    remaining = remaining.slice(closeIdx + 8); // skip "</think>"
  }

  return { thinking: thinking.trim(), content: content.trim() };
}

// ── Tool activity log for inline display ─────────────────────────────

type ToolActivity = {
  id: string;
  name: string;
  args: string;
  status: "running" | "done" | "error";
};

// ── Main Component ───────────────────────────────────────────────────

export function AgentPanel() {
  const [apiKey, setApiKey] = useAtom(openaiKeyAtom);
  const [baseUrl, setBaseUrl] = useAtom(openaiBaseUrlAtom);
  const [model, setModel] = useAtom(openaiModelAtom);
  const [messages, setMessages] = useAtom(messagesAtom);
  const [generating, setGenerating] = useAtom(generatingAtom);

  const [nodes, setNodes] = useAtom(nodesAtom);
  const [edges, setEdges] = useAtom(edgesAtom);
  const [wfName, setWfName] = useAtom(workflowNameAtom);
  const [tokensManifest, setTokensManifest] = useAtom(tokensManifestAtom);
  const [contractsManifest, setContractsManifest] = useAtom(contractsManifestAtom);
  const addNode = useSetAtom(addNodeAtom);
  const triggerAutosave = useSetAtom(autosaveAtom);

  const [input, setInput] = useState("");
  const [toolActivities, setToolActivities] = useState<ToolActivity[]>([]);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const abortRef = useRef<AbortController | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, toolActivities]);

  // ── Tool Handlers ──────────────────────────────────────────────────

  const buildHandlers = useCallback((): ToolHandlers => {
    // Use refs to get latest state inside callbacks
    const getNodes = () => nodes;
    const getEdges = () => edges;

    return {
      add_node: (nodeData: unknown) => {
        const node = nodeData as DefiNode;
        if (!node.type || !node.id) return "Error: node must have type and id";

        const SPACING_X = 320;
        const SPACING_Y = 180;
        const COLS = 4;
        const currentNodes = getNodes();
        const idx = currentNodes.length;

        const canvasNode: CanvasNode = {
          id: node.id,
          type: "defi-node",
          position: {
            x: (idx % COLS) * SPACING_X + Math.random() * 40,
            y: Math.floor(idx / COLS) * SPACING_Y + Math.random() * 40,
          },
          data: {
            defiNode: node,
            label: getNodeLabel(node),
            status: "idle",
          },
        };

        addNode(canvasNode);
        return `Added node "${node.id}" (${node.type})`;
      },

      remove_node: (nodeId: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const exists = currentNodes.find((n) => n.id === nodeId);
        if (!exists) return `Error: node "${nodeId}" not found`;

        setNodes(currentNodes.filter((n) => n.id !== nodeId));
        setEdges(
          currentEdges.filter(
            (e) => e.source !== nodeId && e.target !== nodeId,
          ),
        );
        return `Removed node "${nodeId}" and its edges`;
      },

      update_node: (nodeId: string, fields: Record<string, unknown>) => {
        const currentNodes = getNodes();
        const idx = currentNodes.findIndex((n) => n.id === nodeId);
        if (idx === -1) return `Error: node "${nodeId}" not found`;

        const existing = currentNodes[idx];
        const updated = {
          ...existing,
          data: {
            ...existing.data,
            defiNode: { ...existing.data.defiNode, ...fields },
            label: getNodeLabel({ ...existing.data.defiNode, ...fields } as DefiNode),
          },
        };
        const newNodes = [...currentNodes];
        newNodes[idx] = updated;
        setNodes(newNodes);
        return `Updated node "${nodeId}": ${Object.keys(fields).join(", ")}`;
      },

      add_edge: (fromNode: string, toNode: string, token?: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const src = currentNodes.find((n) => n.id === fromNode);
        const tgt = currentNodes.find((n) => n.id === toNode);
        if (!src) return `Error: source node "${fromNode}" not found`;
        if (!tgt) return `Error: target node "${toNode}" not found`;

        // Check for duplicate
        const exists = currentEdges.find(
          (e) => e.source === fromNode && e.target === toNode,
        );
        if (exists) return `Edge ${fromNode} → ${toNode} already exists`;

        const edgeToken =
          token ?? inferEdgeToken(src.data.defiNode, tgt.data.defiNode);

        const newEdge: CanvasEdge = {
          id: nanoid(),
          source: fromNode,
          target: toNode,
          type: "defi-edge",
          data: {
            token: edgeToken,
            amount: { type: "all" },
            sourceType: src.data.defiNode.type,
          },
        };
        setEdges([...currentEdges, newEdge]);
        return `Added edge ${fromNode} → ${toNode} (${edgeToken})`;
      },

      remove_edge: (fromNode: string, toNode: string) => {
        const currentEdges = getEdges();
        const filtered = currentEdges.filter(
          (e) => !(e.source === fromNode && e.target === toNode),
        );
        if (filtered.length === currentEdges.length)
          return `Error: no edge ${fromNode} → ${toNode} found`;
        setEdges(filtered);
        return `Removed edge ${fromNode} → ${toNode}`;
      },

      set_manifest: (
        type: "tokens" | "contracts",
        key: string,
        chain: string,
        address: string,
      ) => {
        if (type === "tokens") {
          setTokensManifest((prev) => {
            const m = prev ? structuredClone(prev) : {};
            if (!m[key]) m[key] = {};
            m[key][chain] = address;
            return m;
          });
        } else {
          setContractsManifest((prev) => {
            const m = prev ? structuredClone(prev) : {};
            if (!m[key]) m[key] = {};
            m[key][chain] = address;
            return m;
          });
        }
        return `Set ${type}.${key}.${chain} = ${address}`;
      },

      set_name: (name: string) => {
        setWfName(name);
        return `Strategy name set to "${name}"`;
      },

      validate: async () => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const workflow = convertCanvasToDefiFlow(
          currentNodes,
          currentEdges,
          wfName,
          undefined,
          tokensManifest ?? undefined,
          contractsManifest ?? undefined,
        );
        const json = JSON.stringify(workflow);
        const result = await validateWorkflow(json);
        if (result.valid) return "Validation passed. Strategy is valid.";
        return `Validation failed with ${(result.errors ?? []).length} error(s):\n${(result.errors ?? []).join("\n")}`;
      },

      backtest: async (capital?: number, monteCarlo?: number) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const workflow = convertCanvasToDefiFlow(
          currentNodes,
          currentEdges,
          wfName,
          undefined,
          tokensManifest ?? undefined,
          contractsManifest ?? undefined,
        );
        try {
          const res = await runBacktest(workflow, {
            capital: capital ?? 10000,
            auto_fetch: true,
            monte_carlo: monteCarlo,
          });
          const r = res.result;
          let output = `Backtest complete (historical):
- TWRR: ${r.twrr_pct.toFixed(2)}%
- Annualized: ${r.annualized_pct.toFixed(2)}%
- Sharpe: ${r.sharpe.toFixed(3)}
- Max Drawdown: ${r.max_drawdown_pct.toFixed(2)}%
- Net PnL: $${r.net_pnl.toFixed(2)}
- Rebalances: ${r.rebalances}
- Ticks: ${r.ticks}`;

          if (res.monte_carlo) {
            const sims = res.monte_carlo.simulations;
            const sorted = (key: keyof typeof r) =>
              [...sims].sort((a, b) => (a[key] as number) - (b[key] as number));
            const pct = (arr: typeof sims, p: number, key: keyof typeof r) => {
              const idx = Math.min(Math.floor(p * arr.length), arr.length - 1);
              return (arr[idx][key] as number).toFixed(2);
            };
            const byTwrr = sorted("twrr_pct");
            const bySharpe = sorted("sharpe");
            const byDd = sorted("max_drawdown_pct");
            output += `\n\nMonte Carlo (${sims.length} simulations):
  TWRR:  p5=${pct(byTwrr, 0.05, "twrr_pct")}%  p25=${pct(byTwrr, 0.25, "twrr_pct")}%  p50=${pct(byTwrr, 0.5, "twrr_pct")}%  p75=${pct(byTwrr, 0.75, "twrr_pct")}%  p95=${pct(byTwrr, 0.95, "twrr_pct")}%
  Sharpe: p5=${pct(bySharpe, 0.05, "sharpe")}  p50=${pct(bySharpe, 0.5, "sharpe")}  p95=${pct(bySharpe, 0.95, "sharpe")}
  MaxDD:  p5=${pct(byDd, 0.05, "max_drawdown_pct")}%  p50=${pct(byDd, 0.5, "max_drawdown_pct")}%  p95=${pct(byDd, 0.95, "max_drawdown_pct")}%`;
          }
          return output;
        } catch (err) {
          return `Backtest failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      web_search: async (query: string) => {
        // Use a simple web search proxy — try DuckDuckGo instant answers
        try {
          const url = `https://api.duckduckgo.com/?q=${encodeURIComponent(query)}&format=json&no_html=1`;
          const res = await fetch(url);
          const data = await res.json();
          const results: string[] = [];
          if (data.AbstractText) results.push(data.AbstractText);
          if (data.RelatedTopics) {
            for (const t of data.RelatedTopics.slice(0, 5)) {
              if (t.Text) results.push(t.Text);
            }
          }
          if (results.length === 0)
            return `No results found for "${query}". Try a more specific query, or use known contract addresses from the protocol's documentation.`;
          return results.join("\n\n");
        } catch {
          return `Web search unavailable. Use known contract addresses from the protocol's documentation.`;
        }
      },

      get_canvas_state: () => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        if (currentNodes.length === 0) return "Canvas is empty. No nodes or edges.";
        const workflow = convertCanvasToDefiFlow(
          currentNodes,
          currentEdges,
          wfName,
          undefined,
          tokensManifest ?? undefined,
          contractsManifest ?? undefined,
        );
        return JSON.stringify(workflow, null, 2);
      },

      fetch_data: async (days?: number, interval?: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        if (currentNodes.length === 0) return "Error: canvas is empty, nothing to fetch data for.";
        const workflow = convertCanvasToDefiFlow(
          currentNodes, currentEdges, wfName, undefined,
          tokensManifest ?? undefined, contractsManifest ?? undefined,
        );
        try {
          const res = await fetchData(workflow, { days: days ?? 30, interval: interval ?? "1h" });
          return `Data fetched successfully. Data dir: ${res.data_dir}`;
        } catch (err) {
          return `Fetch data failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      start_daemon: async (dryRun?: boolean, network?: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        if (currentNodes.length === 0) return "Error: canvas is empty, nothing to run.";
        const workflow = convertCanvasToDefiFlow(
          currentNodes, currentEdges, wfName, undefined,
          tokensManifest ?? undefined, contractsManifest ?? undefined,
        );
        try {
          const res = await startDaemon(workflow, {
            dry_run: dryRun ?? true,
            network: network ?? "mainnet",
          });
          return `Daemon started. Session ID: ${res.session_id}, Status: ${res.status}`;
        } catch (err) {
          return `Start daemon failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      stop_daemon: async (sessionId: string) => {
        try {
          const res = await stopDaemon(sessionId);
          return `Daemon stopped. Session: ${res.session_id}, Status: ${res.status}`;
        } catch (err) {
          return `Stop daemon failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      list_runs: async () => {
        try {
          const runs = await listRuns();
          if (runs.length === 0) return "No active or recent daemon sessions.";
          return runs.map((r) =>
            `- ${r.session_id}: ${r.workflow_name} (${r.status}, ${r.network})`
          ).join("\n");
        } catch (err) {
          return `List runs failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      get_run_status: async (sessionId: string) => {
        try {
          const s = await getRunStatus(sessionId);
          return `Session ${s.session_id}:
- Status: ${s.status}
- TVL: $${s.tvl.toFixed(2)}
- Network: ${s.network}
- Dry run: ${s.dry_run}
- Workflow: ${s.workflow_name}`;
        } catch (err) {
          return `Get status failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      list_data: async () => {
        try {
          const manifest = await getDataManifest();
          if (manifest.files.length === 0) return "No data files available. Use fetch_data first.";
          return manifest.files.map((f) =>
            `- ${f.name} (${(f.size / 1024).toFixed(1)} KB)`
          ).join("\n");
        } catch (err) {
          return `List data failed: ${err instanceof Error ? err.message : "API unavailable"}`;
        }
      },

      clear_canvas: () => {
        setNodes([]);
        setEdges([]);
        return "Canvas cleared. All nodes and edges removed.";
      },
    };
  }, [nodes, edges, wfName, tokensManifest, contractsManifest, addNode, setNodes, setEdges, setWfName, setTokensManifest, setContractsManifest]);

  // ── Send message ───────────────────────────────────────────────────

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || generating) return;
    if (!apiKey) {
      toast.error("Enter your OpenAI API key first");
      return;
    }

    const userMsg: Message = { role: "user", content: text };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    setGenerating(true);
    setToolActivities([]);

    // Build conversation for API
    const systemPrompt = await buildSystemPrompt(
      nodes,
      edges,
      wfName,
      tokensManifest ?? undefined,
      contractsManifest ?? undefined,
    );

    const apiMessages = [
      { role: "system" as const, content: systemPrompt },
      ...messages.map((m) => ({
        role: m.role as "user" | "assistant",
        content: m.content,
      })),
      { role: "user" as const, content: text },
    ];

    // Start streaming — accumulate raw text, parse think tags on each chunk
    const assistantMsg: Message = { role: "assistant", content: "" };
    setMessages((prev) => [...prev, assistantMsg]);
    let rawAccum = "";

    const abort = new AbortController();
    abortRef.current = abort;

    try {
      const handlers = buildHandlers();
      await agentLoop(
        apiKey,
        model,
        apiMessages,
        handlers,
        // onText
        (chunk) => {
          rawAccum += chunk;
          const { thinking, content } = parseThinkContent(rawAccum);
          setMessages((prev) => {
            const msgs = [...prev];
            const last = msgs[msgs.length - 1];
            if (last?.role === "assistant") {
              msgs[msgs.length - 1] = { ...last, content, thinking: thinking || undefined };
            }
            return msgs;
          });
        },
        // onToolCall
        (name, args) => {
          setToolActivities((prev) => [
            ...prev,
            { id: nanoid(6), name, args, status: "done" },
          ]);
        },
        abort.signal,
        baseUrl,
      );

      triggerAutosave({ immediate: true });
    } catch (err) {
      if ((err as Error).name === "AbortError") return;
      const errMsg = err instanceof Error ? err.message : "Unknown error";
      setMessages((prev) => {
        const msgs = [...prev];
        const last = msgs[msgs.length - 1];
        if (last?.role === "assistant") {
          msgs[msgs.length - 1] = {
            ...last,
            content: last.content + `\n\n**Error**: ${errMsg}`,
          };
        }
        return msgs;
      });
      toast.error(errMsg);
    } finally {
      setGenerating(false);
      abortRef.current = null;
    }
  }, [
    input,
    generating,
    apiKey,
    baseUrl,
    model,
    messages,
    nodes,
    edges,
    wfName,
    tokensManifest,
    contractsManifest,
    setMessages,
    setGenerating,
    buildHandlers,
    triggerAutosave,
  ]);

  const handleStop = () => {
    abortRef.current?.abort();
    setGenerating(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  // ── Render ─────────────────────────────────────────────────────────

  return (
    <div className="h-full flex flex-col">
      {/* API Key + Base URL + Model bar */}
      <div className="border-b px-3 py-2 space-y-1.5">
        <div className="flex gap-1.5">
          <Input
            className="h-7 text-xs flex-1 font-mono"
            type="password"
            placeholder="API key"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
          />
          <Input
            className="h-7 text-xs w-32 font-mono"
            placeholder="Model"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
        </div>
        <Input
          className="h-7 text-xs font-mono"
          placeholder="Base URL (e.g. https://api.openai.com/v1)"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
        />
        {!apiKey && (
          <p className="text-[10px] text-muted-foreground">
            Any OpenAI-compatible API. Key stored in memory only.
          </p>
        )}
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-3">
        {messages.length === 0 && (
          <div className="text-center text-muted-foreground text-xs mt-8 space-y-2">
            <Bot className="w-8 h-8 mx-auto opacity-40" />
            <p>Describe a DeFi strategy and I'll build it on the canvas.</p>
            <p className="text-[10px]">
              I can build nodes, connect edges, set manifests, validate, backtest, fetch data, start/stop daemons, and search the web.
            </p>
          </div>
        )}

        {messages.map((msg, i) => (
          <div key={i} className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
            <div
              className={`max-w-[90%] rounded-lg px-3 py-2 text-xs ${
                msg.role === "user"
                  ? "bg-primary text-primary-foreground"
                  : "bg-muted text-foreground"
              }`}
            >
              {/* Thinking dropdown */}
              {msg.thinking && (
                <details className="mb-1.5 group">
                  <summary className="cursor-pointer select-none text-[10px] text-muted-foreground hover:text-foreground flex items-center gap-1">
                    <ChevronRight className="w-3 h-3 transition-transform group-open:rotate-90" />
                    Thinking{generating && i === messages.length - 1 ? "..." : ""}
                  </summary>
                  <div className="mt-1 pl-4 text-[10px] text-muted-foreground whitespace-pre-wrap border-l border-muted-foreground/20">
                    {msg.thinking}
                  </div>
                </details>
              )}
              {/* Main content */}
              <div className="whitespace-pre-wrap">
                {msg.content || (!msg.thinking && generating && i === messages.length - 1 ? "..." : "")}
              </div>
            </div>
          </div>
        ))}

        {/* Tool activity log (shown while generating) */}
        {toolActivities.length > 0 && (
          <div className="space-y-1 border-l-2 border-muted pl-2">
            {toolActivities.map((ta) => (
              <div key={ta.id} className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                {ta.name === "web_search" ? (
                  <Search className="w-3 h-3" />
                ) : ta.name === "validate" ? (
                  <CheckCircle className="w-3 h-3" />
                ) : ta.name === "start_daemon" ? (
                  <Play className="w-3 h-3" />
                ) : ta.name === "stop_daemon" ? (
                  <CircleStop className="w-3 h-3" />
                ) : ta.name === "fetch_data" || ta.name === "list_data" ? (
                  <Database className="w-3 h-3" />
                ) : ta.name === "clear_canvas" ? (
                  <Trash2 className="w-3 h-3" />
                ) : (
                  <Wrench className="w-3 h-3" />
                )}
                <span className="font-mono">{ta.name}</span>
                {ta.name === "add_node" && (
                  <span className="text-foreground/60">
                    {(() => {
                      try { return JSON.parse(ta.args).node?.id; } catch { return ""; }
                    })()}
                  </span>
                )}
                {ta.name === "add_edge" && (
                  <span className="text-foreground/60">
                    {(() => {
                      try {
                        const a = JSON.parse(ta.args);
                        return `${a.from_node} → ${a.to_node}`;
                      } catch { return ""; }
                    })()}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input bar */}
      <div className="border-t px-3 py-2">
        <div className="flex gap-1.5">
          <textarea
            ref={textareaRef}
            className="flex-1 resize-none rounded-md border bg-background px-3 py-2 text-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            rows={2}
            placeholder={generating ? "Agent is working..." : "Describe a strategy..."}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={generating}
          />
          {generating ? (
            <Button variant="ghost" size="sm" className="h-auto px-2" onClick={handleStop}>
              <Square className="w-4 h-4" />
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="h-auto px-2"
              onClick={handleSend}
              disabled={!input.trim() || !apiKey}
            >
              <Send className="w-4 h-4" />
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
