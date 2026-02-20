# Backtest a DeFi Strategy

The user wants to backtest a DeFi strategy. Follow this pipeline:

Strategy description: $ARGUMENTS

## Steps

1. **Write the strategy JSON** at `strategies/<name>.json` based on the user's description. Use `target/release/defi-flow list-nodes` and `target/release/defi-flow example` for reference if needed.

2. **Validate**: `target/release/defi-flow validate strategies/<name>.json`
   - Fix any errors and re-validate until clean.

3. **Fetch data**: `target/release/defi-flow fetch-data strategies/<name>.json --output-dir data/<name> --days 365 --interval 8h`

4. **Backtest**: `target/release/defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000`

5. **Report results** to the user: TWRR%, Annualized%, Max Drawdown%, Sharpe, Net PnL, and any notable metrics.

6. **If the user wants Monte Carlo**: `target/release/defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000 --monte-carlo 500`

## Key Rules

- All addresses go in `tokens` and `contracts` manifests — nodes reference by named keys
- Chain names are lowercase: `hyperevm`, `base`, `ethereum`
- Each node gets its own simulator — don't split open/collect_funding into separate nodes
- Funding auto-compounds inside perp margin
- Re-run `fetch-data` after renaming node IDs
- Delta-neutral: use same expected_return/volatility for spot+perp legs so Kelly assigns equal weight
