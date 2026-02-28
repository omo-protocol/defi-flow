# HEARTBEAT.md

## 1. Strategy Pipeline (CRITICAL — every heartbeat)

Your job is to find yield opportunities, build strategies, backtest them, and deploy winners.

- [ ] Run yield scan: use the `scan-yields` skill
  - DeFiLlama `/pools` — any new pools >10% APY on supported chains (HyperEVM, Base, Arbitrum)?
  - Hyperliquid funding rates — any pairs with annualized funding >15%?
- [ ] For each promising opportunity:
  - Build a strategy JSON using the `defi-flow` skill (node types, edges, triggers)
  - Validate: `defi-flow validate strategies/<name>.json`
  - Fetch data: `defi-flow fetch-data strategies/<name>.json`
  - Backtest: `defi-flow backtest strategies/<name>.json` (check Sharpe, max DD, PnL)
  - If Sharpe > 1.0 and max DD < 25%: save the strategy
  - If Sharpe > 3.0: be skeptical — check for overfitting, lookahead bias
- [ ] Or use the `quant-scan` skill for autonomous pipeline (scan → build → test → save)
- [ ] Log every backtest result to daily memory (strategy name, Sharpe, DD, PnL)

## 2. Running Daemons

- [ ] Run `defi-flow ps --registry-dir /app/.defi-flow` — list all running daemons
- [ ] For each daemon, check status:
  - Alive and healthy? → note TVL + PnL in daily log
  - Crashed? → check logs: `defi-flow logs <name> -n 100 --registry-dir /app/.defi-flow`
    - If transient error (RPC timeout, nonce issue): restart with same params
    - If persistent error: analyze root cause, log to memory, stop daemon
  - TVL dropped >20% since last check? → investigate, log findings
- [ ] Any dry-run daemons running >24h with good TVL? → flag for production promotion
  - Use the `strategy-daemon` skill to manage (start/stop/promote)
  - **Promotion requires human approval** — never promote without being told to

## 3. Strategy Health

- [ ] Re-backtest top 3 saved strategies (by Sharpe) with fresh data
  - Fetch latest data: `defi-flow fetch-data strategies/<name>.json`
  - Re-run backtest: `defi-flow backtest strategies/<name>.json`
- [ ] Flag strategies where Sharpe dropped >30% from last run — log reason
- [ ] Check if any protocol in saved strategies lost >50% TVL (via DeFiLlama)
- [ ] Use `strategy-stats` skill to get performance summary of running daemons

## 4. Memory Maintenance

- [ ] Log to `memory/YYYY-MM-DD.md`:
  - Scan results: [protocol, chain, asset, APY/funding, TVL]
  - Backtest results: [strategy name, Sharpe, max DD, total PnL, MC median]
  - Daemon status: [name, status, TVL, PnL, action taken]
  - New strategies built or saved
- [ ] Update `MEMORY.md` if new persistent learnings found
- [ ] Clean stale strategies (>30 days, Sharpe < 0.5): move to `strategies/archive/`

## State File
Track last scan timestamps in `memory/heartbeat-state.json`. Don't repeat work.

When nothing needs attention: `HEARTBEAT_OK`
