# MEMORY.md — Quant Agent Long-Term Memory

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
