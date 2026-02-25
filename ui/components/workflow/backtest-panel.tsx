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
import { convertCanvasToDefiFlow } from "@/lib/converters/canvas-defi-flow";
import {
  runBacktest,
  listBacktests,
  type BacktestResult,
  type BacktestSummary,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Play, History, TrendingUp, TrendingDown } from "lucide-react";
import { toast } from "sonner";

export function BacktestPanel() {
  const nodes = useAtomValue(nodesAtom);
  const edges = useAtomValue(edgesAtom);
  const name = useAtomValue(workflowNameAtom);
  const tokens = useAtomValue(tokensManifestAtom);
  const contracts = useAtomValue(contractsManifestAtom);

  const [capital, setCapital] = useState("10000");
  const [slippage, setSlippage] = useState("10");
  const [seed, setSeed] = useState("42");
  const [dataDir, setDataDir] = useState("");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<BacktestResult | null>(null);
  const [history, setHistory] = useState<BacktestSummary[]>([]);
  const [showHistory, setShowHistory] = useState(false);

  useEffect(() => {
    listBacktests().then(setHistory).catch(() => {});
  }, [result]);

  const handleRun = async () => {
    if (nodes.length === 0) {
      toast.error("No nodes to backtest");
      return;
    }

    setRunning(true);
    setResult(null);
    try {
      const workflow = convertCanvasToDefiFlow(
        nodes,
        edges,
        name,
        undefined,
        tokens,
        contracts
      );
      const res = await runBacktest(workflow, {
        capital: parseFloat(capital),
        slippage_bps: parseFloat(slippage),
        seed: parseInt(seed),
        ...(dataDir ? { data_dir: dataDir } : {}),
      });
      setResult(res.result);
      toast.success("Backtest complete");
    } catch (err) {
      toast.error(
        "Backtest failed: " +
          (err instanceof Error ? err.message : "Unknown error")
      );
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="p-4 space-y-4 text-sm">
      <h3 className="font-semibold text-base">Backtest</h3>

      {/* Config form */}
      <div className="space-y-2">
        <div>
          <Label className="text-xs text-muted-foreground">Data Directory</Label>
          <Input
            className="h-7 text-xs font-mono"
            value={dataDir}
            onChange={(e) => setDataDir(e.target.value)}
            placeholder="data/delta_neutral_v2"
          />
        </div>
        <div className="grid grid-cols-3 gap-2">
          <div>
            <Label className="text-xs text-muted-foreground">Capital ($)</Label>
            <Input
              className="h-7 text-xs"
              value={capital}
              onChange={(e) => setCapital(e.target.value)}
            />
          </div>
          <div>
            <Label className="text-xs text-muted-foreground">Slippage (bps)</Label>
            <Input
              className="h-7 text-xs"
              value={slippage}
              onChange={(e) => setSlippage(e.target.value)}
            />
          </div>
          <div>
            <Label className="text-xs text-muted-foreground">Seed</Label>
            <Input
              className="h-7 text-xs"
              value={seed}
              onChange={(e) => setSeed(e.target.value)}
            />
          </div>
        </div>
      </div>

      <div className="flex gap-2">
        <Button
          onClick={handleRun}
          disabled={running || nodes.length === 0}
          size="sm"
          className="flex-1"
        >
          <Play className="w-3.5 h-3.5 mr-1" />
          {running ? "Running..." : "Run Backtest"}
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setShowHistory(!showHistory)}
        >
          <History className="w-3.5 h-3.5" />
        </Button>
      </div>

      {/* Results */}
      {result && (
        <>
          <Separator />
          <div className="space-y-2">
            <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground">
              Results
            </h4>
            <div className="grid grid-cols-2 gap-x-4 gap-y-1">
              <MetricRow
                label="TWRR"
                value={`${result.twrr_pct >= 0 ? "+" : ""}${result.twrr_pct.toFixed(2)}%`}
                positive={result.twrr_pct >= 0}
              />
              <MetricRow
                label="Annualized"
                value={`${result.annualized_pct >= 0 ? "+" : ""}${result.annualized_pct.toFixed(2)}%`}
                positive={result.annualized_pct >= 0}
              />
              <MetricRow
                label="Sharpe"
                value={result.sharpe.toFixed(3)}
                positive={result.sharpe >= 0}
              />
              <MetricRow
                label="Max DD"
                value={`${result.max_drawdown_pct.toFixed(2)}%`}
                positive={false}
              />
              <MetricRow
                label="Net PnL"
                value={`$${result.net_pnl.toFixed(2)}`}
                positive={result.net_pnl >= 0}
              />
              <MetricRow label="Ticks" value={String(result.ticks)} />
              <MetricRow label="Rebalances" value={String(result.rebalances)} />
              <MetricRow label="Liquidations" value={String(result.liquidations)} />
            </div>

            <Separator />

            <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground">
              PnL Breakdown
            </h4>
            <div className="grid grid-cols-2 gap-x-4 gap-y-1">
              <MetricRow
                label="Funding"
                value={`$${result.funding_pnl.toFixed(2)}`}
                positive={result.funding_pnl >= 0}
              />
              <MetricRow
                label="Rewards"
                value={`$${result.rewards_pnl.toFixed(2)}`}
                positive={result.rewards_pnl >= 0}
              />
              <MetricRow
                label="LP Fees"
                value={`$${result.lp_fees.toFixed(2)}`}
                positive={result.lp_fees >= 0}
              />
              <MetricRow
                label="Lending"
                value={`$${result.lending_interest.toFixed(2)}`}
                positive={result.lending_interest >= 0}
              />
              <MetricRow
                label="Swap Costs"
                value={`-$${Math.abs(result.swap_costs).toFixed(2)}`}
                positive={false}
              />
            </div>

            {/* Mini trajectory chart */}
            {result.trajectory.length > 1 && (
              <>
                <Separator />
                <TrajectoryChart trajectory={result.trajectory} capital={parseFloat(capital)} />
              </>
            )}
          </div>
        </>
      )}

      {/* History */}
      {showHistory && history.length > 0 && (
        <>
          <Separator />
          <h4 className="font-medium text-xs uppercase tracking-wider text-muted-foreground">
            History
          </h4>
          <div className="space-y-1 max-h-48 overflow-y-auto">
            {history.map((h) => (
              <div
                key={h.id}
                className="flex items-center justify-between text-xs px-2 py-1.5 rounded hover:bg-accent"
              >
                <span className="truncate flex-1">{h.label}</span>
                <span
                  className={
                    h.twrr_pct >= 0
                      ? "text-emerald-500"
                      : "text-red-500"
                  }
                >
                  {h.twrr_pct >= 0 ? "+" : ""}
                  {h.twrr_pct.toFixed(2)}%
                </span>
                <span className="ml-2 text-muted-foreground">
                  S:{h.sharpe.toFixed(2)}
                </span>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function MetricRow({
  label,
  value,
  positive,
}: {
  label: string;
  value: string;
  positive?: boolean;
}) {
  return (
    <div className="flex justify-between items-center">
      <span className="text-muted-foreground text-xs">{label}</span>
      <span
        className={`text-xs font-mono ${
          positive === true
            ? "text-emerald-500"
            : positive === false
              ? "text-red-500"
              : ""
        }`}
      >
        {value}
      </span>
    </div>
  );
}

function TrajectoryChart({
  trajectory,
  capital,
}: {
  trajectory: { timestamp: number; tvl: number }[];
  capital: number;
}) {
  const values = trajectory.map((p) => p.tvl);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;

  const w = 280;
  const h = 80;
  const points = trajectory
    .map((p, i) => {
      const x = (i / (trajectory.length - 1)) * w;
      const y = h - ((p.tvl - min) / range) * h;
      return `${x},${y}`;
    })
    .join(" ");

  const finalTvl = values[values.length - 1];
  const color = finalTvl >= capital ? "#10b981" : "#ef4444";

  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <span className="text-xs text-muted-foreground">TVL Trajectory</span>
        <span className="text-xs font-mono" style={{ color }}>
          ${finalTvl.toFixed(0)}
        </span>
      </div>
      <svg viewBox={`0 0 ${w} ${h}`} className="w-full h-20">
        {/* Baseline */}
        <line
          x1="0"
          y1={h - ((capital - min) / range) * h}
          x2={w}
          y2={h - ((capital - min) / range) * h}
          stroke="currentColor"
          strokeOpacity="0.15"
          strokeDasharray="4 2"
        />
        <polyline
          fill="none"
          stroke={color}
          strokeWidth="1.5"
          points={points}
        />
      </svg>
    </div>
  );
}
