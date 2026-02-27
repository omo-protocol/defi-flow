# MEMORY.md — Hedgefund Agent Long-Term Memory

## Base Token
- USDT0 on HyperEVM: `0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb`
- 6 decimals (standard Tether)

## Active Strategies (Mainnet)

Three vault strategies are running as Docker containers on the VPS. Each has its own Morpho v2 vault and strategy wallet. Read `vaults.json` for vault addresses. Read strategy state files from mounted volumes at `/app/strategy-states/` for live metrics.

### 1. Lending (USDT0 → HyperLend)
- Supplies USDT0 to HyperLend for lending yield
- Daemon: daily cron — re-supplies idle USDT0 from new vault deposits
- State file: `/app/strategy-states/lending/state.json`

### 2. Delta-Neutral (Spot ETH + Short Perp)
- Spot buy ETH + short ETH perp for funding income. Idle USDT0 to HyperLend.
- Kelly optimizer splits between hedged pair and lending
- Daemon: weekly cron on optimizer, 5% drift threshold
- State file: `/app/strategy-states/delta_neutral/state.json`

### 3. PT Fixed Yield (Pendle PT-kHYPE)
- Swaps USDT0→USDC, buys Pendle PT-kHYPE at discount, holds to maturity
- Daemon: daily cron — swaps + mints PT with new vault deposits
- State file: `/app/strategy-states/pt_yield/state.json`

## Vault Operations
- Use `cast call` to read vault state (totalAssets, balanceOf, convertToAssets)
- Use `cast send` to deposit/withdraw (ERC4626 interface)
- All vaults accept USDT0, all on HyperEVM (chain_id: 999)
- Check `vaults.json` for vault addresses and reserve thresholds

## Vault Performance
*(will populate as monitoring runs)*

## Reserve Events
*(will populate as reserve actions occur)*
