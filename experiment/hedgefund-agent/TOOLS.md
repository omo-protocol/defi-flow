# TOOLS.md — Hedgefund Agent Local Notes

## cast (foundry)

**Binary:** `/usr/local/bin/cast`

All vault interactions go through `cast`. Key patterns:

### Read vault state
```bash
RPC="https://rpc.hyperliquid.xyz/evm"
VAULT="0x_FROM_VAULTS_JSON"
USDT0="0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb"

# Total vault assets (6 decimal USDT0)
cast call $VAULT "totalAssets()(uint256)" --rpc-url $RPC

# Idle USDT0 in vault
cast call $USDT0 "balanceOf(address)(uint256)" $VAULT --rpc-url $RPC

# Our share balance
cast call $VAULT "balanceOf(address)(uint256)" $WALLET --rpc-url $RPC

# Convert shares to assets
cast call $VAULT "convertToAssets(uint256)(uint256)" $SHARES --rpc-url $RPC

# Total shares supply
cast call $VAULT "totalSupply()(uint256)" --rpc-url $RPC
```

### Write transactions
```bash
# Approve vault to spend USDT0
cast send $USDT0 "approve(address,uint256)" $VAULT $AMOUNT \
  --rpc-url $RPC --private-key $PRIVATE_KEY

# Deposit USDT0 into vault
cast send $VAULT "deposit(uint256,address)" $AMOUNT $WALLET \
  --rpc-url $RPC --private-key $PRIVATE_KEY

# Withdraw from vault
cast send $VAULT "withdraw(uint256,address,address)" $AMOUNT $WALLET $WALLET \
  --rpc-url $RPC --private-key $PRIVATE_KEY
```

### Unit conversion
USDT0 has 6 decimals:
- $1,000 = `1000000000` (1000 * 1e6)
- $10,000 = `10000000000`

```bash
cast --to-unit 1000000000 6   # → 1000.0
cast --to-wei 1000 6          # → 1000000000 (gwei, but works for 6 decimals)
```

## Environment Variables
- `PRIVATE_KEY` — Wallet private key for vault transactions. **NEVER echo, print, or display — pipe into `cast` only.**
- `ANTHROPIC_API_KEY` — LLM provider. **NEVER echo.**
- `MONGODB_URI` — Log shipping. **NEVER echo.**
- `GATEWAY_AUTH_TOKEN` — OpenClaw gateway auth. **NEVER echo.**

## Chain
- **HyperEVM**: chain_id 999, RPC `https://rpc.hyperliquid.xyz/evm`
- **USDT0**: `0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb` (6 decimals)
