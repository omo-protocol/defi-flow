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
- Vaults are **Morpho v2 ERC4626** contracts. Interaction MUST use the ERC4626 interface.
- **DEPOSIT**: `cast send $VAULT "deposit(uint256,address)" $AMOUNT $YOUR_WALLET` (after approving)
- **WITHDRAW**: `cast send $VAULT "redeem(uint256,address,address)" $SHARES $YOUR_WALLET $YOUR_WALLET`
- **READ**: `cast call` for totalAssets, balanceOf, convertToAssets
- **NEVER use raw ERC20 transfer() to send USDT0 to a vault address. Funds sent via transfer() are permanently lost — the vault does not track them and no shares are minted.**
- All vaults accept USDT0, all on HyperEVM (chain_id: 999)
- Check `vaults.json` for vault addresses and reserve thresholds

## Capital Deployment

Your wallet has USDT0. You MUST deposit it into the 3 whitelisted vaults (equal split). Check your balance every heartbeat. If you have idle USDT0, deposit it immediately using the `vault-manager` skill.

## Vault Performance
*(will populate as monitoring runs)*

## Reserve Events
*(will populate as reserve actions occur)*
