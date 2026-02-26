---
name: vault-monitor
description: Monitor vault health and reserve ratios. Triggers on vault health, check reserves, monitor vaults, vault status.
version: 1.0.0
metadata:
  openclaw:
    emoji: "📊"
    requires:
      bins:
        - cast
---

# Vault Monitor

Continuous monitoring of whitelisted Morpho v2 vaults on HyperEVM.

## Monitoring Workflow

### 1. Check all vault reserve ratios

For each vault in `vaults.json`:

```bash
RPC="https://rpc.hyperliquid.xyz/evm"
VAULT="0x_VAULT_ADDRESS"
USDT0="0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb"

# Total vault assets
TOTAL_ASSETS=$(cast call $VAULT "totalAssets()(uint256)" --rpc-url $RPC)

# Idle USDT0 in vault
IDLE=$(cast call $USDT0 "balanceOf(address)(uint256)" $VAULT --rpc-url $RPC)

# Our share balance
OUR_SHARES=$(cast call $VAULT "balanceOf(address)(uint256)" $WALLET --rpc-url $RPC)

# Our position value
OUR_VALUE=$(cast call $VAULT "convertToAssets(uint256)(uint256)" $OUR_SHARES --rpc-url $RPC)

# Total supply
TOTAL_SUPPLY=$(cast call $VAULT "totalSupply()(uint256)" --rpc-url $RPC)
```

Compute:
- Reserve ratio: `idle / total_assets`
- Our share %: `our_shares / total_supply * 100`
- Position value: `convertToAssets(our_shares)` in USDT0 (6 decimals)

### 2. Report

Generate a summary:

```
=== Vault Health Report ===

Vault: Morpho USDT0 Vault (0x...)
  TVL: $X (total_assets)
  Idle: $X (reserve)
  Reserve Ratio: X% (target: 20%, trigger: 5%)
  Our Position: $X (X% of vault)
  Status: HEALTHY / WARNING / CRITICAL
```

### 3. Alert Conditions

| Condition | Severity | Action |
|-----------|----------|--------|
| Reserve ratio < 5% | CRITICAL | Report immediately. Strategies should auto-unwind. |
| Reserve ratio 5-10% | WARNING | Log and monitor closely. |
| TVL drop >15% in 24h | HIGH | Investigate — possible large withdrawal. |
| Our position value dropped >10% | HIGH | Check if vault had losses. |

### 4. Log to Memory

After each monitoring run, write to daily memory:
```
## Vault Check [HH:MM]
- Vault X: TVL=$X, idle=$X, reserve=X%, our_position=$X
- Status: HEALTHY/WARNING/CRITICAL
- Actions: [any alerts]
```
