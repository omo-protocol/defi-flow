// ── Core types matching Rust defi-flow model exactly ──────────────────

export type Chain = {
  name: string;
  chain_id?: number;
  rpc_url?: string;
};

export const KNOWN_CHAINS: Chain[] = [
  { name: "hyperliquid", chain_id: 1337 },
  { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" },
  { name: "ethereum", chain_id: 1, rpc_url: "https://eth.llamarpc.com" },
  { name: "arbitrum", chain_id: 42161, rpc_url: "https://arb1.arbitrum.io/rpc" },
  { name: "optimism", chain_id: 10, rpc_url: "https://mainnet.optimism.io" },
  { name: "base", chain_id: 8453, rpc_url: "https://mainnet.base.org" },
  { name: "mantle", chain_id: 5000, rpc_url: "https://rpc.mantle.xyz" },
];

// ── Amount types ─────────────────────────────────────────────────────

export type Amount =
  | { type: "all" }
  | { type: "percentage"; value: number }
  | { type: "fixed"; value: string };

// ── Trigger types ────────────────────────────────────────────────────

export type CronInterval = "hourly" | "daily" | "weekly" | "monthly";

export type Trigger =
  | { type: "cron"; interval: CronInterval }
  | { type: "on_event"; event: string };

// ── Edge ─────────────────────────────────────────────────────────────

export type DefiEdge = {
  from_node: string;
  to_node: string;
  token: string;
  amount: Amount;
};

// ── Venue / Provider enums ───────────────────────────────────────────

export type PerpVenue = "Hyperliquid" | "Hyena";
export type SpotVenue = "Hyperliquid";
export type OptionsVenue = "Rysk";
export type LpVenue = "Aerodrome";

export type PerpAction = "open" | "close" | "adjust" | "collect_funding";
export type PerpDirection = "long" | "short";
export type SpotSide = "buy" | "sell";

export type LendingArchetype = "aave_v3" | "aave_v2" | "morpho" | "compound_v3" | "init_capital";
export type LendingAction = "supply" | "withdraw" | "borrow" | "repay" | "claim_rewards";

export type VaultArchetype = "morpho_v2";
export type VaultAction = "deposit" | "withdraw" | "claim_rewards";

export type LpAction = "add_liquidity" | "remove_liquidity" | "claim_rewards" | "compound" | "stake_gauge" | "unstake_gauge";

export type OptionsAction = "sell_covered_call" | "sell_cash_secured_put" | "buy_call" | "buy_put" | "collect_premium" | "roll" | "close";
export type RyskAsset = "ETH" | "BTC" | "HYPE" | "SOL";

export type PendleAction = "mint_pt" | "redeem_pt" | "mint_yt" | "redeem_yt" | "claim_rewards";

export type MovementType = "swap" | "bridge" | "swap_bridge";
export type MovementProvider = "LiFi" | "HyperliquidNative";

export type OptimizerStrategy = "kelly";

// ── Allocation ───────────────────────────────────────────────────────

export type VenueAllocation = {
  target_node?: string;
  target_nodes?: string[];
  expected_return?: number;
  volatility?: number;
  correlation: number;
};

// ── Node types (discriminated union matching Rust Node enum) ─────────

export type WalletNode = {
  type: "wallet";
  id: string;
  chain: Chain;
  token: string;
  address: string;
};

export type PerpNode = {
  type: "perp";
  id: string;
  venue: PerpVenue;
  pair: string;
  action: PerpAction;
  direction?: PerpDirection;
  leverage?: number;
  margin_token?: string;
  trigger?: Trigger;
};

export type SpotNode = {
  type: "spot";
  id: string;
  venue: SpotVenue;
  pair: string;
  side: SpotSide;
  trigger?: Trigger;
};

export type LendingNode = {
  type: "lending";
  id: string;
  archetype: LendingArchetype;
  chain: Chain;
  pool_address: string;
  asset: string;
  action: LendingAction;
  rewards_controller?: string;
  defillama_slug?: string;
  trigger?: Trigger;
};

export type VaultNode = {
  type: "vault";
  id: string;
  archetype: VaultArchetype;
  chain: Chain;
  vault_address: string;
  asset: string;
  action: VaultAction;
  defillama_slug?: string;
  trigger?: Trigger;
};

export type LpNode = {
  type: "lp";
  id: string;
  venue: LpVenue;
  pool: string;
  action: LpAction;
  tick_lower?: number;
  tick_upper?: number;
  tick_spacing?: number;
  chain?: Chain;
  trigger?: Trigger;
};

export type OptionsNode = {
  type: "options";
  id: string;
  venue: OptionsVenue;
  asset: RyskAsset;
  action: OptionsAction;
  delta_target?: number;
  days_to_expiry?: number;
  min_apy?: number;
  batch_size?: number;
  roll_days_before?: number;
  trigger?: Trigger;
};

export type PendleNode = {
  type: "pendle";
  id: string;
  market: string;
  action: PendleAction;
  input_token?: string;
  trigger?: Trigger;
};

export type MovementNode = {
  type: "movement";
  id: string;
  movement_type: MovementType;
  provider: MovementProvider;
  from_token: string;
  to_token: string;
  from_chain?: Chain;
  to_chain?: Chain;
  trigger?: Trigger;
};

export type OptimizerNode = {
  type: "optimizer";
  id: string;
  strategy: OptimizerStrategy;
  kelly_fraction: number;
  max_allocation?: number;
  drift_threshold: number;
  allocations: VenueAllocation[];
  trigger?: Trigger;
};

export type DefiNode =
  | WalletNode
  | PerpNode
  | SpotNode
  | LendingNode
  | VaultNode
  | LpNode
  | OptionsNode
  | PendleNode
  | MovementNode
  | OptimizerNode;

export type DefiNodeType = DefiNode["type"];

// ── Workflow ─────────────────────────────────────────────────────────

export type DefiFlowWorkflow = {
  name: string;
  description?: string;
  tokens?: Record<string, Record<string, string>>;
  contracts?: Record<string, Record<string, string>>;
  reserve?: unknown;
  nodes: DefiNode[];
  edges: DefiEdge[];
};

// ── Helpers ──────────────────────────────────────────────────────────

export function getNodeId(node: DefiNode): string {
  return node.id;
}

export function getNodeLabel(node: DefiNode): string {
  switch (node.type) {
    case "wallet":
      return `${node.token} @ ${node.chain.name}`;
    case "perp":
      return `${node.venue} ${node.action} ${node.pair}`;
    case "spot":
      return `${node.venue} ${node.side} ${node.pair}`;
    case "lending":
      return `${node.archetype} ${node.action} ${node.asset}`;
    case "vault":
      return `${node.archetype} ${node.action} ${node.asset}`;
    case "lp":
      return `${node.venue} ${node.action} ${node.pool}`;
    case "options":
      return `${node.venue} ${node.action} ${node.asset}`;
    case "pendle":
      return `${node.action} ${node.market}`;
    case "movement":
      return `${node.movement_type} ${node.from_token}→${node.to_token}`;
    case "optimizer":
      return `Kelly ${(node.kelly_fraction * 100).toFixed(0)}%`;
  }
}

/** Create a default node for a given type */
export function createDefaultNode(type: DefiNodeType, id: string): DefiNode {
  const hyperevm: Chain = KNOWN_CHAINS[0];
  switch (type) {
    case "wallet":
      return { type: "wallet", id, chain: hyperevm, token: "USDC", address: "" };
    case "perp":
      return { type: "perp", id, venue: "Hyperliquid", pair: "ETH/USDC", action: "open", direction: "short", leverage: 1.0 };
    case "spot":
      return { type: "spot", id, venue: "Hyperliquid", pair: "ETH/USDC", side: "buy" };
    case "lending":
      return { type: "lending", id, archetype: "aave_v3", chain: hyperevm, pool_address: "", asset: "USDC", action: "supply" };
    case "vault":
      return { type: "vault", id, archetype: "morpho_v2", chain: { name: "ethereum", chain_id: 1, rpc_url: "https://eth.llamarpc.com" }, vault_address: "", asset: "USDC", action: "deposit" };
    case "lp":
      return { type: "lp", id, venue: "Aerodrome", pool: "WETH/USDC", action: "add_liquidity" };
    case "options":
      return { type: "options", id, venue: "Rysk", asset: "ETH", action: "sell_covered_call", delta_target: 0.3, days_to_expiry: 30 };
    case "pendle":
      return { type: "pendle", id, market: "PT-kHYPE", action: "mint_pt", input_token: "HYPE" };
    case "movement":
      return { type: "movement", id, movement_type: "bridge", provider: "LiFi", from_token: "USDC", to_token: "USDC", from_chain: { name: "base", chain_id: 8453, rpc_url: "https://mainnet.base.org" }, to_chain: hyperevm };
    case "optimizer":
      return { type: "optimizer", id, strategy: "kelly", kelly_fraction: 0.5, drift_threshold: 0.05, allocations: [] };
  }
}

/** Infer the output token of a node (what it sends downstream). */
export function getOutputToken(node: DefiNode): string {
  switch (node.type) {
    case "wallet":
      return node.token;
    case "perp":
      // Perp outputs margin (quote currency): ETH/USDC → USDC
      return node.pair.split("/")[1] ?? "USDC";
    case "spot":
      // Buy ETH/USDC → outputs ETH. Sell ETH/USDC → outputs USDC.
      return node.side === "buy"
        ? node.pair.split("/")[0] ?? "ETH"
        : node.pair.split("/")[1] ?? "USDC";
    case "lending":
      return node.asset;
    case "vault":
      return node.asset;
    case "lp":
      // LP outputs the base token of the pool
      return node.pool.split("/")[0]?.replace("W", "") ?? "ETH";
    case "movement":
      return node.to_token;
    case "optimizer":
      return "USDC";
    case "options":
      return "USDC";
    case "pendle":
      return node.input_token ?? "USDC";
  }
}

/** Infer the input token of a node (what it expects from upstream). */
export function getInputToken(node: DefiNode): string {
  switch (node.type) {
    case "wallet":
      return node.token;
    case "perp":
      // Perp takes margin (quote currency): ETH/USDC → USDC
      return node.margin_token ?? node.pair.split("/")[1] ?? "USDC";
    case "spot":
      // Buy ETH/USDC → needs USDC. Sell ETH/USDC → needs ETH.
      return node.side === "buy"
        ? node.pair.split("/")[1] ?? "USDC"
        : node.pair.split("/")[0] ?? "ETH";
    case "lending":
      return node.asset;
    case "vault":
      return node.asset;
    case "lp":
      return node.pool.split("/")[1] ?? "USDC";
    case "movement":
      return node.from_token;
    case "optimizer":
      return "USDC";
    case "options":
      return "USDC";
    case "pendle":
      return node.input_token ?? "USDC";
  }
}

/** Infer the edge token between two nodes. */
export function inferEdgeToken(from: DefiNode, to: DefiNode): string {
  const out = getOutputToken(from);
  const inp = getInputToken(to);
  // If output matches input, use it
  if (out === inp) return out;
  // Prefer the source's output token
  return out;
}

/** Allowed fields per node type — prevents extra fields leaking into JSON */
const NODE_FIELDS: Record<string, string[]> = {
  wallet: ["type", "id", "chain", "token", "address"],
  perp: ["type", "id", "venue", "pair", "action", "direction", "leverage", "margin_token", "trigger"],
  spot: ["type", "id", "venue", "pair", "side", "trigger"],
  lending: ["type", "id", "archetype", "chain", "pool_address", "asset", "action", "rewards_controller", "defillama_slug", "trigger"],
  vault: ["type", "id", "archetype", "chain", "vault_address", "asset", "action", "defillama_slug", "trigger"],
  lp: ["type", "id", "venue", "pool", "action", "tick_lower", "tick_upper", "tick_spacing", "chain", "trigger"],
  options: ["type", "id", "venue", "asset", "action", "delta_target", "days_to_expiry", "min_apy", "batch_size", "roll_days_before", "trigger"],
  pendle: ["type", "id", "market", "action", "input_token", "trigger"],
  movement: ["type", "id", "movement_type", "provider", "from_token", "to_token", "from_chain", "to_chain", "trigger"],
  optimizer: ["type", "id", "strategy", "kelly_fraction", "max_allocation", "drift_threshold", "allocations", "trigger"],
};

/** Clean a Chain object — strip undefined/null chain_id and rpc_url for namespace-only chains */
function cleanChain(chain: Chain): Chain {
  const result: Record<string, unknown> = { name: chain.name };
  if (chain.chain_id != null) result.chain_id = chain.chain_id;
  if (chain.rpc_url != null && chain.rpc_url !== "") result.rpc_url = chain.rpc_url;
  return result as Chain;
}

/** Strip fields that don't belong on this node type + remove undefined/null */
export function cleanNodeForExport(node: DefiNode): DefiNode {
  const allowed = new Set(NODE_FIELDS[node.type] ?? []);
  const cleaned: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(node)) {
    if (!allowed.has(k)) continue;
    if (v === undefined || v === null || v === "") continue;
    if (Array.isArray(v) && v.length === 0 && k !== "allocations") continue;
    // Clean nested Chain objects
    if ((k === "chain" || k === "from_chain" || k === "to_chain") && v && typeof v === "object" && "name" in v) {
      cleaned[k] = cleanChain(v as Chain);
      continue;
    }
    cleaned[k] = v;
  }
  return cleaned as DefiNode;
}
