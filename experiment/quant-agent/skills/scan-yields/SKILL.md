---
name: scan-yields
description: Scan DeFiLlama yield data to find high-yield DeFi opportunities across chains and protocols. Triggers on scan yields, find yields, yield opportunities, DeFi yields, funding rates.
version: 1.0.0
metadata:
  openclaw:
    emoji: "🔍"
    requires:
      bins:
        - curl
---

# Scan DeFi Yields

Query DeFiLlama's public API to find yield opportunities across chains and protocols.

## API Endpoints

### All Pools
```bash
curl -s "https://yields.llama.fi/pools" | jq '.data | sort_by(-.apy) | .[0:20]'
```

Returns array of pool objects with: `pool`, `chain`, `project`, `symbol`, `tvlUsd`, `apy`, `apyBase`, `apyReward`, `stablecoin`, `ilRisk`, `exposure`.

### Filter by Chain
```bash
curl -s "https://yields.llama.fi/pools" | jq '[.data[] | select(.chain == "Hyperliquid" or .chain == "Base" or .chain == "Arbitrum" or .chain == "Ethereum")] | sort_by(-.apy) | .[0:20]'
```

### Filter Stablecoins Only
```bash
curl -s "https://yields.llama.fi/pools" | jq '[.data[] | select(.stablecoin == true and .tvlUsd > 100000)] | sort_by(-.apy) | .[0:20]'
```

### Funding Rates (Perps)
```bash
curl -s "https://api.hyperliquid.xyz/info" -X POST -H "Content-Type: application/json" -d '{"type": "metaAndAssetCtxs"}' | jq '.[1][] | {name: .name, funding: .funding, openInterest: .openInterest}'
```

## Workflow

1. **Fetch pools** from DeFiLlama yields API
2. **Filter** by minimum APY, TVL, and supported chains
3. **Rank** by risk-adjusted yield (APY / IL risk)
4. **Cross-reference** with supported protocols:
   - Lending: any Aave V3 fork (HyperLend, Aave on Base/Arbitrum)
   - Perps: Hyperliquid (funding rate arbitrage)
   - LP: Aerodrome on Base
   - Vaults: Morpho vaults
5. **Report** top opportunities with: protocol, chain, asset, APY, TVL, risk level

## Supported Chains for defi-flow

| Chain | DeFiLlama Name | defi-flow Name |
|-------|---------------|----------------|
| Hyperliquid L1 | Hyperliquid | hyperliquid |
| HyperEVM | Hyperliquid | hyperevm |
| Base | Base | base |
| Arbitrum | Arbitrum | arbitrum |
| Ethereum | Ethereum | ethereum |
| Optimism | Optimism | optimism |
| Mantle | Mantle | mantle |

## What to Look For

- **Funding rate arbitrage**: Positive funding > 10% annualized → short perp + long spot
- **Lending yields**: Supply APY > 5% on stablecoins or major assets
- **LP fees**: Concentrated liquidity pools with high fee APY and manageable IL
- **Vault yields**: Morpho/ERC4626 vaults with competitive APY
- **Cross-chain arbitrage**: Same asset, different yields on different chains

## Output Format

For each opportunity, report:
```
Protocol: HyperLend
Chain: hyperevm
Asset: USDC
APY: 8.5% (base: 6%, rewards: 2.5%)
TVL: $12M
Risk: Low (stablecoin lending)
Strategy: Idle USDC lending via aave_v3 archetype
```

## Guardrails

- Never fabricate yield data — always query the API
- TVL < $100k = low liquidity, flag as risky
- IL risk "yes" on non-stablecoin pools — factor into strategy design
- APY > 100% is usually temporary/unsustainable — flag as high risk
- Check that the protocol is actually supported by defi-flow before recommending
