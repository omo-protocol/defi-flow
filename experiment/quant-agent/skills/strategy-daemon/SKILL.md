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

### Start a strategy (always dry-run first)

```bash
# Create directories
mkdir -p /app/.defi-flow/logs /app/.defi-flow/state

# Start as daemon — ALWAYS dry-run first
nohup defi-flow run /app/strategies/<name>.json \
  --dry-run \
  --state-file /app/.defi-flow/state/<name>.state.json \
  --log-file /app/.defi-flow/logs/<name>.log \
  --network testnet \
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

### Promote to production (dry-run → live)

**REQUIRES HUMAN APPROVAL.** Never promote without being told to.

```bash
# 1. Stop the dry-run daemon
defi-flow stop <name>

# 2. Restart without --dry-run on mainnet
nohup defi-flow run /app/strategies/<name>.json \
  --state-file /app/.defi-flow/state/<name>.state.json \
  --log-file /app/.defi-flow/logs/<name>.log \
  --network mainnet \
  --registry-dir /app/.defi-flow \
  > /app/.defi-flow/logs/<name>.log 2>&1 &

# 3. Verify
defi-flow ps --registry-dir /app/.defi-flow
```

### Health check (run on every heartbeat)

```bash
# Check all daemons
defi-flow ps --registry-dir /app/.defi-flow

# For each "crashed" entry, investigate:
defi-flow logs <name> -n 100 --registry-dir /app/.defi-flow

# Restart crashed strategies if appropriate
```

## Rules

1. **Always start dry-run.** No exceptions.
2. **Run dry-run ≥24h** before considering promotion.
3. **Never promote** without explicit human approval.
4. **Check `defi-flow ps`** on every heartbeat — restart crashed daemons.
5. **Log everything.** Always use `--log-file` so `defi-flow logs` works.
6. **State files persist.** A restarted daemon picks up where it left off via `--state-file`.
7. **One daemon per strategy.** Don't start the same strategy twice — stop first, then start.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| `defi-flow ps` shows "crashed" | Check logs: `defi-flow logs <name> -n 100`. Restart if transient. |
| PID alive but no state updates | Check `last_tick` in state file. May be stuck — stop and restart. |
| Strategy shows $0 TVL | Deploy phase may have failed. Check logs for "Deploy phase" errors. |
| Can't stop — PID stale | `defi-flow stop <name>` handles stale PIDs (cleans registry). |
