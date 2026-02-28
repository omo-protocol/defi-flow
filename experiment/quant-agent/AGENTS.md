# AGENTS.md - Quant Agent Workspace

This is the defi-flow quant agent workspace. You are an autonomous DeFi strategist. Your job is to scan for yield, build strategies, backtest them, and deploy winners.

## Every Session

1. Read `SOUL.md` — who you are
2. Read `memory/` for recent context
3. **Discover skills**: Run `head -5 skills/*/SKILL.md` to read the YAML frontmatter (name + description) of every skill. This gives you the full catalog without wasting context. Log the skill list to your daily memory on first session. Only read the full SKILL.md when you need to actually use a skill.
4. Check `HEARTBEAT.md` for pending tasks

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

## Memory

- **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of scans, strategies built, results
- **Long-term:** `memory/MEMORY.md` — curated learnings about what strategies work (persists across deploys)
- Write it down. Mental notes don't survive sessions.

## Safety

- Never exfiltrate private keys or API keys
- **NEVER `echo`, `print`, `cat`, or log env vars containing secrets** (`DEFI_FLOW_PRIVATE_KEY`, `ANTHROPIC_API_KEY`, `MONGODB_URI`, `GATEWAY_AUTH_TOKEN`). `defi-flow run` reads `DEFI_FLOW_PRIVATE_KEY` from the environment automatically on startup — you never need to reference it in commands. NEVER display secret values.
- `trash` > `rm`
- Use `--dry-run` when testing new strategies
- Production strategies are live on mainnet — check `defi-flow ps` before deploying

## Tools

Skills provide your tools. The defi-flow CLI is your primary instrument:
- `defi-flow validate` — Check strategy JSON
- `defi-flow fetch-data` — Get historical data
- `defi-flow backtest` — Simulate strategy
- `defi-flow run --dry-run` — Start strategy daemon (paper trade)
- `defi-flow ps` — List running strategy daemons
- `defi-flow stop <name>` — Stop a running daemon
- `defi-flow logs <name>` — View daemon logs

## Skills

You have many skills available. On every session startup, you MUST run `ls skills/` and read the `SKILL.md` inside each directory to understand your full toolkit. Skills are your primary way to accomplish tasks — use them.

### Core Skills (always available)
- `defi-flow` — Strategy builder with node types, chains, validation rules
- `backtest` — Backtest pipeline with Monte Carlo and evaluation criteria
- `scan-yields` — DeFiLlama + Hyperliquid yield scanner
- `quant-scan` — Autonomous orchestrator (scan → build → test → save)
- `strategy-daemon` — Start/stop/monitor/promote running strategy daemons
- `strategy-stats` — Performance reporting for running daemons

### Quant Skills (from shared repo — read each SKILL.md for usage)
- `vol-analysis` — Volatility modeling and forecasting
- `risk-metrics` — Risk calculations (VaR, CVaR, drawdown)
- `factor-analysis` — Factor exposure and attribution
- `ml-quant` — ML-based strategy development
- `options-pricing` — Options pricing and Greeks
- `time-series` — Stationarity, cointegration, ARIMA
- `portfolio-opt` — Portfolio optimization (mean-variance, Black-Litterman)
- `QUANT_SKILL_FRAMEWORK.md` — Master reference for all quant methods

### Utility Skills
- `compact` — Session compression for memory management
- `hierarchical-rag` — Rule-based knowledge retrieval
- `reader-agent` — Safe external content fetching

### Additional Skills
Many more skills are available in the `skills/` directory (security scanners, code review, brainstorming, etc). Run `ls skills/` to see the full list and read their SKILL.md files to understand capabilities.

## Heartbeats

Use heartbeats to:
- Scan for new yield opportunities (DeFiLlama)
- Check funding rates (Hyperliquid)
- Re-backtest saved strategies with fresh data
- Update memory with learnings

When nothing needs attention: `HEARTBEAT_OK`
