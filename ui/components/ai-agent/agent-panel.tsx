"use client";

import { useAtom, useAtomValue, useSetAtom } from "jotai";
import { useCallback, useEffect, useRef, useState } from "react";
import { nanoid } from "nanoid";
import { toast } from "sonner";
import { Bot, Send, Square, Wrench, Search, CheckCircle, Play, CircleStop, Database, Trash2, ChevronRight, LayoutGrid, BarChart3, Loader2, XCircle } from "lucide-react";
import Markdown from "react-markdown";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  openaiKeyAtom,
  openaiBaseUrlAtom,
  openaiModelAtom,
  messagesAtom,
  generatingAtom,
  PROMPT_TEMPLATES,
  type Message,
  type ToolActivity,
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
  type DefiFlowWorkflow,
} from "@/lib/types/defi-flow";
import { convertCanvasToDefiFlow, convertDefiFlowToCanvas } from "@/lib/converters/canvas-defi-flow";
import type { CanvasNode, CanvasEdge } from "@/lib/types/canvas";
import { validateWorkflow as validateWasm } from "@/lib/wasm";
import {
  validateWorkflow as validateApi,
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
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const abortRef = useRef<AbortController | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Mutable refs so tool handlers always see the latest state,
  // even when multiple tools run in the same agent loop iteration
  // (React state updates are async and won't be visible until re-render)
  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);
  const wfNameRef = useRef(wfName);
  const tokensManifestRef = useRef(tokensManifest);
  const contractsManifestRef = useRef(contractsManifest);
  useEffect(() => { nodesRef.current = nodes; }, [nodes]);
  useEffect(() => { edgesRef.current = edges; }, [edges]);
  useEffect(() => { wfNameRef.current = wfName; }, [wfName]);
  useEffect(() => { tokensManifestRef.current = tokensManifest; }, [tokensManifest]);
  useEffect(() => { contractsManifestRef.current = contractsManifest; }, [contractsManifest]);

  // Wrappers that update BOTH React state AND the ref immediately
  const setNodesNow = useCallback((val: CanvasNode[] | ((prev: CanvasNode[]) => CanvasNode[])) => {
    if (typeof val === "function") {
      setNodes((prev) => {
        const next = val(prev);
        nodesRef.current = next;
        return next;
      });
    } else {
      nodesRef.current = val;
      setNodes(val);
    }
  }, [setNodes]);

  const setEdgesNow = useCallback((val: CanvasEdge[] | ((prev: CanvasEdge[]) => CanvasEdge[])) => {
    if (typeof val === "function") {
      setEdges((prev) => {
        const next = val(prev);
        edgesRef.current = next;
        return next;
      });
    } else {
      edgesRef.current = val;
      setEdges(val);
    }
  }, [setEdges]);

  const setWfNameNow = useCallback((val: string) => {
    wfNameRef.current = val;
    setWfName(val);
  }, [setWfName]);

  const setTokensManifestNow = useCallback((val: Record<string, Record<string, string>> | ((prev: Record<string, Record<string, string>> | null) => Record<string, Record<string, string>>)) => {
    if (typeof val === "function") {
      setTokensManifest((prev) => {
        const next = val(prev ?? {});
        tokensManifestRef.current = next;
        return next;
      });
    } else {
      tokensManifestRef.current = val;
      setTokensManifest(val);
    }
  }, [setTokensManifest]);

  const setContractsManifestNow = useCallback((val: Record<string, Record<string, string>> | ((prev: Record<string, Record<string, string>> | null) => Record<string, Record<string, string>>)) => {
    if (typeof val === "function") {
      setContractsManifest((prev) => {
        const next = val(prev ?? {});
        contractsManifestRef.current = next;
        return next;
      });
    } else {
      contractsManifestRef.current = val;
      setContractsManifest(val);
    }
  }, [setContractsManifest]);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // ── Tool Handlers ──────────────────────────────────────────────────

  const buildHandlers = useCallback((): ToolHandlers => {
    // Read from refs — always sees latest state even mid-agent-loop
    const getNodes = () => nodesRef.current;
    const getEdges = () => edgesRef.current;

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

        setNodesNow([...currentNodes, canvasNode]);
        return `Added node "${node.id}" (${node.type})`;
      },

      remove_node: (nodeId: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const exists = currentNodes.find((n) => n.id === nodeId);
        if (!exists) return `Error: node "${nodeId}" not found`;

        setNodesNow(currentNodes.filter((n) => n.id !== nodeId));
        setEdgesNow(
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
        setNodesNow(newNodes);
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
        setEdgesNow([...currentEdges, newEdge]);
        return `Added edge ${fromNode} → ${toNode} (${edgeToken})`;
      },

      remove_edge: (fromNode: string, toNode: string) => {
        const currentEdges = getEdges();
        const filtered = currentEdges.filter(
          (e) => !(e.source === fromNode && e.target === toNode),
        );
        if (filtered.length === currentEdges.length)
          return `Error: no edge ${fromNode} → ${toNode} found`;
        setEdgesNow(filtered);
        return `Removed edge ${fromNode} → ${toNode}`;
      },

      set_manifest: (
        type: "tokens" | "contracts",
        key: string,
        chain: string,
        address: string,
      ) => {
        if (type === "tokens") {
          setTokensManifestNow((prev) => {
            const m = prev ? structuredClone(prev) : {};
            if (!m[key]) m[key] = {};
            m[key][chain] = address;
            return m;
          });
        } else {
          setContractsManifestNow((prev) => {
            const m = prev ? structuredClone(prev) : {};
            if (!m[key]) m[key] = {};
            m[key][chain] = address;
            return m;
          });
        }
        return `Set ${type}.${key}.${chain} = ${address}`;
      },

      set_name: (name: string) => {
        setWfNameNow(name);
        return `Strategy name set to "${name}"`;
      },

      validate: async () => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const workflow = convertCanvasToDefiFlow(
          currentNodes,
          currentEdges,
          wfNameRef.current,
          undefined,
          tokensManifestRef.current ?? undefined,
          contractsManifestRef.current ?? undefined,
        );
        // Try API (offline + on-chain), only fall back to WASM on network errors
        try {
          const result = await validateApi(workflow, true);
          const errors = result.errors ?? [];
          const warnings = result.warnings ?? [];
          let output = result.valid ? "Validation passed (offline + on-chain)." : `Validation failed with ${errors.length} error(s):\n${errors.join("\n")}`;
          if (warnings.length > 0) {
            output += `\n\nWarnings (${warnings.length}):\n${warnings.join("\n")}`;
          }
          return output;
        } catch (err) {
          // Only fall back to WASM if it's a network error (API not running)
          const msg = err instanceof Error ? err.message : String(err);
          if (msg.includes("fetch") || msg.includes("Failed") || msg.includes("NetworkError") || msg.includes("ECONNREFUSED")) {
            const json = JSON.stringify(workflow);
            const result = await validateWasm(json);
            if (result.valid) return "Validation passed (offline only — API server not running for on-chain checks).";
            return `Validation failed with ${(result.errors ?? []).length} error(s):\n${(result.errors ?? []).join("\n")}`;
          }
          // API returned an error response — surface it, don't swallow
          return `Validation error: ${msg}`;
        }
      },

      backtest: async (capital?: number, monteCarlo?: number) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        const workflow = convertCanvasToDefiFlow(
          currentNodes,
          currentEdges,
          wfNameRef.current,
          undefined,
          tokensManifestRef.current ?? undefined,
          contractsManifestRef.current ?? undefined,
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
          wfNameRef.current,
          undefined,
          tokensManifestRef.current ?? undefined,
          contractsManifestRef.current ?? undefined,
        );
        return JSON.stringify(workflow, null, 2);
      },

      fetch_data: async (days?: number, interval?: string) => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        if (currentNodes.length === 0) return "Error: canvas is empty, nothing to fetch data for.";
        const workflow = convertCanvasToDefiFlow(
          currentNodes, currentEdges, wfNameRef.current, undefined,
          tokensManifestRef.current ?? undefined, contractsManifestRef.current ?? undefined,
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
          currentNodes, currentEdges, wfNameRef.current, undefined,
          tokensManifestRef.current ?? undefined, contractsManifestRef.current ?? undefined,
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
        setNodesNow([]);
        setEdgesNow([]);
        return "Canvas cleared. All nodes and edges removed.";
      },

      auto_layout: () => {
        const currentNodes = getNodes();
        const currentEdges = getEdges();
        if (currentNodes.length === 0) return "Canvas is empty, nothing to layout.";

        // Build adjacency
        const children: Record<string, string[]> = {};
        const parents: Record<string, string[]> = {};
        for (const n of currentNodes) {
          children[n.id] = [];
          parents[n.id] = [];
        }
        for (const e of currentEdges) {
          children[e.source]?.push(e.target);
          parents[e.target]?.push(e.source);
        }

        // Topological sort via Kahn's algorithm
        const inDeg: Record<string, number> = {};
        for (const n of currentNodes) inDeg[n.id] = parents[n.id].length;
        const queue = currentNodes.filter((n) => inDeg[n.id] === 0).map((n) => n.id);
        const sorted: string[] = [];
        while (queue.length > 0) {
          const id = queue.shift()!;
          sorted.push(id);
          for (const child of children[id] ?? []) {
            inDeg[child]--;
            if (inDeg[child] === 0) queue.push(child);
          }
        }
        // Add any remaining (cycles or disconnected)
        for (const n of currentNodes) {
          if (!sorted.includes(n.id)) sorted.push(n.id);
        }

        // Assign layers by longest path from roots
        const layer: Record<string, number> = {};
        for (const id of sorted) {
          const pLayers = parents[id].map((p) => layer[p] ?? 0);
          layer[id] = pLayers.length > 0 ? Math.max(...pLayers) + 1 : 0;
        }

        // Group by layer
        const layers: Record<number, string[]> = {};
        for (const [id, l] of Object.entries(layer)) {
          if (!layers[l]) layers[l] = [];
          layers[l].push(id);
        }

        const SPACING_X = 320;
        const SPACING_Y = 160;
        const START_X = 60;
        const START_Y = 60;

        const newNodes = currentNodes.map((n) => {
          const l = layer[n.id] ?? 0;
          const nodesInLayer = layers[l] ?? [n.id];
          const idx = nodesInLayer.indexOf(n.id);
          return {
            ...n,
            position: {
              x: START_X + l * SPACING_X,
              y: START_Y + idx * SPACING_Y,
            },
          };
        });

        setNodesNow(newNodes);
        return `Auto-layout complete. Arranged ${newNodes.length} nodes in ${Object.keys(layers).length} columns.`;
      },

      import_workflow: (workflow: unknown) => {
        try {
          const wf = workflow as DefiFlowWorkflow;
          if (!wf.nodes || !wf.edges) return "Error: workflow must have nodes and edges arrays";
          const result = convertDefiFlowToCanvas(wf);
          setNodesNow(result.nodes);
          setEdgesNow(result.edges);
          if (wf.name) setWfNameNow(wf.name);
          if (result.tokens) setTokensManifestNow(result.tokens);
          if (result.contracts) setContractsManifestNow(result.contracts);
          return `Imported strategy "${wf.name ?? "Untitled"}": ${result.nodes.length} nodes, ${result.edges.length} edges. Call auto_layout to arrange them.`;
        } catch (err) {
          return `Import failed: ${err instanceof Error ? err.message : String(err)}`;
        }
      },
    };
  }, [setNodesNow, setEdgesNow, setWfNameNow, setTokensManifestNow, setContractsManifestNow]);

  // ── Send message ───────────────────────────────────────────────────

  const handleSend = useCallback(async (overrideText?: string) => {
    const text = (overrideText ?? input).trim();
    if (!text || generating) return;
    if (!apiKey) {
      toast.error("Enter your OpenAI API key first");
      return;
    }

    const userMsg: Message = { role: "user", content: text };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    setGenerating(true);

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
          setMessages((prev) => {
            const msgs = [...prev];
            const last = msgs[msgs.length - 1];
            if (last?.role === "assistant") {
              const activities = [...(last.toolActivities ?? [])];
              activities.push({ id: nanoid(6), name, args, status: "running" });
              msgs[msgs.length - 1] = { ...last, toolActivities: activities };
            }
            return msgs;
          });
        },
        // onToolResult
        (name, result) => {
          setMessages((prev) => {
            const msgs = [...prev];
            const last = msgs[msgs.length - 1];
            if (last?.role === "assistant" && last.toolActivities) {
              const activities = [...last.toolActivities];
              // Find the last activity with this name that is still running
              for (let j = activities.length - 1; j >= 0; j--) {
                if (activities[j].name === name && activities[j].status === "running") {
                  const isError = result.startsWith("Tool error:") || result.startsWith("Error:");
                  activities[j] = {
                    ...activities[j],
                    status: isError ? "error" : "done",
                    result: (name === "validate" || name === "backtest") ? result : undefined,
                  };
                  break;
                }
              }
              msgs[msgs.length - 1] = { ...last, toolActivities: activities };
            }
            return msgs;
          });
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

  const handleTemplateClick = useCallback((message: string) => {
    if (message === "") {
      textareaRef.current?.focus();
      return;
    }
    handleSend(message);
  }, [handleSend]);

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
          <div className="text-center text-muted-foreground text-xs mt-8 space-y-4">
            <Bot className="w-8 h-8 mx-auto opacity-40" />
            <p>Describe a strategy and I'll build, validate, and backtest it automatically.</p>
            <div className="grid grid-cols-1 gap-1.5 px-2 max-w-xs mx-auto">
              {PROMPT_TEMPLATES.map((t) => (
                <button
                  key={t.id}
                  className="flex items-center gap-2 rounded-lg border border-border/50 bg-background/50 px-3 py-2 text-left text-xs hover:bg-muted/80 hover:border-border transition-colors disabled:opacity-50"
                  onClick={() => handleTemplateClick(t.message)}
                  disabled={!apiKey || generating}
                >
                  <span className="text-sm shrink-0">{t.icon}</span>
                  <div className="min-w-0">
                    <div className="font-medium text-foreground truncate">{t.label}</div>
                    <div className="text-[10px] text-muted-foreground truncate">{t.description}</div>
                  </div>
                  <ChevronRight className="w-3 h-3 ml-auto shrink-0 opacity-40" />
                </button>
              ))}
            </div>
            <p className="text-[10px] opacity-60">
              Click a template or describe your own strategy below.
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
              {msg.role === "assistant" ? (
                <Markdown
                  components={{
                    p: ({ children }) => <p className="mb-1.5 last:mb-0">{children}</p>,
                    strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
                    ul: ({ children }) => <ul className="list-disc pl-4 mb-1.5">{children}</ul>,
                    ol: ({ children }) => <ol className="list-decimal pl-4 mb-1.5">{children}</ol>,
                    li: ({ children }) => <li className="mb-0.5">{children}</li>,
                    code: ({ children, className }) => {
                      const isBlock = className?.includes("language-");
                      return isBlock ? (
                        <pre className="bg-background/50 rounded px-2 py-1.5 my-1.5 overflow-x-auto text-[10px]">
                          <code>{children}</code>
                        </pre>
                      ) : (
                        <code className="bg-background/50 rounded px-1 py-0.5 text-[10px] font-mono">{children}</code>
                      );
                    },
                    pre: ({ children }) => <>{children}</>,
                    h1: ({ children }) => <h1 className="font-bold text-sm mb-1">{children}</h1>,
                    h2: ({ children }) => <h2 className="font-bold text-xs mb-1">{children}</h2>,
                    h3: ({ children }) => <h3 className="font-semibold text-xs mb-1">{children}</h3>,
                    table: ({ children }) => (
                      <div className="my-1.5 overflow-x-auto rounded border border-border/50">
                        <table className="w-full text-[10px]">{children}</table>
                      </div>
                    ),
                    thead: ({ children }) => <thead className="bg-background/60">{children}</thead>,
                    tbody: ({ children }) => <tbody>{children}</tbody>,
                    tr: ({ children }) => <tr className="border-b border-border/30 last:border-0">{children}</tr>,
                    th: ({ children }) => <th className="px-2 py-1 text-left font-semibold text-muted-foreground whitespace-nowrap">{children}</th>,
                    td: ({ children }) => <td className="px-2 py-1 whitespace-nowrap font-mono">{children}</td>,
                  }}
                >
                  {msg.content || (!msg.thinking && generating && i === messages.length - 1 ? "..." : "")}
                </Markdown>
              ) : (
                <div className="whitespace-pre-wrap">
                  {msg.content}
                </div>
              )}

              {/* Inline tool activities */}
              {msg.toolActivities && msg.toolActivities.length > 0 && (
                <div className="mt-1.5 pt-1.5 border-t border-foreground/10 space-y-1">
                  {msg.toolActivities.map((ta) => (
                    <div key={ta.id}>
                      <div className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                        {ta.status === "running" ? (
                          <Loader2 className="w-3 h-3 shrink-0 animate-spin" />
                        ) : ta.status === "error" ? (
                          <XCircle className="w-3 h-3 shrink-0 text-destructive" />
                        ) : ta.name === "web_search" ? (
                          <Search className="w-3 h-3 shrink-0" />
                        ) : ta.name === "validate" ? (
                          <CheckCircle className="w-3 h-3 shrink-0" />
                        ) : ta.name === "backtest" ? (
                          <BarChart3 className="w-3 h-3 shrink-0" />
                        ) : ta.name === "start_daemon" ? (
                          <Play className="w-3 h-3 shrink-0" />
                        ) : ta.name === "stop_daemon" ? (
                          <CircleStop className="w-3 h-3 shrink-0" />
                        ) : ta.name === "fetch_data" || ta.name === "list_data" ? (
                          <Database className="w-3 h-3 shrink-0" />
                        ) : ta.name === "clear_canvas" ? (
                          <Trash2 className="w-3 h-3 shrink-0" />
                        ) : ta.name === "auto_layout" ? (
                          <LayoutGrid className="w-3 h-3 shrink-0" />
                        ) : (
                          <Wrench className="w-3 h-3 shrink-0" />
                        )}
                        <span className="font-mono">{ta.name}</span>
                        {ta.status === "running" && (
                          <span className="text-foreground/40 italic">running...</span>
                        )}
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
                      {/* Show validate/backtest results inline */}
                      {ta.result && (ta.name === "validate" || ta.name === "backtest") && (
                        <details className="ml-[18px] mt-0.5 group">
                          <summary className="cursor-pointer select-none text-[10px] text-muted-foreground/70 hover:text-muted-foreground flex items-center gap-1">
                            <ChevronRight className="w-2.5 h-2.5 transition-transform group-open:rotate-90" />
                            {ta.status === "error" ? "Error details" : "Results"}
                          </summary>
                          <pre className="mt-0.5 ml-3.5 text-[9px] leading-tight text-muted-foreground/80 whitespace-pre-wrap bg-background/40 rounded px-2 py-1 max-h-48 overflow-y-auto font-mono">
                            {ta.result}
                          </pre>
                        </details>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        ))}

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
              onClick={() => handleSend()}
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
