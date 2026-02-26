---
name: backtest
description: Backtest a DeFi strategy through the full pipeline — write JSON, validate, fetch data, run backtest with Monte Carlo. Triggers on backtest strategy, run backtest, test strategy, Monte Carlo, simulate strategy.
version: 1.0.0
metadata:
  openclaw:
    emoji: "📊"
    requires:
      bins:
        - defi-flow
        - curl
---

# Backtest a DeFi Strategy

Run the full backtest pipeline for a defi-flow strategy.

## Steps

1. **Write strategy JSON** to `strategies/<name>.json`
2. **Validate**: `defi-flow validate strategies/<name>.json` — fix errors, re-validate
3. **Fetch data**: `defi-flow fetch-data strategies/<name>.json --output-dir data/<name> --days 365 --interval 8h`
4. **Backtest**: `defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000`
5. **Report**: TWRR%, Annualized%, Max Drawdown%, Sharpe, Net PnL
6. **Monte Carlo** (optional): add `--monte-carlo 500`

## Extra Flags

- `--output results/<name>.json` — save metrics to JSON file
- `--tick-csv results/<name>_ticks.csv` — per-tick venue values
- `--verbose` — per-tick logging

## Key Rules

- All addresses in `tokens`/`contracts` manifests — nodes reference by key
- Chain names lowercase: `hyperevm`, `base`, `hyperliquid`
- Each node has its own simulator — don't split open/collect_funding
- Funding auto-compounds inside perp margin
- Re-run `fetch-data` after renaming node IDs
- Adaptive Kelly: omit `expected_return`/`volatility` in allocations — derived from venue data
- Delta-neutral: use `target_nodes: ["buy_eth", "short_eth"]` — never rebalances between legs

## Interpreting Monte Carlo

MC generates synthetic data via parametric models (GBM prices, OU funding, AR(1) yields) then re-runs backtest on each path.

**Why MC often looks worse:**
1. Funding rate distribution — mean small vs stdev → many negative paths
2. Price vol + rebalancing drag — whipsaws vs smooth historical
3. Liquidation tail risk — 1x short liquidates around 2x price
4. Adaptive Kelly with sparse data — 0% return venues get 0% allocation

Historical Sharpe >> MC Sharpe is normal. MC median is closer to reality.

**Check if MC looks bad:**
- Venues getting nonzero adaptive stats? Check `[kelly]` log lines
- Rebalance frequency vs vol? Daily > weekly for high-vol
- Set explicit `expected_return`/`volatility` if data is sparse

## Guardrails

- Never fabricate backtest results — always run the actual CLI command
- If validation fails, show errors and fix the JSON before proceeding
- If fetch-data fails, check node IDs match what fetch-data expects
- Save results to `results/` directory for persistence
