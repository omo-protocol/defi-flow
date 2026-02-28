# HEARTBEAT.md

## 1. Capital Deployment (CRITICAL — check every heartbeat)

Your wallet holds USDT0. Your job is to deposit it into the whitelisted vaults.

- [ ] Check your USDT0 balance: `cast call $USDT0 "balanceOf(address)(uint256)" $WALLET --rpc-url $RPC`
  - `USDT0=0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb`, `RPC=https://rpc.hyperliquid.xyz/evm`
  - Derive your wallet: `cast wallet address --private-key $PRIVATE_KEY`
- [ ] If balance > $1 USDT0 (> 1000000 raw): **deposit equally across all 3 vaults** using the `vault-manager` skill
  - Split: 1/3 to Lending Vault, 1/3 to Delta-Neutral Vault, 1/3 to PT Yield Vault
  - **CRITICAL: These are Morpho v2 ERC4626 vaults. You MUST use the `deposit(uint256,address)` function via `cast send`. NEVER do a raw ERC20 `transfer()` to the vault address — tokens sent via transfer are NOT tracked by the vault and WILL BE PERMANENTLY LOST. Always: `approve` the vault, then call `deposit` on the vault contract.**
  - For each vault: `approve` then `deposit` (see `vault-manager` skill for exact commands)
  - Log every tx hash to daily memory
  - Re-check vault state after each deposit to verify success

## 2. Vault Health
- [ ] Read `vaults.json` — check each whitelisted vault's reserve ratio
- [ ] Any vault with reserve_ratio < trigger_threshold (5%)? Flag immediately.
- [ ] Any vault TVL changed >10% since last check? Investigate.

## 3. Strategy State
- [ ] Read strategy state files at `/app/strategy-states/*/state.json`
- [ ] Check each strategy's TVL, PnL, and last_tick
- [ ] Any strategy with TVL=0 or last_tick stale (>2h old)? Flag as unhealthy.

## 4. Reserve Actions
- [ ] Check state files for recent `reserve_actions` entries
- [ ] Log any new reserve unwinds to daily memory

## 5. Memory Maintenance
- [ ] Log vault metrics + wallet balance to `memory/YYYY-MM-DD.md`
- [ ] Update `MEMORY.md` if new persistent learnings found

## State File
Track last check timestamps in `memory/heartbeat-state.json`. Don't repeat work.

When nothing needs attention: `HEARTBEAT_OK`
