#!/bin/bash
set -e

REGISTRY_DIR="/app/.defi-flow"

# ── Graceful shutdown handler ──
# Forward SIGTERM to strategy daemons so they save state.
# Registry entries are preserved (defi-flow run doesn't deregister on SIGTERM)
# so resume-all can restart them on next container boot.
cleanup() {
  echo "=== Received shutdown signal ==="

  # Stop all running strategy daemons (graceful — they save state)
  echo "Stopping strategy daemons..."
  pkill -TERM -f "defi-flow run" 2>/dev/null || true
  sleep 3

  # Stop gateway
  [ -n "$GATEWAY_PID" ] && kill "$GATEWAY_PID" 2>/dev/null || true

  echo "=== Shutdown complete ==="
  exit 0
}
trap cleanup SIGTERM SIGINT

PORT=${GATEWAY_PORT:-18789}

echo "=== Quant Agent Starting (model: ${MODEL_NAME:-unknown}, port: $PORT) ==="

# Ship any pending logs from previous session
if [ -n "$MONGODB_URI" ]; then
  echo "Shipping pending logs to MongoDB..."
  node /app/scripts/ship-logs.mjs || echo "Log shipment failed (non-fatal), continuing."
fi

# Start gateway in background
openclaw gateway --port $PORT --verbose &
GATEWAY_PID=$!

# Wait for gateway to be ready
echo "Waiting for gateway..."
for i in $(seq 1 30); do
  if curl -sf http://127.0.0.1:$PORT/ > /dev/null 2>&1; then
    echo "Gateway ready."
    break
  fi
  sleep 1
done

# Resume previously running strategy daemons
if [ -f "$REGISTRY_DIR/registry.json" ]; then
  echo "=== Resuming strategies from registry ==="
  defi-flow resume-all --registry-dir "$REGISTRY_DIR" || echo "Resume failed (non-fatal), continuing."
fi

# Register cron jobs — these are the ONLY way the agent takes action (heartbeat is disabled)
echo "Registering cron jobs..."

# Clear stale cron jobs from previous runs
openclaw cron list --json 2>/dev/null | python3 -c "
import sys, json, subprocess
try:
    data = json.load(sys.stdin)
    for job in data.get('jobs', []):
        subprocess.run(['openclaw', 'cron', 'remove', '--id', job['id']], capture_output=True)
        print(f'  Removed stale cron: {job.get(\"name\", job[\"id\"])}')
except: pass
" 2>/dev/null || true

openclaw cron add \
  --name "scan-build-deploy" \
  --cron "0 * * * *" \
  --session isolated \
  --message "You are a DeFi quant agent. Execute this pipeline NOW:

STEP 1: Check running strategies
  Run: defi-flow ps --registry-dir /app/.defi-flow
  If strategies are running and healthy, log their TVL/PnL and skip to STEP 5.
  If crashed, check logs and restart them.

STEP 2: Scan for yield opportunities
  Run these two commands:
    curl -s 'https://yields.llama.fi/pools' | jq '[.data[] | select((.chain==\"Hyperliquid\" or .chain==\"Base\" or .chain==\"Arbitrum\" or .chain==\"HyperEVM\") and .tvlUsd > 500000 and .apy > 5)] | sort_by(-.apy) | .[0:15] | .[] | {pool: .pool, symbol: .symbol, chain: .chain, apy: .apy, tvl: .tvlUsd}'
    curl -s 'https://api.hyperliquid.xyz/info' -X POST -H 'Content-Type: application/json' -d '{\"type\": \"metaAndAssetCtxs\"}' | jq '.[1][] | select(.funding != null) | {coin: .coin, funding_ann: ((.funding | tonumber) * 8760 * 100), oi: .openInterest} | select(.funding_ann > 10 or .funding_ann < -10)'

STEP 3: Build a strategy JSON
  Pick the BEST opportunity. Read the defi-flow skill (head -50 skills/defi-flow/SKILL.md) for the JSON format.
  Start simple — a single lending strategy (USDT0 on HyperLend) is easiest:
    wallet(hyperevm) -> lending(aave_v3, hyperlend_pool, USDT0, supply)
  Save to /app/strategies/<name>.json

STEP 4: Validate, fetch data, backtest, and deploy
  defi-flow validate /app/strategies/<name>.json
  If validation fails, FIX the JSON and retry (up to 3 times).
  defi-flow fetch-data /app/strategies/<name>.json --days 90 --interval 8h
  defi-flow backtest /app/strategies/<name>.json --capital 30 --monte-carlo 50
  If Sharpe > 1.0 and DD < 25%:
    mkdir -p /app/.defi-flow/logs /app/.defi-flow/state
    nohup defi-flow run /app/strategies/<name>.json --network mainnet --registry-dir /app/.defi-flow --state-file /app/.defi-flow/state/<name>.state.json --log-file /app/.defi-flow/logs/<name>.log > /app/.defi-flow/logs/<name>.log 2>&1 &
    defi-flow ps --registry-dir /app/.defi-flow

STEP 5: Log results to memory
  Write scan results, backtest metrics, and daemon status to memory/\$(date +%Y-%m-%d).md
  Update MEMORY.md if you learned something new." \
  2>/dev/null || echo "Cron 'scan-build-deploy' may already exist, skipping."

openclaw cron add \
  --name "daemon-health-check" \
  --cron "*/15 * * * *" \
  --session isolated \
  --message "You are a DeFi quant agent. Check daemon health NOW:

1. Run: defi-flow ps --registry-dir /app/.defi-flow
2. For each running daemon: log TVL and PnL
3. For each crashed daemon: run 'defi-flow logs <name> -n 50 --registry-dir /app/.defi-flow', diagnose, and restart if appropriate
4. If NO strategies are running and /app/strategies/ has validated strategy files, deploy the best one:
   nohup defi-flow run /app/strategies/<name>.json --network mainnet --registry-dir /app/.defi-flow --state-file /app/.defi-flow/state/<name>.state.json --log-file /app/.defi-flow/logs/<name>.log > /app/.defi-flow/logs/<name>.log 2>&1 &
5. Write a one-line status to memory/\$(date +%Y-%m-%d).md" \
  2>/dev/null || echo "Cron 'daemon-health-check' may already exist, skipping."

openclaw cron add \
  --name "daily-memory-cleanup" \
  --cron "0 0 * * *" \
  --session isolated \
  --message "Review memory/ files. Update MEMORY.md with any new persistent learnings. Remove strategies older than 30 days with Sharpe < 0.5." \
  2>/dev/null || echo "Cron 'daily-memory-cleanup' may already exist, skipping."

echo "=== Cron jobs registered ==="

# Background log shipper — ships to MongoDB every 15 minutes
if [ -n "$MONGODB_URI" ]; then
  echo "Starting background log shipper (every 15m)..."
  while true; do
    sleep 900
    node /app/scripts/ship-logs.mjs 2>&1 | while read -r line; do echo "[log-shipper] $line"; done
  done &
  echo "Log shipper PID: $!"

  # Background stats shipper — watches registry + state files, ships perf metrics
  echo "Starting strategy stats shipper..."
  node /app/scripts/ship-stats.mjs 2>&1 | while read -r line; do echo "$line"; done &
  echo "Stats shipper PID: $!"
fi

echo "=== Quant Agent running (gateway PID: $GATEWAY_PID) ==="

# Keep container alive — wait uses bash's built-in which is signal-aware
wait $GATEWAY_PID
