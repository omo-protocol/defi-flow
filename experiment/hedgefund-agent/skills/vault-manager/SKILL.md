---
name: vault-manager
description: Manage Morpho v2 vaults on HyperEVM — deposit, withdraw, check metrics. Triggers on vault deposit, vault withdraw, vault metrics, vault balance, check vault, vault TVL.
version: 1.0.0
metadata:
  openclaw:
    emoji: "🏦"
    requires:
      bins:
        - cast
      env:
        - PRIVATE_KEY
    primaryEnv: PRIVATE_KEY
---

# Vault Manager

Manage whitelisted Morpho v2 vaults on HyperEVM. All operations use USDT0 as the base token.

## Check Vault Metrics

Read on-chain vault state using `cast` (from foundry):

```bash
RPC="https://rpc.hyperliquid.xyz/evm"
VAULT="0x_VAULT_ADDRESS"  # from vaults.json
USDT0="0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb"

# Total vault assets (ERC4626)
cast call $VAULT "totalAssets()(uint256)" --rpc-url $RPC

# Vault's idle USDT0 balance
cast call $USDT0 "balanceOf(address)(uint256)" $VAULT --rpc-url $RPC

# Total shares supply
cast call $VAULT "totalSupply()(uint256)" --rpc-url $RPC

# Our share balance
cast call $VAULT "balanceOf(address)(uint256)" $WALLET --rpc-url $RPC

# Convert our shares to assets
cast call $VAULT "convertToAssets(uint256)(uint256)" $OUR_SHARES --rpc-url $RPC
```

Compute reserve ratio: `idle_balance / total_assets`

### Interpreting Metrics

| Metric | Healthy | Warning | Critical |
|--------|---------|---------|----------|
| Reserve ratio | > 20% | 5-20% | < 5% (triggers unwind) |
| TVL change (24h) | < 5% | 5-10% | > 10% |

## Deposit to Vault

```bash
# Approve USDT0 for vault
cast send $USDT0 "approve(address,uint256)" $VAULT $AMOUNT \
  --rpc-url $RPC --private-key $PRIVATE_KEY

# Deposit to vault (ERC4626)
cast send $VAULT "deposit(uint256,address)" $AMOUNT $WALLET \
  --rpc-url $RPC --private-key $PRIVATE_KEY
```

**Note:** USDT0 has 6 decimals. $1,000 = `1000000000` (1000 * 10^6).

## Withdraw from Vault

```bash
# Withdraw assets (ERC4626)
cast send $VAULT "withdraw(uint256,address,address)" $AMOUNT $WALLET $WALLET \
  --rpc-url $RPC --private-key $PRIVATE_KEY
```

## Rules

1. **Only whitelisted vaults.** Read `vaults.json` before any operation.
2. **Check metrics before depositing.** Don't deposit into a vault with anomalous state.
3. **Never withdraw more than 50%** without human approval.
4. **Log every operation** to daily memory with tx hash and amounts.
5. **Always verify** the tx succeeded by re-reading vault state after.
6. **NEVER use raw ERC20 `transfer()` to send tokens to a vault address.** These are Morpho v2 ERC4626 vaults — you MUST use the `deposit()` function above. Tokens sent via `transfer()` are permanently lost (no shares minted, vault doesn't track them).
