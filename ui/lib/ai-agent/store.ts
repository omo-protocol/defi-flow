import { atom } from "jotai";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";

export type ToolActivity = {
  id: string;
  name: string;
  args: string;
  status: "running" | "done" | "error";
};

export type Message = {
  role: "user" | "assistant";
  content: string;
  /** Reasoning/thinking tokens (from <think> blocks) */
  thinking?: string;
  /** If the assistant generated a workflow, store it here */
  workflow?: DefiFlowWorkflow;
  /** Validation errors for this message's workflow (if any) */
  validationErrors?: string[];
  /** Tool calls made during this assistant turn */
  toolActivities?: ToolActivity[];
};

// In-memory only — never persisted to localStorage
export const openaiKeyAtom = atom<string>("");
export const openaiBaseUrlAtom = atom<string>("https://api.openai.com/v1");
export const openaiModelAtom = atom<string>("gpt-4o");

// Chat state
export const messagesAtom = atom<Message[]>([]);
export const generatingAtom = atom<boolean>(false);

// ── Prompt Templates ─────────────────────────────────────────────────

export type PromptTemplate = {
  id: string;
  label: string;
  description: string;
  icon: string;
  message: string;
};

export const PROMPT_TEMPLATES: PromptTemplate[] = [
  {
    id: "dn-eth-kelly",
    label: "Delta-Neutral ETH",
    description: "Spot + perp hedge with Kelly optimizer & lending",
    icon: "\u2696\uFE0F",
    message: `Import the following workflow using import_workflow, then auto_layout, validate, and backtest with capital=10000 and monte_carlo=100. Show me the results.

\`\`\`json
${JSON.stringify({
  name: "ETH Delta-Neutral Yield Farm v2",
  description: "Delta-neutral: short ETH perp (funding) + long ETH spot, bridged to HyperLend for ETH lending yield + idle USDC lending. Kelly splits between DN pair and USDC lending. Weekly rebalance on 5% drift.",
  tokens: { USDC: { hyperevm: "0xb88339CB7199b77E23DB6E890353E22632Ba630f" }, ETH: { hyperevm: "0xbe6727b535545c67d5caa73dea54865b92cf7907" } },
  contracts: { hyperlend_pool: { hyperevm: "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b" }, hyperlend_rewards: { hyperevm: "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F" } },
  nodes: [
    { type: "wallet", id: "wallet", chain: { name: "hyperliquid", chain_id: 1337 }, token: "USDC", address: "0x0000000000000000000000000000000000000000" },
    { type: "optimizer", id: "kelly", strategy: "kelly", kelly_fraction: 0.5, max_allocation: 1.0, drift_threshold: 0.05, allocations: [{ target_nodes: ["buy_eth", "short_eth"], correlation: 0.0 }, { target_node: "lend_usdc", correlation: 0.0 }], trigger: { type: "cron", interval: "weekly" } },
    { type: "spot", id: "buy_eth", venue: "Hyperliquid", pair: "ETH/USDC", side: "buy" },
    { type: "perp", id: "short_eth", venue: "Hyperliquid", pair: "ETH/USDC", action: "open", direction: "short", leverage: 1.0 },
    { type: "movement", id: "bridge_usdc", movement_type: "bridge", provider: "HyperliquidNative", from_token: "USDC", to_token: "USDC", from_chain: { name: "hyperliquid", chain_id: 1337 }, to_chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" } },
    { type: "movement", id: "bridge_eth", movement_type: "bridge", provider: "HyperliquidNative", from_token: "ETH", to_token: "ETH", from_chain: { name: "hyperliquid", chain_id: 1337 }, to_chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" } },
    { type: "lending", id: "lend_eth", archetype: "aave_v3", chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" }, pool_address: "hyperlend_pool", asset: "ETH", action: "supply", rewards_controller: "hyperlend_rewards", defillama_slug: "hyperlend-pooled" },
    { type: "lending", id: "lend_usdc", archetype: "aave_v3", chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" }, pool_address: "hyperlend_pool", asset: "USDC", action: "supply", rewards_controller: "hyperlend_rewards", defillama_slug: "hyperlend-pooled" }
  ],
  edges: [
    { from_node: "wallet", to_node: "kelly", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "buy_eth", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "short_eth", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "bridge_usdc", token: "USDC", amount: { type: "all" } },
    { from_node: "bridge_usdc", to_node: "lend_usdc", token: "USDC", amount: { type: "all" } },
    { from_node: "buy_eth", to_node: "bridge_eth", token: "ETH", amount: { type: "all" } },
    { from_node: "bridge_eth", to_node: "lend_eth", token: "ETH", amount: { type: "all" } }
  ]
}, null, 2)}
\`\`\``,
  },
  {
    id: "dn-btc-kelly",
    label: "Delta-Neutral BTC",
    description: "Spot + perp hedge with Kelly optimizer & idle USDC lending",
    icon: "\uD83D\uDFE0",
    message: `Import the following workflow using import_workflow, then auto_layout, validate, and backtest with capital=10000 and monte_carlo=100. Show me the results.

\`\`\`json
${JSON.stringify({
  name: "BTC Delta-Neutral Funding Arb",
  description: "Delta-neutral: long BTC spot + short BTC perp on Hyperliquid. Kelly splits between DN pair and idle USDC lending on HyperLend. Weekly rebalance on 5% drift.",
  tokens: { USDC: { hyperevm: "0xb88339CB7199b77E23DB6E890353E22632Ba630f" } },
  contracts: { hyperlend_pool: { hyperevm: "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b" }, hyperlend_rewards: { hyperevm: "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F" } },
  nodes: [
    { type: "wallet", id: "wallet", chain: { name: "hyperliquid", chain_id: 1337 }, token: "USDC", address: "0x0000000000000000000000000000000000000000" },
    { type: "optimizer", id: "kelly", strategy: "kelly", kelly_fraction: 0.5, max_allocation: 1.0, drift_threshold: 0.05, allocations: [{ target_nodes: ["buy_btc", "short_btc"], correlation: 0.0 }, { target_node: "lend_usdc", correlation: 0.0 }], trigger: { type: "cron", interval: "weekly" } },
    { type: "spot", id: "buy_btc", venue: "Hyperliquid", pair: "BTC/USDC", side: "buy" },
    { type: "perp", id: "short_btc", venue: "Hyperliquid", pair: "BTC/USDC", action: "open", direction: "short", leverage: 1.0 },
    { type: "movement", id: "bridge_usdc", movement_type: "bridge", provider: "HyperliquidNative", from_token: "USDC", to_token: "USDC", from_chain: { name: "hyperliquid", chain_id: 1337 }, to_chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" } },
    { type: "lending", id: "lend_usdc", archetype: "aave_v3", chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" }, pool_address: "hyperlend_pool", asset: "USDC", action: "supply", rewards_controller: "hyperlend_rewards", defillama_slug: "hyperlend-pooled" }
  ],
  edges: [
    { from_node: "wallet", to_node: "kelly", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "buy_btc", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "short_btc", token: "USDC", amount: { type: "all" } },
    { from_node: "kelly", to_node: "bridge_usdc", token: "USDC", amount: { type: "all" } },
    { from_node: "bridge_usdc", to_node: "lend_usdc", token: "USDC", amount: { type: "all" } }
  ]
}, null, 2)}
\`\`\``,
  },
  {
    id: "supply-earn",
    label: "Supply & Earn",
    description: "Simple USDC lending on HyperLend",
    icon: "\uD83C\uDFE6",
    message: `Import the following workflow using import_workflow, then auto_layout, validate, and backtest with capital=10000 and monte_carlo=100. Show me the results.

\`\`\`json
${JSON.stringify({
  name: "USDC Lending — HyperLend",
  description: "Supply USDC to HyperLend on HyperEVM for variable supply APY. No leverage, single-leg lending.",
  tokens: { USDC: { hyperevm: "0xb88339CB7199b77E23DB6E890353E22632Ba630f" } },
  contracts: { hyperlend_pool: { hyperevm: "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b" }, hyperlend_rewards: { hyperevm: "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F" } },
  nodes: [
    { type: "wallet", id: "wallet", chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" }, token: "USDC", address: "0x0000000000000000000000000000000000000000" },
    { type: "lending", id: "lend_usdc", archetype: "aave_v3", chain: { name: "hyperevm", chain_id: 999, rpc_url: "https://rpc.hyperliquid.xyz/evm" }, pool_address: "hyperlend_pool", asset: "USDC", action: "supply", rewards_controller: "hyperlend_rewards", defillama_slug: "hyperlend-pooled" }
  ],
  edges: [
    { from_node: "wallet", to_node: "lend_usdc", token: "USDC", amount: { type: "all" } }
  ]
}, null, 2)}
\`\`\``,
  },
  {
    id: "funding-harvest",
    label: "Funding Harvest",
    description: "ETH perp short to collect funding rates",
    icon: "\uD83D\uDCB8",
    message: `Import the following workflow using import_workflow, then auto_layout, validate, and backtest with capital=10000 and monte_carlo=100. Show me the results.

\`\`\`json
${JSON.stringify({
  name: "ETH Funding Rate Harvest",
  description: "Short ETH perp at 1x on Hyperliquid to collect funding. Directional bet that funding stays positive. No spot hedge.",
  tokens: {},
  contracts: {},
  nodes: [
    { type: "wallet", id: "wallet", chain: { name: "hyperliquid", chain_id: 1337 }, token: "USDC", address: "0x0000000000000000000000000000000000000000" },
    { type: "perp", id: "short_eth", venue: "Hyperliquid", pair: "ETH/USDC", action: "open", direction: "short", leverage: 1.0 }
  ],
  edges: [
    { from_node: "wallet", to_node: "short_eth", token: "USDC", amount: { type: "all" } }
  ]
}, null, 2)}
\`\`\``,
  },
  {
    id: "custom",
    label: "Custom Strategy",
    description: "Describe your own strategy from scratch",
    icon: "\uD83E\uDDE9",
    message: "",
  },
];
