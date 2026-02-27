#!/bin/bash
set -e

echo "=== Hedgefund Agent Starting ==="

# Ship any pending logs from previous session
if [ -n "$MONGODB_URI" ]; then
  echo "Shipping pending logs to MongoDB..."
  node /app/scripts/ship-logs.mjs || echo "Log shipment failed (non-fatal), continuing."
fi

# Start gateway in background
openclaw gateway --port 18790 --verbose &
GATEWAY_PID=$!

# Wait for gateway to be ready
echo "Waiting for gateway..."
for i in $(seq 1 30); do
  if curl -sf http://127.0.0.1:18790/ > /dev/null 2>&1; then
    echo "Gateway ready."
    break
  fi
  sleep 1
done

# Register cron jobs
echo "Registering cron jobs..."

# Every 15 minutes: check vault health
openclaw cron add \
  --name "vault-health-check" \
  --cron "*/15 * * * *" \
  --session isolated \
  --message "Run vault-monitor skill. Check all whitelisted vaults in vaults.json. Report reserve ratios, TVL, any alerts. Log to memory." \
  2>/dev/null || echo "Cron 'vault-health-check' may already exist, skipping."

# Hourly: detailed vault metrics
openclaw cron add \
  --name "hourly-vault-metrics" \
  --cron "0 * * * *" \
  --session isolated \
  --message "Run vault-manager skill. Get detailed metrics for all whitelisted vaults: totalAssets, idle balance, reserve ratio, our share value. Log full report to daily memory." \
  2>/dev/null || echo "Cron 'hourly-vault-metrics' may already exist, skipping."

echo "=== Cron jobs registered ==="

# Background log shipper — ships to MongoDB every 15 minutes
if [ -n "$MONGODB_URI" ]; then
  echo "Starting background log shipper (every 15m)..."
  while true; do
    sleep 900
    node /app/scripts/ship-logs.mjs 2>&1 | while read -r line; do echo "[log-shipper] $line"; done
  done &
  echo "Log shipper PID: $!"

  # Portfolio stats shipper — runs every 5 minutes
  echo "Starting portfolio stats shipper (every 5m)..."
  while true; do
    sleep 300
    node /app/scripts/ship-stats.mjs 2>&1 || true
  done &
  echo "Stats shipper PID: $!"
fi

echo "=== Hedgefund Agent running (gateway PID: $GATEWAY_PID) ==="

wait $GATEWAY_PID
