# defi-flow CLI — Strategy Builder

You are building a DeFi strategy using the `defi-flow` workflow engine. The user will describe a strategy in plain English. Your job is to:

1. Write the workflow JSON (DAG of nodes + edges)
2. Validate it: `target/release/defi-flow validate <file>`
3. Fetch data: `target/release/defi-flow fetch-data <file> --output-dir data/<name> --days 365`
4. Backtest: `target/release/defi-flow backtest <file> --data-dir data/<name> --capital 10000`

User's strategy description: $ARGUMENTS

---

## CLI Commands

| Command | Usage |
|---------|-------|
| `schema` | Output JSON Schema for workflow definitions |
| `validate <FILE>` | Validate workflow JSON (offline + on-chain RPC checks) |
| `visualize <FILE>` | Render graph (`--format ascii\|dot\|svg\|png`, `--output`, `--scope from:to`) |
| `list-nodes` | Print all node types with fields and enums |
| `example` | Print example workflow JSON |
| `fetch-data <FILE>` | Fetch historical data (`--output-dir`, `--days`, `--interval 4h\|8h\|1d`) |
| `backtest <FILE>` | Simulate (`--data-dir`, `--capital`, `--slippage-bps`, `--monte-carlo N`, `--verbose`, `--output`, `--tick-csv`) |
| `run <FILE>` | Execute on-chain (`--network mainnet\|testnet`, `--dry-run`, `--once`, `--state-file`) |

---

## Workflow JSON Structure

```json
{
  "name": "Strategy Name",
  "description": "Optional",
  "tokens": { "<SYMBOL>": { "<chain>": "0x<address>" } },
  "contracts": { "<key>": { "<chain>": "0x<address>" } },
  "reserve": { ... },
  "nodes": [ ... ],
  "edges": [ ... ]
}
```

### Manifests

**tokens** — Maps token symbols to contract addresses per chain. Every token referenced by nodes/edges must have an entry for its chain.

```json
"tokens": {
  "USDC": {
    "hyperevm": "0x2Df1c51E09aECF9cacB7bc98cB1742757f163dF7",
    "base": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
  }
}
```

**contracts** — Maps named contract keys to addresses per chain. Nodes reference contracts by key, not raw address. You pick meaningful names.

```json
"contracts": {
  "hyperlend_pool": { "hyperevm": "0xC0EE4e7e60D0A1F9a9AfaE0706D1b5C5A7f5B9b4" },
  "hyperlend_rewards": { "hyperevm": "0x54586bE62E3c3580375aE3723C145253060Ca0C2" },
  "aerodrome_position_manager": { "base": "0x827922686190790b37229fd06084350E74485b72" },
  "pendle_router": { "hyperevm": "0x00000000005BBB0EF59571E58418F9a4357b68A0" }
}
```

Pendle keys are derived: market `"PT-kHYPE"` -> `pendle_pt_khype_market`, `pendle_pt_khype_sy`, `pendle_pt_khype_yt`.

### Reserve Config (optional)

Vault-based reserve that maintains a cash buffer. On each tick, if reserve drops below `trigger_threshold`, the engine unwinds venues pro-rata to restore `target_ratio`.

```json
"reserve": {
  "vault_address": "morpho_usdc_vault",
  "vault_chain": { "name": "hyperevm", "chain_id": 999 },
  "vault_token": "USDC",
  "target_ratio": 0.20,
  "trigger_threshold": 0.05,
  "min_unwind": 100.0
}
```

- `vault_address`: contracts manifest key (not raw address)
- `target_ratio`: fraction of TVL to keep in reserve (default 0.20)
- `trigger_threshold`: rebalance when reserve falls below this (default 0.05)
- `min_unwind`: minimum USD to unwind per operation (default 100.0)

### Manifest Validation

| Condition | Behavior |
|-----------|----------|
| `tokens` present, entry missing | **Error**: `TokenNotInManifest` |
| `tokens` absent | Silent pass (backtest compat) |
| `contracts` present, entry missing | **Error**: `ContractNotInManifest` |
| `contracts` absent, nodes need contracts | **Warning** to stderr, no error |

---

## Node Types

### wallet
Source/sink for funds.
- `chain`: Chain (required)
- `token`: String — token symbol, must be in tokens manifest (required)
- `address`: String — 0x wallet address (required)

### perp
Perpetual futures.
- `venue`: `Hyperliquid` | `Hyena` (required)
- `pair`: String, e.g. `"ETH/USDC"` (required)
- `action`: `open` | `close` | `adjust` | `collect_funding` (required)
- `direction`: `long` | `short` (required for open/adjust)
- `leverage`: f64 (required for open/adjust)
- `margin_token`: String (optional, default: USDC for Hyperliquid, USDe for Hyena)
- `trigger`: Trigger (optional)

### options
Options trading via Rysk on HyperEVM.
- `venue`: `Rysk` (required)
- `asset`: `ETH` | `BTC` | `HYPE` | `SOL` (required)
- `action`: `sell_covered_call` | `sell_cash_secured_put` | `buy_call` | `buy_put` | `collect_premium` | `roll` | `close` (required)
- `delta_target`: f64 0.0-1.0 (optional)
- `days_to_expiry`: u32 (optional)
- `min_apy`: f64 (optional)
- `batch_size`: u32 (optional)
- `roll_days_before`: u32 (optional)
- `trigger`: Trigger (optional)

### spot
Spot trade on DEX.
- `venue`: `Aerodrome` (required)
- `pair`: String, e.g. `"ETH/USDC"` (required)
- `side`: `buy` | `sell` (required)
- `trigger`: Trigger (optional)

### lp
Concentrated liquidity provision.
- `venue`: `Aerodrome` (required)
- `pool`: String, e.g. `"cbBTC/WETH"` (required)
- `action`: `add_liquidity` | `remove_liquidity` | `claim_rewards` | `compound` | `stake_gauge` | `unstake_gauge` (required)
- `tick_lower`: i32 (optional, omit for full range)
- `tick_upper`: i32 (optional)
- `tick_spacing`: i32 (optional, e.g. 100 for CL100)
- `trigger`: Trigger (optional)
- Requires `aerodrome_position_manager` in contracts manifest

### movement
Token swaps, bridges, or atomic swap+bridge.
- `movement_type`: `swap` | `bridge` | `swap_bridge` (required)
- `provider`: `LiFi` | `HyperliquidNative` (required)
- `from_token`: String (required)
- `to_token`: String (required)
- `from_chain`: Chain (required for bridge/swap_bridge)
- `to_chain`: Chain (required for bridge/swap_bridge)
- `trigger`: Trigger (optional)

**Movement providers:**
- **LiFi**: EVM↔EVM bridges and swaps (Base↔Arbitrum, Base↔HyperEVM). Supports `swap`, `bridge`, and `swap_bridge` (atomic swap+bridge in one node). **NEVER chain two LiFi nodes** — use `swap_bridge` instead.
- **HyperliquidNative**: HyperCore↔HyperEVM only, bridge only (no swaps), uses native `spotSend`. For swaps on Hyperliquid, bridge to HyperEVM first → LiFi swap there → bridge back.
- Base→Hyperliquid = two nodes: LiFi(Base→HyperEVM) + HyperliquidNative(HyperEVM→Hyperliquid). The LiFi node can be `swap_bridge` if tokens also need swapping.

### lending
Lending protocol interactions.
- `archetype`: `aave_v3` | `aave_v2` | `morpho` | `compound_v3` | `init_capital` (required)
- `chain`: Chain (required)
- `pool_address`: String — **contracts manifest key** e.g. `"hyperlend_pool"` (required)
- `asset`: String — token symbol (required)
- `action`: `supply` | `withdraw` | `borrow` | `repay` | `claim_rewards` (required)
- `rewards_controller`: String — **contracts manifest key** (optional)
- `defillama_slug`: String (optional, for fetch-data)
- `trigger`: Trigger (optional)

### vault
Yield-bearing vaults (ERC4626).
- `archetype`: `morpho_v2` (required)
- `chain`: Chain (required)
- `vault_address`: String — **contracts manifest key** e.g. `"morpho_usdc_vault"` (required)
- `asset`: String (required)
- `action`: `deposit` | `withdraw` | `claim_rewards` (required)
- `defillama_slug`: String (optional)
- `trigger`: Trigger (optional)

### pendle
Yield tokenization.
- `market`: String, e.g. `"PT-kHYPE"` (required)
- `action`: `mint_pt` | `redeem_pt` | `mint_yt` | `redeem_yt` | `claim_rewards` (required)
- `trigger`: Trigger (optional)
- Requires in contracts: `pendle_<normalized>_market`, `_sy`, `_yt`, `pendle_router`

### optimizer
Kelly Criterion capital allocator. Adaptive mode: derives expected_return and volatility from venue data automatically.
- `strategy`: `kelly` (required)
- `kelly_fraction`: f64 0.0-1.0 (required, typically 0.5 for half-Kelly)
- `max_allocation`: f64 0.0-1.0 (optional, default 1.0)
- `drift_threshold`: f64 (required, typically 0.05)
- `allocations[]`: array of VenueAllocation (required, at least 1)
- `trigger`: Trigger (optional)

**VenueAllocation:**
- `target_node`: String — single target node ID (use for single venues)
- `target_nodes`: String[] — group of target nodes sharing allocation equally (use for delta-neutral: `["buy_eth", "short_eth"]`)
- `expected_return`: f64 (optional — omit for adaptive mode, derived from venue data)
- `volatility`: f64 (optional — omit for adaptive mode)
- `correlation`: f64 (default 0.0 — correlation with reference asset)

Use `target_nodes` for delta-neutral groups (spot+perp). The optimizer never rebalances between legs within a group.

---

## Edge

```json
{ "from_node": "wallet", "to_node": "kelly", "token": "USDC", "amount": { "type": "all" } }
```

Amount types: `{"type": "fixed", "value": "1000.50"}` | `{"type": "percentage", "value": 50.0}` | `{"type": "all"}`

**Distribution rules:**
- For wallet and optimizer nodes with multiple outgoing edges: use `percentage` type on all edges, must sum to 100%
- Don't mix `all` with `percentage` on the same node's outgoing edges
- `all` means the full amount flows to that single target

## Trigger

```json
{"type": "cron", "interval": "daily"}
```

Intervals: `hourly` | `daily` | `weekly` | `monthly`. Non-triggered nodes execute once during deploy.

## Chain

```json
{"name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm"}
```

Known chains (lowercase): `ethereum` (1), `arbitrum` (42161), `optimism` (10), `base` (8453), `mantle` (5000), `hyperevm` (999), `hyperliquid` (1337, namespace — perps/spot L1).

---

## Validation Rules

The validator catches these errors:
- **Orphan nodes**: Every non-wallet node must have at least one incoming edge
- **Sink nodes**: Nodes with sink actions (`supply`, `deposit`, `open`, `add_liquidity`, `stake_gauge`) cannot have outgoing edges — tokens are locked
- **Edge distribution**: Wallet and optimizer nodes with multiple outgoing edges must use `percentage` amounts summing to 100%
- **Self-loops**: No `from_node == to_node`
- **Cycles**: DAG must be acyclic
- **Duplicate IDs**: Node IDs must be unique
- **On-chain checks** (when using `validate`): RPC connectivity, chain ID verification, contract code existence, LiFi route quoting

---

## Key Gotchas

- Each node gets its **own** venue simulator — two perp nodes with same pair don't share state
- `collect_funding` won't collect from a separate `open` node's position (different simulators)
- Funding auto-compounds inside perp margin — don't need separate collect_funding node
- Manifest maps node IDs to CSV files — re-run `fetch-data` after renaming nodes
- Chain names are **case-sensitive**: use lowercase (`"hyperevm"` not `"HyperEVM"`)
- Swap nodes for non-stablecoins track spot price via perp price feed
- Lending data may start later than perp data -> 0 APY for early ticks
- `pool_address`, `vault_address`, `rewards_controller` are **manifest key names**, not raw 0x addresses
- Delta-neutral: equal-weight spot + short perp at 1x = zero delta. Use `target_nodes` group in optimizer.
