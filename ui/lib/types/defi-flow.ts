// ── Core types matching Rust defi-flow model exactly ──────────────────

export type Chain = {
  name: string;
  chain_id: number;
  rpc_url: string;
};

export const KNOWN_CHAINS: Chain[] = [
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
export type MovementProvider = "LiFi" | "Stargate";

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
      return { type: "pendle", id, market: "PT-kHYPE", action: "mint_pt" };
    case "movement":
      return { type: "movement", id, movement_type: "bridge", provider: "LiFi", from_token: "USDC", to_token: "USDC", from_chain: { name: "base", chain_id: 8453, rpc_url: "https://mainnet.base.org" }, to_chain: hyperevm };
    case "optimizer":
      return { type: "optimizer", id, strategy: "kelly", kelly_fraction: 0.5, drift_threshold: 0.05, allocations: [] };
  }
}

/** Strip undefined/null optional fields for clean JSON export */
export function cleanNodeForExport(node: DefiNode): DefiNode {
  const cleaned: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(node)) {
    if (v !== undefined && v !== null && v !== "") {
      // Skip empty arrays for target_nodes in allocations
      if (Array.isArray(v) && v.length === 0 && k !== "allocations") continue;
      cleaned[k] = v;
    }
  }
  return cleaned as DefiNode;
}
