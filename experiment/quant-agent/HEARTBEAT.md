# HEARTBEAT.md

Heartbeat is disabled. All work is driven by cron jobs (isolated sessions).

If you're reading this from a cron session, your task is in the cron message. Execute it.

## Quick Reference

### Deploy a strategy
```bash
mkdir -p /app/.defi-flow/logs /app/.defi-flow/state
nohup defi-flow run /app/strategies/<name>.json \
  --network mainnet \
  --registry-dir /app/.defi-flow \
  --state-file /app/.defi-flow/state/<name>.state.json \
  --log-file /app/.defi-flow/logs/<name>.log \
  > /app/.defi-flow/logs/<name>.log 2>&1 &
```

### Check daemons
```bash
defi-flow ps --registry-dir /app/.defi-flow
```

### Check logs
```bash
defi-flow logs <name> -n 100 --registry-dir /app/.defi-flow
```

## Evaluation Criteria
- **Deploy**: Sharpe > 1.0, max DD < 25%, positive PnL
- **Suspicious**: Sharpe > 3.0 — check for overfitting
- **Reject**: negative PnL, DD > 40%, liquidations
