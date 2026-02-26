# Backtest a DeFi Strategy

The user wants to backtest a DeFi strategy. Follow this pipeline:

Strategy description: $ARGUMENTS

## Steps

1. **Write the strategy JSON** at `strategies/<name>.json` based on the user's description. Use `target/release/defi-flow list-nodes` and `target/release/defi-flow example` for reference if needed.

2. **Validate**: `target/release/defi-flow validate strategies/<name>.json`
   - Fix any errors and re-validate until clean.

3. **Fetch data**: `target/release/defi-flow fetch-data strategies/<name>.json --output-dir data/<name> --days 365 --interval 8h`

4. **Backtest**: `target/release/defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000`
   - Add `--output results/<name>.json` to save metrics to file
   - Add `--tick-csv results/<name>_ticks.csv` for per-tick venue values
   - Add `--verbose` for per-tick logging

5. **Report results** to the user: TWRR%, Annualized%, Max Drawdown%, Sharpe, Net PnL, and any notable metrics.

6. **If the user wants Monte Carlo**: `target/release/defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000 --monte-carlo 500`

## Key Rules

- All addresses go in `tokens` and `contracts` manifests — nodes reference by named keys
- Chain names are lowercase: `hyperevm`, `base`, `ethereum`
- Each node gets its own simulator — don't split open/collect_funding into separate nodes
- Funding auto-compounds inside perp margin
- Re-run `fetch-data` after renaming node IDs
- Delta-neutral: use `target_nodes` group in optimizer allocations (spot+perp legs). Omit `expected_return`/`volatility` for adaptive mode.
- Adaptive Kelly: leave `expected_return` and `volatility` empty in allocations — the optimizer derives them from venue data automatically
- `target_nodes: ["buy_eth", "short_eth"]` groups legs — optimizer never rebalances between them

## LP Backtest Notes

- Concentrated liquidity uses tick math, fee concentration multiplier, and gauge rewards
- Backtest simulates: tick as OU process, fee APY as AR(1), reward rate as AR(1), price from shared GBM
- Use `tick_lower`/`tick_upper` to set range, or omit for full-range
- Gauge staking rewards require `stake_gauge` action node downstream

## Reserve Config

If the strategy uses a reserve config, the backtest monitors vault TVL each tick:
- If reserve < `trigger_threshold`, it unwinds venues pro-rata to restore `target_ratio`
- `min_unwind` prevents dust-level operations (default $100)

## Interpreting Monte Carlo Results

Monte Carlo generates synthetic data via parametric models (GBM prices, OU funding rates, AR(1) yields) estimated from historical data, then re-runs the backtest on each synthetic path. It stress-tests beyond the single historical path.

**Why MC results often look worse than historical:**

1. **Funding rate distribution**: If the historical mean funding rate is small relative to its stdev (common — e.g. mean 1bps, stdev 1.5bps), the OU process generates many paths where funding is net negative for extended periods. Shorts pay instead of receive.

2. **Price volatility + rebalancing drag**: The GBM uses historical vol (often 50-80% for crypto). Between weekly rebalances, delta drifts. The optimizer rebalances at moved prices → buys high, sells low. The historical path may have been smooth; MC generates whipsaws.

3. **Liquidation tail risk**: At 1x short leverage, liquidation triggers around ~2x price. With high vol, some GBM paths hit this. Liquidation is a permanent margin loss; the offsetting spot gain is unrealized and can reverse.

4. **Adaptive Kelly with missing data**: If a venue reports 0% return / 100% vol (e.g. lending with sparse data), Kelly allocates 0% there. This removes diversification and puts 100% into the alpha leg with no cash buffer.

**Historical Sharpe >> MC Sharpe is normal.** A single benign period produces inflated Sharpe ratios. MC median Sharpe is closer to reality. If MC shows negative 5th percentile returns, the strategy has genuine tail risk that the historical period didn't trigger.

**What to check if MC looks bad:**
- Are all venues getting nonzero adaptive stats? Check the `[kelly]` log lines for `return=0.00%` venues
- Is the rebalance frequency appropriate for the vol? Daily > weekly for high-vol assets
- Consider setting explicit `expected_return`/`volatility` on allocations instead of adaptive mode if the data is sparse
