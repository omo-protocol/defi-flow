---
name: strategy-stats
description: Check running strategy performance — TVL, PnL, APR, APY, drawdown. Triggers on strategy stats, check performance, how are strategies doing, strategy health, portfolio performance, show returns.
version: 1.0.0
metadata:
  openclaw:
    emoji: "📈"
    requires:
      bins:
        - cast
---

# Strategy Stats

Check the performance of running defi-flow strategy daemons. Shows TVL, PnL, APR, APY, max drawdown, and per-source income breakdowns. Also cross-references on-chain vault state.

## Quick Overview

```bash
defi-flow ps --registry-dir /app/.defi-flow
```

## Detailed Performance

For each strategy, read its state file for full metrics:

```bash
cat /app/.defi-flow/registry.json | python3 -c "
import json, sys
reg = json.load(sys.stdin)
for name, entry in reg.get('daemons', {}).items():
    print(f'{name}|{entry[\"state_file\"]}|{entry[\"mode\"]}|{entry[\"network\"]}|{entry.get(\"started_at\",\"\")}')
"
```

For each strategy state file:

```bash
cat <STATE_FILE> | python3 -c "
import json, sys, math
from datetime import datetime

state = json.load(sys.stdin)
balances = state.get('balances', {})
tvl = sum(balances.values())
initial = state.get('initial_capital', 0)
peak = state.get('peak_tvl', tvl)
pnl = tvl - initial if initial > 0 else 0

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

## Cross-Reference with On-Chain Vault

For vault strategies, verify the on-chain valuer report matches:

```bash
RPC="https://rpc.hyperliquid.xyz/evm"
VALUER="0x_VALUER_ADDRESS"

# Read strategy_id from the strategy JSON
STRATEGY_ID=$(cat <STRATEGY_JSON> | python3 -c "
import json, sys, hashlib
w = json.load(sys.stdin)
sid = w.get('valuer', {}).get('strategy_id', '')
if sid:
    h = hashlib.sha3_256(sid.encode()).hexdigest()
    print(f'0x{h}')
")

# Get on-chain report
cast call $VALUER "getReport(bytes32)(uint256,uint256,uint256,uint256,bool,address)" $STRATEGY_ID --rpc-url $RPC
```

Compare `value` (first return) with the strategy's TVL from state file. They should match within the push interval.

## Summary Report Format

```
=== Portfolio Performance Report ===

Strategy          Mode     TVL         PnL        APR     APY    Max DD   On-Chain
───────────────────────────────────────────────────────────────────────────────────
USDC Lending      live     $50,000     $1,234     12.0%   12.7%  0.5%     $50,000
Delta Neutral     live     $100,000    $3,456     18.5%   20.3%  1.2%     $100,000
PT Yield          dry-run  $25,000     $890       15.2%   16.4%  0.3%     N/A

Total Portfolio: $175,000 | Total PnL: $5,580 | Weighted APR: 15.8%

=== Vault Health ===
Morpho USDC Vault: TVL=$175,000 | Reserve Ratio: 22% | Status: HEALTHY
```

## Alert Conditions

| Condition | Severity | Action |
|-----------|----------|--------|
| Strategy PnL negative > $100 | WARNING | Check logs, investigate cause |
| Max drawdown > 5% | HIGH | Consider reducing allocation |
| On-chain value diverges >2% from state | HIGH | Check valuer push timing |
| Strategy crashed | CRITICAL | Restart via strategy-daemon skill |
| APR < 2% after 7 days | INFO | Review strategy viability |

## MongoDB Historical Trends

```bash
# 24h performance trend for a strategy
mongosh "$MONGODB_URI" --eval "
  const cutoff = new Date(Date.now() - 24*3600*1000);
  db.getSiblingDB('$MONGODB_DB').strategy_stats
    .find({ strategy: '<NAME>', timestamp: { \$gte: cutoff } })
    .sort({ timestamp: 1 })
    .forEach(r => print(\`\${r.timestamp.toISOString()} TVL=\$\${r.tvl.toFixed(2)} APR=\${(r.apr*100).toFixed(2)}%\`))
"
```

## Rules

1. **Always show all strategies** — include dry-run and live
2. **Cross-reference on-chain** for live vault strategies
3. **Flag divergences** between state file TVL and on-chain valuer
4. **Weighted APR** for portfolio — weight by TVL, not equal
5. **Short uptime caveat** — note if <24h uptime (APR is noisy)
6. **Never expose private keys** in reports
