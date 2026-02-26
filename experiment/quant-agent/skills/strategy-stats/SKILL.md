---
name: strategy-stats
description: Check running strategy performance — TVL, PnL, APR, APY, drawdown. Triggers on strategy stats, check performance, how are strategies doing, strategy health, portfolio performance, show returns.
version: 1.0.0
metadata:
  openclaw:
    emoji: "📈"
    requires:
      bins:
        - defi-flow
---

# Strategy Stats

Check the performance of running defi-flow strategy daemons. Shows TVL, PnL, APR, APY, max drawdown, and per-source income breakdowns.

## Quick Overview

```bash
defi-flow ps --registry-dir /app/.defi-flow
```

This shows all registered strategies with their status, mode, TVL, and uptime.

## Detailed Performance Report

For each strategy shown by `defi-flow ps`, read its state file for full metrics:

```bash
# Get state file path from registry
cat /app/.defi-flow/registry.json | python3 -c "
import json, sys
reg = json.load(sys.stdin)
for name, entry in reg.get('daemons', {}).items():
    print(f'{name}|{entry[\"state_file\"]}|{entry[\"mode\"]}|{entry[\"network\"]}|{entry.get(\"started_at\",\"\")}')
"
```

Then for each strategy, read the state file:

```bash
cat /app/.defi-flow/state/<name>.state.json | python3 -c "
import json, sys, math
from datetime import datetime

state = json.load(sys.stdin)
balances = state.get('balances', {})
tvl = sum(balances.values())
initial = state.get('initial_capital', 0)
peak = state.get('peak_tvl', tvl)
pnl = tvl - initial if initial > 0 else 0

# Compute rates (pass started_at as arg)
started = sys.argv[1] if len(sys.argv) > 1 else ''
if started and initial > 0:
    hours = max((datetime.now() - datetime.fromisoformat(started.replace('Z',''))).total_seconds() / 3600, 1)
    apr = (pnl / initial) * (8766 / hours)
    apy = math.exp(apr) - 1
else:
    apr = apy = 0

dd = max(0, 1 - tvl / peak) if peak > 0 else 0

print(f'TVL:           \${tvl:,.2f}')
print(f'Initial:       \${initial:,.2f}')
print(f'PnL:           \${pnl:,.2f}')
print(f'APR:           {apr*100:.2f}%')
print(f'APY:           {apy*100:.2f}%')
print(f'Max Drawdown:  {dd*100:.2f}%')
print(f'Peak TVL:      \${peak:,.2f}')
print()
print('Income Breakdown:')
print(f'  Funding:     \${state.get(\"cumulative_funding\", 0):,.2f}')
print(f'  Interest:    \${state.get(\"cumulative_interest\", 0):,.2f}')
print(f'  Rewards:     \${state.get(\"cumulative_rewards\", 0):,.2f}')
print(f'  Swap Costs:  -\${state.get(\"cumulative_costs\", 0):,.2f}')
print()
print('Balances:')
for node, bal in sorted(balances.items()):
    print(f'  {node}: \${bal:,.2f}')
" "\$STARTED_AT"
```

## Generating the Report

When asked for strategy stats, performance, or portfolio health:

1. Run `defi-flow ps --registry-dir /app/.defi-flow` for the overview
2. For each strategy, read its state file and compute metrics using the script above
3. Present a summary table:

```
=== Strategy Performance Report ===

Strategy          Mode     TVL         PnL        APR     APY    Max DD
─────────────────────────────────────────────────────────────────────────
USDC Lending      live     $50,000     $1,234     12.0%   12.7%  0.5%
Delta Neutral     dry-run  $100,000    $3,456     18.5%   20.3%  1.2%
PT Yield          dry-run  $25,000     $890       15.2%   16.4%  0.3%

Total Portfolio: $175,000 | Total PnL: $5,580

=== Income Sources ===
Funding:   $2,100
Interest:  $2,890
Rewards:   $740
Costs:     -$150
```

4. Flag any issues:
   - Negative PnL → investigate logs
   - Max drawdown > 5% → alert
   - Stale last_tick (>1 hour behind) → may be stuck
   - Crashed status → needs restart

## MongoDB Historical Data

If MongoDB is available, query historical stats:

```bash
# Latest stats for all strategies
mongosh "$MONGODB_URI" --eval "
  db.getSiblingDB('$MONGODB_DB').strategy_stats
    .aggregate([
      { \$sort: { timestamp: -1 } },
      { \$group: { _id: '\$strategy', latest: { \$first: '$$ROOT' } } }
    ])
    .forEach(r => printjson(r.latest))
"
```

## Rules

1. **Always show all strategies** — don't skip dry-run ones
2. **Flag anomalies** — negative PnL, high drawdown, stale ticks
3. **Compare to initial** — PnL is always relative to initial_capital
4. **Round sensibly** — 2 decimal places for USD, 1 for percentages
5. **Include uptime** — context matters for APR extrapolation (short uptime = noisy APR)
