# HEARTBEAT.md

## Yield Scan (every heartbeat)
- [ ] Check DeFiLlama `/pools` — any new pools >10% APY on supported chains?
- [ ] Check Hyperliquid funding rates — any pairs with annualized funding >15%?
- [ ] Any saved strategies need re-backtesting with fresh data?

## Running Daemons
- [ ] Run `defi-flow ps` — are all daemons alive?
- [ ] Any crashed daemons? Check logs, restart if transient.
- [ ] Any dry-run daemons running >24h with good TVL? Flag for promotion review.
- [ ] Any daemon TVL dropped >20% since last check? Investigate.

## Strategy Health
- [ ] Re-backtest top 3 saved strategies (by Sharpe) with latest data
- [ ] Flag strategies where Sharpe dropped >30% from last run
- [ ] Check if any protocol in saved strategies lost >50% TVL

## Memory Maintenance
- [ ] Log scan results to `memory/YYYY-MM-DD.md`
- [ ] Update `MEMORY.md` if new persistent learnings found
- [ ] Clean stale strategies (>30 days, Sharpe < 0.5)

## State File
Track last scan timestamps in `memory/heartbeat-state.json`. Don't repeat work.

When nothing needs attention: `HEARTBEAT_OK`
