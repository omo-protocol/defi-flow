"use client";

import { useState, useCallback } from "react";
import type { CanvasNode } from "@/lib/types/canvas";
import type {
  WalletNode, PerpNode, SpotNode, LendingNode, VaultNode,
  LpNode, OptionsNode, PendleNode, MovementNode, OptimizerNode,
  DefiNode, VenueAllocation,
} from "@/lib/types/defi-flow";
import { getNodeLabel } from "@/lib/types/defi-flow";
import { useSetAtom, useAtomValue } from "jotai";
import { updateNodeDataAtom, walletAddressAtom, edgesAtom, nodesAtom } from "@/lib/workflow-store";
import { walletsAtom } from "@/lib/auth-store";
import { TextField, NumberField, SelectField, ChainSelect, TriggerConfig } from "./shared";

// Generic updater hook
function useNodeUpdater(nodeId: string) {
  const update = useSetAtom(updateNodeDataAtom);
  return (partial: Partial<DefiNode>) => {
    // We merge partial into the existing defiNode
    update({
      id: nodeId,
      data: {
        defiNode: partial as any, // merged in parent
      },
    });
  };
}

type ConfigProps<T extends DefiNode> = {
  node: CanvasNode;
  defi: T;
  onUpdate: (field: string, value: unknown) => void;
};

// ── Wallet ───────────────────────────────────────────────────────────

function WalletConfig({ node, defi, onUpdate }: ConfigProps<WalletNode>) {
  const walletAddr = useAtomValue(walletAddressAtom);
  const wallets = useAtomValue(walletsAtom);

  const currentAddr = defi.address || walletAddr;
  const walletOptions = [
    ...wallets.map((w) => ({
      value: w.address,
      label: `${w.label}  ${w.address.slice(0, 6)}...${w.address.slice(-4)}`,
    })),
    { value: "__custom__", label: "Custom address" },
  ];

  const isCustom = wallets.length === 0 || !wallets.some((w) => w.address === currentAddr);

  return (
    <div className="space-y-3">
      <ChainSelect
        value={defi.chain}
        onChange={(chain) => onUpdate("chain", chain)}
      />
      <TextField
        label="Token"
        value={defi.token}
        onChange={(v) => onUpdate("token", v)}
        placeholder="USDC"
      />
      {wallets.length > 0 && (
        <SelectField
          label="Wallet"
          value={isCustom ? "__custom__" : currentAddr}
          onChange={(v) => {
            if (v !== "__custom__") onUpdate("address", v);
          }}
          options={walletOptions}
        />
      )}
      {(isCustom || wallets.length === 0) && (
        <TextField
          label="Address"
          value={currentAddr}
          onChange={(v) => onUpdate("address", v)}
          placeholder="0x..."
        />
      )}
    </div>
  );
}

// ── Perp ─────────────────────────────────────────────────────────────

function PerpConfig({ node, defi, onUpdate }: ConfigProps<PerpNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Venue"
        value={defi.venue}
        onChange={(v) => onUpdate("venue", v)}
        options={[
          { value: "Hyperliquid", label: "Hyperliquid" },
          { value: "Hyena", label: "Hyena (HIP-3)" },
        ]}
      />
      <TextField
        label="Pair"
        value={defi.pair}
        onChange={(v) => onUpdate("pair", v)}
        placeholder="ETH/USDC"
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "open", label: "Open Position" },
          { value: "close", label: "Close Position" },
          { value: "adjust", label: "Adjust Position" },
          { value: "collect_funding", label: "Collect Funding" },
        ]}
      />
      {(defi.action === "open" || defi.action === "adjust") && (
        <>
          <SelectField
            label="Direction"
            value={defi.direction ?? "short"}
            onChange={(v) => onUpdate("direction", v)}
            options={[
              { value: "long", label: "Long" },
              { value: "short", label: "Short" },
            ]}
          />
          <NumberField
            label="Leverage"
            value={defi.leverage}
            onChange={(v) => onUpdate("leverage", v)}
            placeholder="1.0"
            step={0.1}
            min={0.1}
          />
        </>
      )}
      <TextField
        label="Margin Token (optional)"
        value={defi.margin_token ?? ""}
        onChange={(v) => onUpdate("margin_token", v || undefined)}
        placeholder="USDC (default)"
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Spot ─────────────────────────────────────────────────────────────

function SpotConfig({ node, defi, onUpdate }: ConfigProps<SpotNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Venue"
        value={defi.venue}
        onChange={(v) => onUpdate("venue", v)}
        options={[{ value: "Hyperliquid", label: "Hyperliquid" }]}
      />
      <TextField
        label="Pair"
        value={defi.pair}
        onChange={(v) => onUpdate("pair", v)}
        placeholder="ETH/USDC"
      />
      <SelectField
        label="Side"
        value={defi.side}
        onChange={(v) => onUpdate("side", v)}
        options={[
          { value: "buy", label: "Buy" },
          { value: "sell", label: "Sell" },
        ]}
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Lending ──────────────────────────────────────────────────────────

function LendingConfig({ node, defi, onUpdate }: ConfigProps<LendingNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Archetype"
        value={defi.archetype}
        onChange={(v) => onUpdate("archetype", v)}
        options={[
          { value: "aave_v3", label: "Aave V3" },
          { value: "aave_v2", label: "Aave V2" },
          { value: "morpho", label: "Morpho" },
          { value: "compound_v3", label: "Compound V3" },
          { value: "init_capital", label: "Init Capital" },
        ]}
      />
      <ChainSelect value={defi.chain} onChange={(c) => onUpdate("chain", c)} />
      <TextField
        label="Pool Address (manifest key)"
        value={defi.pool_address}
        onChange={(v) => onUpdate("pool_address", v)}
        placeholder="hyperlend_pool"
      />
      <TextField
        label="Asset"
        value={defi.asset}
        onChange={(v) => onUpdate("asset", v)}
        placeholder="USDC"
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "supply", label: "Supply" },
          { value: "withdraw", label: "Withdraw" },
          { value: "borrow", label: "Borrow" },
          { value: "repay", label: "Repay" },
          { value: "claim_rewards", label: "Claim Rewards" },
        ]}
      />
      <TextField
        label="Rewards Controller (optional)"
        value={defi.rewards_controller ?? ""}
        onChange={(v) => onUpdate("rewards_controller", v || undefined)}
        placeholder="hyperlend_rewards"
      />
      <TextField
        label="DefiLlama Slug (optional)"
        value={defi.defillama_slug ?? ""}
        onChange={(v) => onUpdate("defillama_slug", v || undefined)}
        placeholder="hyperlend-pooled"
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Vault ────────────────────────────────────────────────────────────

function VaultConfig({ node, defi, onUpdate }: ConfigProps<VaultNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Archetype"
        value={defi.archetype}
        onChange={(v) => onUpdate("archetype", v)}
        options={[{ value: "morpho_v2", label: "Morpho V2" }]}
      />
      <ChainSelect value={defi.chain} onChange={(c) => onUpdate("chain", c)} />
      <TextField
        label="Vault Address (manifest key)"
        value={defi.vault_address}
        onChange={(v) => onUpdate("vault_address", v)}
        placeholder="morpho_usdc_vault"
      />
      <TextField
        label="Asset"
        value={defi.asset}
        onChange={(v) => onUpdate("asset", v)}
        placeholder="USDC"
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "deposit", label: "Deposit" },
          { value: "withdraw", label: "Withdraw" },
          { value: "claim_rewards", label: "Claim Rewards" },
        ]}
      />
      <TextField
        label="DefiLlama Slug (optional)"
        value={defi.defillama_slug ?? ""}
        onChange={(v) => onUpdate("defillama_slug", v || undefined)}
        placeholder="morpho-vaults-v2"
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── LP ───────────────────────────────────────────────────────────────

function LpConfig({ node, defi, onUpdate }: ConfigProps<LpNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Venue"
        value={defi.venue}
        onChange={(v) => onUpdate("venue", v)}
        options={[{ value: "Aerodrome", label: "Aerodrome" }]}
      />
      <TextField
        label="Pool"
        value={defi.pool}
        onChange={(v) => onUpdate("pool", v)}
        placeholder="WETH/USDC"
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "add_liquidity", label: "Add Liquidity" },
          { value: "remove_liquidity", label: "Remove Liquidity" },
          { value: "claim_rewards", label: "Claim Rewards" },
          { value: "compound", label: "Compound" },
          { value: "stake_gauge", label: "Stake Gauge" },
          { value: "unstake_gauge", label: "Unstake Gauge" },
        ]}
      />
      <NumberField
        label="Tick Lower (optional)"
        value={defi.tick_lower}
        onChange={(v) => onUpdate("tick_lower", v)}
        placeholder="Full range"
      />
      <NumberField
        label="Tick Upper (optional)"
        value={defi.tick_upper}
        onChange={(v) => onUpdate("tick_upper", v)}
        placeholder="Full range"
      />
      <NumberField
        label="Tick Spacing (optional)"
        value={defi.tick_spacing}
        onChange={(v) => onUpdate("tick_spacing", v)}
        placeholder="100"
      />
      <ChainSelect
        value={defi.chain}
        onChange={(c) => onUpdate("chain", c)}
        label="Chain (default: Base)"
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Options ──────────────────────────────────────────────────────────

function OptionsConfig({ node, defi, onUpdate }: ConfigProps<OptionsNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Venue"
        value={defi.venue}
        onChange={(v) => onUpdate("venue", v)}
        options={[{ value: "Rysk", label: "Rysk" }]}
      />
      <SelectField
        label="Asset"
        value={defi.asset}
        onChange={(v) => onUpdate("asset", v)}
        options={[
          { value: "ETH", label: "ETH" },
          { value: "BTC", label: "BTC" },
          { value: "HYPE", label: "HYPE" },
          { value: "SOL", label: "SOL" },
        ]}
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "sell_covered_call", label: "Sell Covered Call" },
          { value: "sell_cash_secured_put", label: "Sell Cash-Secured Put" },
          { value: "buy_call", label: "Buy Call" },
          { value: "buy_put", label: "Buy Put" },
          { value: "collect_premium", label: "Collect Premium" },
          { value: "roll", label: "Roll" },
          { value: "close", label: "Close" },
        ]}
      />
      <NumberField
        label="Delta Target (0-1)"
        value={defi.delta_target}
        onChange={(v) => onUpdate("delta_target", v)}
        placeholder="0.3"
        step={0.05}
        min={0}
        max={1}
      />
      <NumberField
        label="Days to Expiry"
        value={defi.days_to_expiry}
        onChange={(v) => onUpdate("days_to_expiry", v)}
        placeholder="30"
      />
      <NumberField
        label="Min APY"
        value={defi.min_apy}
        onChange={(v) => onUpdate("min_apy", v)}
        placeholder="0.05"
        step={0.01}
      />
      <NumberField
        label="Batch Size"
        value={defi.batch_size}
        onChange={(v) => onUpdate("batch_size", v)}
        placeholder="10"
      />
      <NumberField
        label="Roll Days Before"
        value={defi.roll_days_before}
        onChange={(v) => onUpdate("roll_days_before", v)}
        placeholder="3"
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Pendle ───────────────────────────────────────────────────────────

function PendleConfig({ node, defi, onUpdate }: ConfigProps<PendleNode>) {
  return (
    <div className="space-y-3">
      <TextField
        label="Market"
        value={defi.market}
        onChange={(v) => onUpdate("market", v)}
        placeholder="PT-kHYPE"
      />
      <SelectField
        label="Action"
        value={defi.action}
        onChange={(v) => onUpdate("action", v)}
        options={[
          { value: "mint_pt", label: "Mint PT" },
          { value: "redeem_pt", label: "Redeem PT" },
          { value: "mint_yt", label: "Mint YT" },
          { value: "redeem_yt", label: "Redeem YT" },
          { value: "claim_rewards", label: "Claim Rewards" },
        ]}
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Movement ─────────────────────────────────────────────────────────

function MovementConfig({ node, defi, onUpdate }: ConfigProps<MovementNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Type"
        value={defi.movement_type}
        onChange={(v) => onUpdate("movement_type", v)}
        options={[
          { value: "swap", label: "Swap" },
          { value: "bridge", label: "Bridge" },
          { value: "swap_bridge", label: "Swap + Bridge" },
        ]}
      />
      <SelectField
        label="Provider"
        value={defi.provider}
        onChange={(v) => onUpdate("provider", v)}
        options={[
          { value: "LiFi", label: "LiFi" },
          { value: "HyperliquidNative", label: "Hyperliquid Native" },
        ]}
      />
      <TextField
        label="From Token"
        value={defi.from_token}
        onChange={(v) => onUpdate("from_token", v)}
        placeholder="USDC"
      />
      <TextField
        label="To Token"
        value={defi.to_token}
        onChange={(v) => onUpdate("to_token", v)}
        placeholder="USDC"
      />
      {(defi.movement_type === "bridge" || defi.movement_type === "swap_bridge") && (
        <>
          <ChainSelect
            value={defi.from_chain}
            onChange={(c) => onUpdate("from_chain", c)}
            label="From Chain"
          />
          <ChainSelect
            value={defi.to_chain}
            onChange={(c) => onUpdate("to_chain", c)}
            label="To Chain"
          />
        </>
      )}
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Optimizer ────────────────────────────────────────────────────────

function AllocationsEditor({
  allocations,
  optimizerId,
  onChange,
}: {
  allocations: VenueAllocation[];
  optimizerId: string;
  onChange: (allocs: VenueAllocation[]) => void;
}) {
  const edges = useAtomValue(edgesAtom);
  const nodes = useAtomValue(nodesAtom);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Downstream node IDs from edges
  const downstreamIds = edges
    .filter((e) => e.source === optimizerId)
    .map((e) => e.target);

  // All node IDs already in an allocation
  const allocatedIds = new Set(
    allocations.flatMap((a) => a.target_nodes ?? (a.target_node ? [a.target_node] : []))
  );

  // Unallocated downstream nodes (connected but not in any allocation yet)
  const unallocated = downstreamIds.filter((id) => !allocatedIds.has(id));

  // Auto-add unallocated as single-node allocations
  const autoSync = useCallback(() => {
    if (unallocated.length === 0) return;
    const newAllocs = [
      ...allocations,
      ...unallocated.map((id) => ({
        target_node: id,
        correlation: 0,
      } as VenueAllocation)),
    ];
    onChange(newAllocs);
  }, [allocations, unallocated, onChange]);

  // Auto-sync on first render or when edges change
  if (unallocated.length > 0) {
    // Schedule to avoid render-during-render
    setTimeout(autoSync, 0);
  }

  // Remove allocations whose nodes are no longer connected
  const pruneDisconnected = useCallback(() => {
    const dsSet = new Set(downstreamIds);
    const pruned = allocations
      .map((a) => {
        if (a.target_nodes) {
          const kept = a.target_nodes.filter((id) => dsSet.has(id));
          if (kept.length === 0) return null;
          if (kept.length === 1) return { ...a, target_node: kept[0], target_nodes: undefined };
          return { ...a, target_nodes: kept };
        }
        if (a.target_node && !dsSet.has(a.target_node)) return null;
        return a;
      })
      .filter(Boolean) as VenueAllocation[];
    if (pruned.length !== allocations.length) {
      setTimeout(() => onChange(pruned), 0);
    }
  }, [allocations, downstreamIds, onChange]);

  // Check for stale entries
  const hasStale = allocations.some((a) => {
    if (a.target_nodes) return a.target_nodes.some((id) => !downstreamIds.includes(id));
    return a.target_node ? !downstreamIds.includes(a.target_node) : false;
  });
  if (hasStale) pruneDisconnected();

  const getNodeLabel = (id: string) => {
    const n = nodes.find((n) => n.id === id);
    return n?.data.label ?? id;
  };

  const getAllocNodeIds = (a: VenueAllocation): string[] =>
    a.target_nodes ?? (a.target_node ? [a.target_node] : []);

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const groupSelected = () => {
    if (selected.size < 2) return;
    const groupIds = Array.from(selected);
    // Remove selected nodes from their current allocations
    const remaining = allocations
      .map((a) => {
        const ids = getAllocNodeIds(a).filter((id) => !selected.has(id));
        if (ids.length === 0) return null;
        if (ids.length === 1) return { ...a, target_node: ids[0], target_nodes: undefined };
        return { ...a, target_nodes: ids, target_node: undefined };
      })
      .filter(Boolean) as VenueAllocation[];
    // Add the new group
    remaining.push({ target_nodes: groupIds, correlation: 0 });
    onChange(remaining);
    setSelected(new Set());
  };

  const ungroupAlloc = (idx: number) => {
    const a = allocations[idx];
    const ids = getAllocNodeIds(a);
    if (ids.length <= 1) return;
    const newAllocs = [
      ...allocations.slice(0, idx),
      ...allocations.slice(idx + 1),
      ...ids.map((id) => ({ target_node: id, correlation: 0 } as VenueAllocation)),
    ];
    onChange(newAllocs);
  };

  const updateCorrelation = (idx: number, corr: number) => {
    const newAllocs = [...allocations];
    newAllocs[idx] = { ...newAllocs[idx], correlation: corr };
    onChange(newAllocs);
  };

  const removeAlloc = (idx: number) => {
    onChange([...allocations.slice(0, idx), ...allocations.slice(idx + 1)]);
  };

  if (downstreamIds.length === 0) {
    return (
      <div className="space-y-1.5">
        <label className="text-xs font-medium">Allocations</label>
        <p className="text-[10px] text-muted-foreground">
          Connect edges from optimizer to venue nodes to create allocations.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium">
          Allocations ({allocations.length})
        </label>
        {selected.size >= 2 && (
          <button
            className="text-[10px] px-2 py-0.5 rounded bg-indigo-500/20 text-indigo-400 hover:bg-indigo-500/30 transition-colors"
            onClick={groupSelected}
          >
            Group ({selected.size})
          </button>
        )}
      </div>
      <p className="text-[10px] text-muted-foreground">
        Select nodes and click Group to create delta-neutral pairs.
      </p>

      <div className="space-y-1.5">
        {allocations.map((alloc, idx) => {
          const ids = getAllocNodeIds(alloc);
          const isGroup = ids.length > 1;
          return (
            <div
              key={idx}
              className="rounded border p-2 space-y-1.5 text-xs"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1 flex-wrap">
                  {ids.map((id) => (
                    <button
                      key={id}
                      className={`px-1.5 py-0.5 rounded text-[10px] font-mono transition-colors ${
                        selected.has(id)
                          ? "bg-indigo-500 text-white"
                          : "bg-muted hover:bg-accent"
                      }`}
                      onClick={() => toggleSelect(id)}
                    >
                      {getNodeLabel(id)}
                    </button>
                  ))}
                  {isGroup && (
                    <span className="text-[10px] text-muted-foreground ml-1">(group)</span>
                  )}
                </div>
                <div className="flex items-center gap-1">
                  {isGroup && (
                    <button
                      className="text-[10px] text-muted-foreground hover:text-foreground"
                      onClick={() => ungroupAlloc(idx)}
                      title="Ungroup"
                    >
                      split
                    </button>
                  )}
                  <button
                    className="text-[10px] text-destructive hover:text-destructive/80"
                    onClick={() => removeAlloc(idx)}
                    title="Remove"
                  >
                    x
                  </button>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-muted-foreground w-16 shrink-0">Correlation</span>
                <input
                  type="number"
                  className="h-6 w-20 rounded border bg-transparent px-1.5 text-[10px] font-mono"
                  value={alloc.correlation}
                  onChange={(e) => updateCorrelation(idx, Number(e.target.value))}
                  step={0.1}
                  min={-1}
                  max={1}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function OptimizerConfig({ node, defi, onUpdate }: ConfigProps<OptimizerNode>) {
  return (
    <div className="space-y-3">
      <SelectField
        label="Strategy"
        value={defi.strategy}
        onChange={(v) => onUpdate("strategy", v)}
        options={[{ value: "kelly", label: "Kelly Criterion" }]}
      />
      <NumberField
        label="Kelly Fraction (0-1)"
        value={defi.kelly_fraction}
        onChange={(v) => onUpdate("kelly_fraction", v ?? 0.5)}
        placeholder="0.5"
        step={0.1}
        min={0}
        max={1}
      />
      <NumberField
        label="Max Allocation (0-1)"
        value={defi.max_allocation}
        onChange={(v) => onUpdate("max_allocation", v)}
        placeholder="1.0"
        step={0.1}
        min={0}
        max={1}
      />
      <NumberField
        label="Drift Threshold"
        value={defi.drift_threshold}
        onChange={(v) => onUpdate("drift_threshold", v ?? 0)}
        placeholder="0.05"
        step={0.01}
        min={0}
        max={1}
      />
      <AllocationsEditor
        allocations={defi.allocations}
        optimizerId={defi.id}
        onChange={(allocs) => onUpdate("allocations", allocs)}
      />
      <TriggerConfig
        value={defi.trigger}
        onChange={(t) => onUpdate("trigger", t)}
      />
    </div>
  );
}

// ── Main dispatcher ──────────────────────────────────────────────────

export function NodeConfigForm({ node }: { node: CanvasNode }) {
  const updateNodeData = useSetAtom(updateNodeDataAtom);
  const defi = node.data.defiNode;

  const onUpdate = (field: string, value: unknown) => {
    const updatedDefi = { ...defi, [field]: value };
    updateNodeData({
      id: node.id,
      data: {
        defiNode: updatedDefi as DefiNode,
        label: getNodeLabel(updatedDefi as DefiNode),
      },
    });
  };

  const props = { node, onUpdate };

  switch (defi.type) {
    case "wallet": return <WalletConfig {...props} defi={defi} />;
    case "perp": return <PerpConfig {...props} defi={defi} />;
    case "spot": return <SpotConfig {...props} defi={defi} />;
    case "lending": return <LendingConfig {...props} defi={defi} />;
    case "vault": return <VaultConfig {...props} defi={defi} />;
    case "lp": return <LpConfig {...props} defi={defi} />;
    case "options": return <OptionsConfig {...props} defi={defi} />;
    case "pendle": return <PendleConfig {...props} defi={defi} />;
    case "movement": return <MovementConfig {...props} defi={defi} />;
    case "optimizer": return <OptimizerConfig {...props} defi={defi} />;
    default: return <div className="text-xs text-muted-foreground">Unknown node type</div>;
  }
}
