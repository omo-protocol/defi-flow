---
name: strategy-daemon
description: Manage running strategy daemons — start, stop, monitor, promote. Triggers on run strategy, start daemon, stop strategy, check running, promote to production, strategy status.
version: 1.0.0
metadata:
  openclaw:
    emoji: "⚙️"
    requires:
      bins:
        - defi-flow
---

# Strategy Daemon Manager

Manage defi-flow strategy daemons. Start strategies as background processes, monitor their health, and promote from dry-run to production.

## Commands

**IMPORTANT:** Always pass `--registry-dir /app/.defi-flow` to every `defi-flow` daemon command. This directory is volume-mounted and persists across container restarts. Strategies registered here will auto-resume on redeploy.

### List running strategies

```bash
defi-flow ps --registry-dir /app/.defi-flow
```

Output shows name, mode (dry-run/live), network, PID, TVL, uptime, and status (running/crashed).

### Start a strategy (deploy to mainnet)

```bash
# Create directories
mkdir -p /app/.defi-flow/logs /app/.defi-flow/state

# Start as daemon on mainnet
nohup defi-flow run /app/strategies/<name>.json \
  --state-file /app/.defi-flow/state/<name>.state.json \
  --log-file /app/.defi-flow/logs/<name>.log \
  --network mainnet \
  --registry-dir /app/.defi-flow \
  > /app/.defi-flow/logs/<name>.log 2>&1 &

# Verify it registered
defi-flow ps --registry-dir /app/.defi-flow
```

The `run` command self-registers in the daemon registry with its PID, mode, and paths.

### Check logs for a strategy

```bash
# Last 50 lines
defi-flow logs <name> --registry-dir /app/.defi-flow

# Follow live output
defi-flow logs <name> -f --registry-dir /app/.defi-flow

# Last 200 lines
defi-flow logs <name> -n 200 --registry-dir /app/.defi-flow
```

### Stop a strategy

```bash
defi-flow stop <name> --registry-dir /app/.defi-flow
```

Sends SIGTERM for graceful shutdown (state is saved), then deregisters from the registry.

### Health check

```bash
# Check all daemons
defi-flow ps --registry-dir /app/.defi-flow

# For each "crashed" entry, investigate:
defi-flow logs <name> -n 100 --registry-dir /app/.defi-flow

# Restart crashed strategies
```

## Rules

1. **Deploy directly to mainnet.** No dry-runs — capital is small and time is short.
2. **Check `defi-flow ps`** regularly — restart crashed daemons.
3. **Log everything.** Always use `--log-file` so `defi-flow logs` works.
4. **State files persist.** A restarted daemon picks up where it left off via `--state-file`.
5. **One daemon per strategy.** Don't start the same strategy twice — stop first, then start.
6. **Max 2 strategies running** at once — capital is too small to split further.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `defi-flow ps` shows "crashed" | Check logs: `defi-flow logs <name> -n 100`. Restart if transient. |
| PID alive but no state updates | Check `last_tick` in state file. May be stuck — stop and restart. |
| Strategy shows $0 TVL | Deploy phase may have failed. Check logs for "Deploy phase" errors. |
| Can't stop — PID stale | `defi-flow stop <name>` handles stale PIDs (cleans registry). |
