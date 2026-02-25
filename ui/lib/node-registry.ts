import {
  Wallet,
  TrendingUp,
  ArrowLeftRight,
  Landmark,
  Vault,
  Droplets,
  BarChart3,
  Coins,
  ArrowRightLeft,
  Settings2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { DefiNodeType } from "./types/defi-flow";

export type NodeCategory = "source" | "venue" | "movement" | "optimizer";

export type NodeTypeConfig = {
  type: DefiNodeType;
  label: string;
  icon: LucideIcon;
  /** Tailwind color name (used for accents) */
  color: string;
  category: NodeCategory;
  description: string;
};

export const NODE_REGISTRY: NodeTypeConfig[] = [
  // Source
  { type: "wallet", label: "Wallet", icon: Wallet, color: "blue", category: "source", description: "Source or sink for funds on a chain" },
  // Venues
  { type: "perp", label: "Perpetuals", icon: TrendingUp, color: "purple", category: "venue", description: "Perpetual futures trading" },
  { type: "spot", label: "Spot", icon: ArrowLeftRight, color: "green", category: "venue", description: "Spot DEX trading" },
  { type: "lending", label: "Lending", icon: Landmark, color: "cyan", category: "venue", description: "Supply, borrow, and earn on lending protocols" },
  { type: "vault", label: "Vault", icon: Vault, color: "indigo", category: "venue", description: "Yield vault deposits" },
  { type: "lp", label: "LP", icon: Droplets, color: "teal", category: "venue", description: "Concentrated liquidity provision" },
  { type: "options", label: "Options", icon: BarChart3, color: "orange", category: "venue", description: "Options trading on Rysk" },
  { type: "pendle", label: "Pendle", icon: Coins, color: "pink", category: "venue", description: "Yield tokenization (PT/YT)" },
  // Movement
  { type: "movement", label: "Bridge/Swap", icon: ArrowRightLeft, color: "amber", category: "movement", description: "Cross-chain bridges and swaps" },
  // Optimizer
  { type: "optimizer", label: "Optimizer", icon: Settings2, color: "red", category: "optimizer", description: "Kelly Criterion capital allocation" },
];

export function getNodeConfig(type: DefiNodeType): NodeTypeConfig | undefined {
  return NODE_REGISTRY.find((n) => n.type === type);
}

export const CATEGORIES: { key: NodeCategory; label: string }[] = [
  { key: "source", label: "Source" },
  { key: "venue", label: "Venues" },
  { key: "movement", label: "Movement" },
  { key: "optimizer", label: "Optimizer" },
];
