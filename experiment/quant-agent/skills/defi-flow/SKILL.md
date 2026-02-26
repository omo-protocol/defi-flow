---
name: defi-flow
description: Build DeFi strategy workflow JSONs for the defi-flow engine. Validates, fetches data, and backtests strategies. Triggers on build strategy, create strategy, write strategy JSON, defi-flow, new strategy.
version: 1.0.0
metadata:
  openclaw:
    emoji: "🔧"
    requires:
      bins:
        - defi-flow
---

# defi-flow CLI — Strategy Builder

Build DeFi quant strategies as JSON workflow DAGs for the defi-flow engine.

## Pipeline

1. Write workflow JSON to `strategies/<name>.json`
2. Validate: `defi-flow validate strategies/<name>.json`
3. Fetch data: `defi-flow fetch-data strategies/<name>.json --output-dir data/<name> --days 365 --interval 8h`
4. Backtest: `defi-flow backtest strategies/<name>.json --data-dir data/<name> --capital 10000`

## CLI Commands

| Command | Usage |
|---------|-------|
| `schema` | Output JSON Schema for workflow definitions |
| `validate <FILE>` | Validate workflow JSON (offline + on-chain RPC checks) |
| `visualize <FILE>` | Render graph (`--format ascii\|dot\|svg\|png`) |
| `list-nodes` | Print all node types with fields |
| `example` | Print example workflow JSON |
| `fetch-data <FILE>` | Fetch historical data (`--output-dir`, `--days`, `--interval`) |
| `backtest <FILE>` | Simulate (`--data-dir`, `--capital`, `--monte-carlo N`, `--output`, `--tick-csv`, `--verbose`) |
| `run <FILE>` | Execute on-chain (`--network mainnet\|testnet`, `--dry-run`, `--once`). Reads `DEFI_FLOW_PRIVATE_KEY` from env on startup — never pass it manually. |

## Workflow JSON Structure

```json
{
  "name": "Strategy Name",
  "tokens": { "<SYMBOL>": { "<chain>": "0x<address>" } },
  "contracts": { "<key>": { "<chain>": "0x<address>" } },
  "reserve": { "vault_address": "key", "vault_chain": {...}, "vault_token": "USDC", "target_ratio": 0.20, "trigger_threshold": 0.05 },
  "nodes": [ ... ],
  "edges": [ ... ]
}
```

**tokens** — Token symbol → chain → contract address. Required for all tokens on EVM chains.
**contracts** — Named key → chain → address. Nodes reference by key, not raw address.
**reserve** — Optional vault cash buffer. Unwinds venues pro-rata when below trigger_threshold.

## Node Types (10)

### wallet
Source/sink. `chain`, `token`, `address` (42-char hex).

### perp
Perpetual futures. `venue`: Hyperliquid|Hyena. `pair`: "ETH/USDC". `action`: open|close|adjust|collect_funding. `direction`: long|short. `leverage`: f64.

### options
Rysk on HyperEVM. `asset`: ETH|BTC|HYPE|SOL. `action`: sell_covered_call|sell_cash_secured_put|buy_call|buy_put|collect_premium|roll|close.

### spot
DEX spot trade. `venue`: Hyperliquid|Aerodrome. `pair`: "ETH/USDC". `side`: buy|sell.

### lp
Concentrated liquidity. `venue`: Aerodrome. `pool`: "cbBTC/WETH". `action`: add_liquidity|remove_liquidity|claim_rewards|compound|stake_gauge|unstake_gauge. Optional `tick_lower`, `tick_upper`, `tick_spacing`.

### movement
Swaps, bridges, atomic swap+bridge. `provider`: LiFi|HyperliquidNative. `movement_type`: swap|bridge|swap_bridge. `from_token`, `to_token`, `from_chain`, `to_chain`.

**Providers:**
- **LiFi**: EVM↔EVM. Supports swap, bridge, swap_bridge. NEVER chain two LiFi nodes — use swap_bridge.
- **HyperliquidNative**: HyperCore(1337)↔HyperEVM(999) bridge only. No swaps.

### lending
`archetype`: aave_v3|aave_v2|morpho|compound_v3|init_capital. `pool_address`: contracts key. `asset`: token symbol. `action`: supply|withdraw|borrow|repay|claim_rewards.

### vault
ERC4626 vaults. `archetype`: morpho_v2. `vault_address`: contracts key. `action`: deposit|withdraw|claim_rewards.

### pendle
Yield tokenization. `market`: "PT-kHYPE". `action`: mint_pt|redeem_pt|mint_yt|redeem_yt|claim_rewards.

### optimizer
Kelly Criterion allocator. `kelly_fraction`: 0.5 (half-Kelly). `drift_threshold`: 0.05. `allocations[]` with `target_node` or `target_nodes` (groups). Adaptive mode: omit expected_return/volatility.

## Edges

```json
{ "from_node": "wallet", "to_node": "kelly", "token": "USDC", "amount": { "type": "all" } }
```

Amount: `{"type": "fixed", "value": "1000"}` | `{"type": "percentage", "value": 50.0}` | `{"type": "all"}`

Distribution rules: wallet/optimizer with multiple outgoing edges must use percentage summing to 100%.

## Chains (lowercase)

ethereum (1), arbitrum (42161), optimism (10), base (8453), mantle (5000), hyperevm (999), hyperliquid (1337).

## Validation Rules

- Orphan nodes: every non-wallet node needs incoming edge
- Sink nodes: supply/deposit/open/add_liquidity can't have outgoing edges
- Edge distribution: percentages must sum to 100% on splitter nodes
- No cycles, no self-loops, no duplicate IDs

## Key Gotchas

- Each node gets its OWN simulator — two perp nodes with same pair don't share state
- Funding auto-compounds inside perp margin — no separate collect_funding needed
- Chain names are case-sensitive: lowercase only
- `pool_address`, `vault_address` are manifest key names, NOT raw 0x addresses
- Delta-neutral: spot + short perp at 1x = zero delta. Use `target_nodes` group.

## Example Strategy

```json
{
  "name": "ETH Delta-Neutral Yield Farm v2",
  "tokens": {
    "USDC": { "hyperevm": "0xb88339CB7199b77E23DB6E890353E22632Ba630f" },
    "ETH": { "hyperevm": "0xbe6727b535545c67d5caa73dea54865b92cf7907" }
  },
  "contracts": {
    "hyperlend_pool": { "hyperevm": "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b" },
    "hyperlend_rewards": { "hyperevm": "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F" }
  },
  "nodes": [
    { "type": "wallet", "id": "wallet", "chain": { "name": "hyperliquid", "chain_id": 1337 }, "token": "USDC", "address": "0x0000000000000000000000000000000000000000" },
    { "type": "optimizer", "id": "kelly", "strategy": "kelly", "kelly_fraction": 0.5, "max_allocation": 1.0, "drift_threshold": 0.05, "allocations": [{ "target_nodes": ["buy_eth", "short_eth"], "correlation": 0.0 }, { "target_node": "lend_usdc", "correlation": 0.0 }], "trigger": { "type": "cron", "interval": "weekly" } },
    { "type": "spot", "id": "buy_eth", "venue": "Hyperliquid", "pair": "ETH/USDC", "side": "buy" },
    { "type": "perp", "id": "short_eth", "venue": "Hyperliquid", "pair": "ETH/USDC", "action": "open", "direction": "short", "leverage": 1.0 },
    { "type": "movement", "id": "bridge_usdc", "movement_type": "bridge", "provider": "HyperliquidNative", "from_token": "USDC", "to_token": "USDC", "from_chain": { "name": "hyperliquid", "chain_id": 1337 }, "to_chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" } },
    { "type": "movement", "id": "bridge_eth", "movement_type": "bridge", "provider": "HyperliquidNative", "from_token": "ETH", "to_token": "ETH", "from_chain": { "name": "hyperliquid", "chain_id": 1337 }, "to_chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" } },
    { "type": "lending", "id": "lend_eth", "archetype": "aave_v3", "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" }, "pool_address": "hyperlend_pool", "asset": "ETH", "action": "supply", "rewards_controller": "hyperlend_rewards", "defillama_slug": "hyperlend-pooled" },
    { "type": "lending", "id": "lend_usdc", "archetype": "aave_v3", "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" }, "pool_address": "hyperlend_pool", "asset": "USDC", "action": "supply", "rewards_controller": "hyperlend_rewards", "defillama_slug": "hyperlend-pooled" }
  ],
  "edges": [
    { "from_node": "wallet", "to_node": "kelly", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "kelly", "to_node": "buy_eth", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "kelly", "to_node": "short_eth", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "kelly", "to_node": "bridge_usdc", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "bridge_usdc", "to_node": "lend_usdc", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "buy_eth", "to_node": "bridge_eth", "token": "ETH", "amount": { "type": "all" } },
    { "from_node": "bridge_eth", "to_node": "lend_eth", "token": "ETH", "amount": { "type": "all" } }
  ]
}
```
