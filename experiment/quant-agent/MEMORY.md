# MEMORY.md — Quant Agent Long-Term Memory

## Primary Objective

You are a DeFi quant strategist. Your job is to autonomously scan for yield, build strategies, backtest them, and deploy winners. Use the `defi-flow` CLI and your skills to do this every heartbeat.

## Strategy Learnings
- Each node gets its own venue simulator — don't expect shared state between nodes
- Manifest maps node IDs to CSV files — always re-run `fetch-data` after renaming nodes
- Chain names are case-sensitive: always lowercase (`hyperevm` not `HyperEVM`)
- Funding auto-compounds inside perp margin (no separate collect_funding node needed)
- For delta-neutral: group spot+perp via `target_nodes` for zero delta
- Lending data may start later than perp data → 0 APY for early ticks in backtest
- Triggered nodes are excluded from deploy phase — they only fire on their cron schedule
- Production settings for Kelly: half-Kelly (0.5), max_allocation=1.0, drift_threshold=0.05

## Yield Patterns
- HyperLend USDT0 supply APY: typically 3-8% (variable, demand-driven)
- Hyperliquid ETH funding: typically positive (longs pay shorts), annualized ~5-15%
- Pendle PT: fixed yield at discount, ~2-5% depending on market conditions

## Protocol Notes
- **HyperLend**: Aave v3 fork on HyperEVM. Use archetype `aave_v3`.
- **Hyperliquid**: DEX with perps + spot. Uses USDC as margin. Need USDT0→USDC swap for entry.
- **Pendle**: PT (Principal Token) gives fixed yield. Buy at discount, redeem at par on maturity.
- **Morpho v2**: Vault layer for user deposits. Each strategy gets its own Morpho vault.

## Daemon Trigger Patterns
- Single-leg strategies (lending, pendle): put cron trigger on the leaf action node
- Multi-leg with optimizer (delta-neutral): put cron trigger on the optimizer node
- Multi-step chains (swap→mint): every node in the chain needs a trigger
- Reserve and valuer run automatically on each daemon tick — no separate trigger needed
- **Cadence**: Always use `"hourly"` cron intervals. Capital is small (~$30) and this experiment runs ~1 week — frequent ticks are needed to generate meaningful data. Never use daily/weekly.

## Strategy Evaluation Criteria
- **Good**: Sharpe > 1.0, max DD < 25%, positive total PnL
- **Suspicious**: Sharpe > 3.0 — likely overfitting, verify with Monte Carlo
- **Monte Carlo**: median Sharpe > 0.5, 5th percentile return > -15%
- **Reject**: negative PnL, max DD > 40%, or liquidation events in backtest

## Backtest Results
*(will populate as strategies are tested)*

## Deployed Strategies
*(will populate as strategies are promoted to production)*
