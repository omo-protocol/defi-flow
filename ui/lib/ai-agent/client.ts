import { getToken } from "@/lib/auth-store";

// ── Types ────────────────────────────────────────────────────────────

export type ChatMessage =
  | { role: "system" | "user"; content: string }
  | { role: "assistant"; content: string | null; tool_calls?: ToolCall[] }
  | { role: "tool"; tool_call_id: string; content: string };

type ToolCall = {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
};

// ── Tool definitions ─────────────────────────────────────────────────

export const TOOLS = [
  {
    type: "function" as const,
    function: {
      name: "add_node",
      description:
        "Add a node to the strategy canvas. Returns the created node ID.",
      parameters: {
        type: "object",
        properties: {
          node: {
            type: "object",
            description:
              'The full node object matching the DefiFlow schema. Must include "type" and "id" fields. Example: {"type":"perp","id":"short_eth","venue":"Hyperliquid","pair":"ETH/USDC","action":"open","direction":"short","leverage":1.0}',
          },
        },
        required: ["node"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "remove_node",
      description: "Remove a node (and its connected edges) from the canvas.",
      parameters: {
        type: "object",
        properties: {
          node_id: { type: "string", description: "ID of the node to remove" },
        },
        required: ["node_id"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "update_node",
      description:
        "Update fields on an existing node. Merges the provided fields into the existing node.",
      parameters: {
        type: "object",
        properties: {
          node_id: { type: "string", description: "ID of the node to update" },
          fields: {
            type: "object",
            description:
              "Fields to update. Only include fields that should change.",
          },
        },
        required: ["node_id", "fields"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "add_edge",
      description:
        "Add an edge (token flow) between two nodes. The token is auto-inferred if not specified.",
      parameters: {
        type: "object",
        properties: {
          from_node: { type: "string", description: "Source node ID" },
          to_node: { type: "string", description: "Target node ID" },
          token: {
            type: "string",
            description:
              "Token symbol (e.g. 'USDC', 'ETH'). Auto-inferred if omitted.",
          },
        },
        required: ["from_node", "to_node"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "remove_edge",
      description: "Remove an edge between two nodes.",
      parameters: {
        type: "object",
        properties: {
          from_node: { type: "string", description: "Source node ID" },
          to_node: { type: "string", description: "Target node ID" },
        },
        required: ["from_node", "to_node"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "set_manifest",
      description:
        "Set a token or contract address in the manifest. Only needed for EVM chains (not namespace chains like hyperliquid).",
      parameters: {
        type: "object",
        properties: {
          manifest_type: {
            type: "string",
            enum: ["tokens", "contracts"],
            description: "Which manifest to update",
          },
          key: {
            type: "string",
            description:
              "Token symbol (e.g. 'USDC') or contract label (e.g. 'hyperlend_pool')",
          },
          chain: {
            type: "string",
            description: "Chain name (e.g. 'hyperevm', 'base')",
          },
          address: { type: "string", description: "Contract address (0x...)" },
        },
        required: ["manifest_type", "key", "chain", "address"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "set_name",
      description: "Set the strategy name.",
      parameters: {
        type: "object",
        properties: {
          name: { type: "string", description: "Strategy name" },
        },
        required: ["name"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "validate",
      description:
        "Run offline validation on the current canvas state. Returns validation errors or success.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "backtest",
      description:
        "Run a backtest on the current strategy. Requires the API server to be running. Returns performance metrics (TWRR, Sharpe, drawdown, etc.). Optionally run Monte Carlo simulations for risk analysis.",
      parameters: {
        type: "object",
        properties: {
          capital: {
            type: "number",
            description: "Starting capital in USD (default 10000)",
          },
          monte_carlo: {
            type: "number",
            description:
              "Number of Monte Carlo simulations to run (e.g. 100). Uses parametric GBM/OU/AR(1) to generate synthetic paths. Returns percentile stats (5th/25th/50th/75th/95th) across sims.",
          },
        },
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "web_search",
      description:
        "Search the web for DeFi protocol info: contract addresses, pool addresses, DeFiLlama slugs, chain deployments. Use this when you need real data.",
      parameters: {
        type: "object",
        properties: {
          query: {
            type: "string",
            description:
              "Search query, e.g. 'HyperLend pool address hyperevm'",
          },
        },
        required: ["query"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "get_canvas_state",
      description:
        "Get the current strategy state from the canvas as JSON. Use this to understand what exists before making modifications.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "fetch_data",
      description:
        "Fetch historical data for the current strategy (perp prices, funding, lending APY, etc.). Must be done before backtesting if auto_fetch is not used.",
      parameters: {
        type: "object",
        properties: {
          days: {
            type: "number",
            description: "Number of days of history to fetch (default 30)",
          },
          interval: {
            type: "string",
            description: "Data interval, e.g. '1h', '4h', '1d' (default '1h')",
          },
        },
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "start_daemon",
      description:
        "Start a live execution daemon for the current strategy. Returns a session ID to monitor. Use dry_run=true for paper trading.",
      parameters: {
        type: "object",
        properties: {
          dry_run: {
            type: "boolean",
            description: "If true, paper-trade only (default true)",
          },
          network: {
            type: "string",
            description: "Network to run on, e.g. 'mainnet', 'testnet' (default 'mainnet')",
          },
        },
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "stop_daemon",
      description: "Stop a running execution daemon by session ID.",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string", description: "Session ID to stop" },
        },
        required: ["session_id"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "list_runs",
      description:
        "List all active and recent daemon sessions with their status.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "get_run_status",
      description:
        "Get detailed status of a running daemon session (TVL, status, network, etc.).",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string", description: "Session ID to query" },
        },
        required: ["session_id"],
      },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "list_data",
      description:
        "List available CSV data files that have been fetched or uploaded.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "clear_canvas",
      description:
        "Remove ALL nodes and edges from the canvas. Use when starting a completely new strategy from scratch.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "auto_layout",
      description:
        "Auto-arrange all nodes on the canvas in a clean left-to-right DAG layout. Call this after building a strategy.",
      parameters: { type: "object", properties: {} },
    },
  },
  {
    type: "function" as const,
    function: {
      name: "import_workflow",
      description:
        "Import a complete DefiFlowWorkflow JSON onto the canvas. This REPLACES the entire canvas with the new strategy. Use this instead of individual add_node/add_edge calls when building a new strategy or making major changes. Much faster than adding nodes one by one.",
      parameters: {
        type: "object",
        properties: {
          workflow: {
            type: "object",
            description:
              "The full DefiFlowWorkflow JSON object with name, nodes, edges, tokens, and contracts fields.",
          },
        },
        required: ["workflow"],
      },
    },
  },
];

// ── Tool handlers (injected by the panel) ────────────────────────────

export type ToolHandlers = {
  add_node: (node: unknown) => string;
  remove_node: (nodeId: string) => string;
  update_node: (nodeId: string, fields: Record<string, unknown>) => string;
  add_edge: (fromNode: string, toNode: string, token?: string) => string;
  remove_edge: (fromNode: string, toNode: string) => string;
  set_manifest: (
    type: "tokens" | "contracts",
    key: string,
    chain: string,
    address: string,
  ) => string;
  set_name: (name: string) => string;
  validate: () => Promise<string>;
  backtest: (capital?: number, monteCarlo?: number) => Promise<string>;
  web_search: (query: string) => Promise<string>;
  get_canvas_state: () => string;
  fetch_data: (days?: number, interval?: string) => Promise<string>;
  start_daemon: (dryRun?: boolean, network?: string) => Promise<string>;
  stop_daemon: (sessionId: string) => Promise<string>;
  list_runs: () => Promise<string>;
  get_run_status: (sessionId: string) => Promise<string>;
  list_data: () => Promise<string>;
  clear_canvas: () => string;
  auto_layout: () => string;
  import_workflow: (workflow: unknown) => string;
};

// ── Agentic loop ─────────────────────────────────────────────────────

/**
 * Run an agentic loop with tool use.
 * The model calls tools, gets results, and continues until it responds with text only.
 */
export async function agentLoop(
  messages: ChatMessage[],
  handlers: ToolHandlers,
  onText: (text: string) => void,
  onToolCall?: (name: string, args: string) => void,
  onToolResult?: (name: string, result: string) => void,
  signal?: AbortSignal,
): Promise<string> {
  const conversation = [...messages];
  let fullText = "";
  const MAX_ITERATIONS = 200;

  for (let i = 0; i < MAX_ITERATIONS; i++) {
    const token = getToken();
    const res = await fetch("/api/ai/chat", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify({
        messages: conversation,
        tools: TOOLS,
        temperature: 0.3,
      }),
      signal,
    });

    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      if (res.status === 429) {
        throw new Error(body?.error ?? "Rate limit exceeded. Please wait a moment.");
      }
      throw new Error(
        body?.error ??
          `API error ${res.status}: ${res.statusText}`,
      );
    }

    const reader = res.body?.getReader();
    if (!reader) throw new Error("No response body");

    const decoder = new TextDecoder();
    let buffer = "";
    let textChunk = "";
    let toolCalls: ToolCall[] = [];

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        const data = trimmed.slice(6);
        if (data === "[DONE]") continue;

        try {
          const parsed = JSON.parse(data);
          const choice = parsed.choices?.[0];
          if (!choice?.delta) continue;

          if (choice.delta.content) {
            textChunk += choice.delta.content;
            fullText += choice.delta.content;
            onText(choice.delta.content);
          }

          if (choice.delta.tool_calls) {
            for (const tc of choice.delta.tool_calls) {
              if (tc.index !== undefined) {
                while (toolCalls.length <= tc.index) {
                  toolCalls.push({
                    id: "",
                    type: "function",
                    function: { name: "", arguments: "" },
                  });
                }
                if (tc.id) toolCalls[tc.index].id = tc.id;
                if (tc.function?.name)
                  toolCalls[tc.index].function.name += tc.function.name;
                if (tc.function?.arguments)
                  toolCalls[tc.index].function.arguments +=
                    tc.function.arguments;
              }
            }
          }
        } catch {
          // Skip malformed chunks
        }
      }
    }

    // No tool calls — done
    if (toolCalls.length === 0) {
      conversation.push({ role: "assistant", content: textChunk });
      break;
    }

    // Add assistant message with tool calls
    conversation.push({
      role: "assistant",
      content: textChunk || null,
      tool_calls: toolCalls,
    });

    // Execute each tool call
    for (const tc of toolCalls) {
      const name = tc.function.name;
      const rawArgs = tc.function.arguments;
      onToolCall?.(name, rawArgs);

      let result: string;
      try {
        const args = JSON.parse(rawArgs);
        switch (name) {
          case "add_node":
            result = handlers.add_node(args.node);
            break;
          case "remove_node":
            result = handlers.remove_node(args.node_id);
            break;
          case "update_node":
            result = handlers.update_node(args.node_id, args.fields);
            break;
          case "add_edge":
            result = handlers.add_edge(args.from_node, args.to_node, args.token);
            break;
          case "remove_edge":
            result = handlers.remove_edge(args.from_node, args.to_node);
            break;
          case "set_manifest":
            result = handlers.set_manifest(
              args.manifest_type,
              args.key,
              args.chain,
              args.address,
            );
            break;
          case "set_name":
            result = handlers.set_name(args.name);
            break;
          case "validate":
            result = await handlers.validate();
            break;
          case "backtest":
            result = await handlers.backtest(args.capital, args.monte_carlo);
            break;
          case "web_search":
            result = await handlers.web_search(args.query);
            break;
          case "get_canvas_state":
            result = handlers.get_canvas_state();
            break;
          case "fetch_data":
            result = await handlers.fetch_data(args.days, args.interval);
            break;
          case "start_daemon":
            result = await handlers.start_daemon(args.dry_run, args.network);
            break;
          case "stop_daemon":
            result = await handlers.stop_daemon(args.session_id);
            break;
          case "list_runs":
            result = await handlers.list_runs();
            break;
          case "get_run_status":
            result = await handlers.get_run_status(args.session_id);
            break;
          case "list_data":
            result = await handlers.list_data();
            break;
          case "clear_canvas":
            result = handlers.clear_canvas();
            break;
          case "auto_layout":
            result = handlers.auto_layout();
            break;
          case "import_workflow":
            result = handlers.import_workflow(args.workflow);
            break;
          default:
            result = `Unknown tool: ${name}`;
        }
      } catch (err) {
        result = `Tool error: ${err instanceof Error ? err.message : String(err)}`;
      }

      onToolResult?.(name, result);

      conversation.push({
        role: "tool",
        tool_call_id: tc.id,
        content: result,
      });
    }
  }

  return fullText;
}
