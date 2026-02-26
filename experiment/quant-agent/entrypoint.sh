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

echo "=== Quant Agent Starting ==="

# Ship any pending logs from previous session
if [ -n "$MONGODB_URI" ]; then
  echo "Shipping pending logs to MongoDB..."
  node /app/scripts/ship-logs.mjs || echo "Log shipment failed (non-fatal), continuing."
fi

# Start gateway in background
openclaw gateway --port 18789 --verbose &
GATEWAY_PID=$!

# Wait for gateway to be ready
echo "Waiting for gateway..."
for i in $(seq 1 30); do
  if curl -sf http://127.0.0.1:18789/ > /dev/null 2>&1; then
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

# Register cron jobs
echo "Registering cron jobs..."

openclaw cron add \
  --name "hourly-yield-scan" \
  --cron "0 * * * *" \
  --session isolated \
  --message "Run the quant-scan skill. Scan DeFiLlama yields and Hyperliquid funding rates. Build and backtest any promising strategies. Save results to memory." \
  2>/dev/null || echo "Cron 'hourly-yield-scan' may already exist, skipping."

openclaw cron add \
  --name "daily-rebacktest" \
  --cron "0 6 * * *" \
  --session isolated \
  --message "Re-backtest all saved strategies in /app/strategies/ with fresh data. Flag any where Sharpe dropped >30%. Update memory with results." \
  2>/dev/null || echo "Cron 'daily-rebacktest' may already exist, skipping."

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
