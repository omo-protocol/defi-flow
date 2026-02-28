# defi-flow

Workflow engine for DeFi quant strategies. An LLM describes a strategy as a JSON DAG of nodes (venues) and edges (token flows). The engine validates, backtests, visualizes, and executes it on-chain.

## How LLMs Use It

1. `defi-flow schema` — get the JSON schema. Feed it to the LLM as context.
2. `defi-flow list-nodes` — get all node types and their parameters.
3. LLM outputs a workflow JSON matching the schema.
4. `defi-flow validate workflow.json` — catch errors before execution.
5. `defi-flow backtest --data-dir data/ workflow.json` — simulate against historical data.
6. `defi-flow run workflow.json --dry-run` — paper trade live, or remove `--dry-run` for real execution.

## Commands

| Command | What it does |
|---------|-------------|
| `schema` | Print JSON schema for workflow definitions |
| `validate <file>` | Validate a workflow JSON |
| `visualize <file>` | Render workflow DAG (ASCII, DOT, SVG, PNG) |
| `list-nodes` | Print all node types and their parameters |
| `example` | Print an example workflow JSON |
| `backtest <file>` | Backtest against historical CSV data |
| `fetch-data <file>` | Fetch historical venue data for backtesting |
| `run <file>` | Execute live on-chain (daemon or single-pass) |

### visualize

```
--format ascii|dot|svg|png   Output format (default: ascii)
--scope from_node:to_node    Render only the subgraph between two nodes
-o, --output <path>          Write to file (required for svg/png)
```

### backtest

```
--data-dir <dir>       CSV data directory (default: data)
--capital <f64>        Initial capital USD (default: 10000)
--slippage-bps <f64>   Slippage in basis points (default: 10)
--monte-carlo <N>      Run N Monte Carlo simulations
--block-size <N>       Bootstrap block size (default: 10)
--gbm-vol-scale <f64>  GBM volatility scale (default: 1.0)
--verbose              Tick-by-tick output
```

### run

```
--network mainnet|testnet   (default: testnet)
--dry-run                   Paper trading mode
--once                      Execute once then exit
--state-file <path>         Persist state across restarts (default: state.json)
--slippage-bps <f64>        (default: 50)
```

## Features

### Validation
- **Token + chain flow safety**: every edge is validated for both token and chain compatibility. Nodes declare what they output and what they expect as input. Mismatches produce actionable error messages telling the LLM exactly what movement nodes to insert (swap, bridge, or swap_bridge)
- DAG cycle detection (triggered nodes exempt — they form periodic cycles by design)
- Optimizer constraints: kelly_fraction/max_allocation in [0,1], allocations wired to targets
- Perp constraints: direction + leverage required for open/adjust actions
- Duplicate node ID detection, edge reference checks, bridge same-chain rejection

Example: connecting a wallet on HyperEVM directly to Aerodrome LP on Base:
```
Edge wallet->aero_lp: chain mismatch: 'wallet' outputs USDC on hyperevm,
but 'aero_lp' expects it on base.
Insert a Movement(bridge, from_chain: hyperevm, to_chain: base, token: USDC)
```

### Backtesting
- Two-phase execution: deploy (topological order) then tick loop
- Per-venue metrics: TWRR, annualized return, max drawdown, Sharpe ratio, net PnL
- Breakdown columns: funding, rewards, premium, LP fees, lending interest, swap costs, liquidations
- **JSON output**: trajectory (timestamp + TVL per tick), full metrics, optional Monte Carlo results — pipe to `scripts/plot_backtest.py` for visualization
- Configurable slippage and random seed for reproducibility

### Monte Carlo
- **Parametric simulation**: estimates model parameters from historical data, generates synthetic paths
  - Prices: GBM (Geometric Brownian Motion) with drift + volatility from log-returns
  - Funding rates: OU (Ornstein-Uhlenbeck) mean-reverting process
  - Lending/vault yields: AR(1) autoregressive process
  - LP: tick OU + fee/reward AR(1) + price from shared GBM
- Shared GBM across correlated venues (spot + perp use the same price path per simulation)
- Percentile output: 5th/25th/50th/75th/95th for TWRR, drawdown, Sharpe, net PnL
- Value-at-Risk at 95% and 99% confidence

**Why MC results diverge from historical:**
- Funding rates have high variance relative to mean (typical: stdev 1.5x mean). The OU process generates paths with extended negative funding periods where shorts pay instead of earn.
- GBM with crypto-level vol (50-80% annual) creates extreme price paths that cause rebalancing drag (buy high, sell low between rebalance intervals) and occasional liquidation of leveraged positions.
- Historical backtest reflects one realized path. A Sharpe of 10+ on a single path doesn't mean the strategy is low-risk — MC median Sharpe of 0.3-0.5 is more realistic.
- If a venue has sparse/zero data (e.g. lending APY starts late), adaptive Kelly sees 0% return and allocates nothing there, concentrating risk in fewer venues.

### Venue Simulators

**Perps** — Isolated margin, funding accrual (8h periods), rewards accrual, liquidation at 1% maintenance margin (2% penalty), bid/ask slippage modeling. Actions: open, close, adjust, collect_funding.

**Options** — Covered calls and cash-secured puts via Rysk. Delta-targeted strike selection, days-to-expiry filtering, min APY gating, batch sizing for RFQ. Expiry settlement with intrinsic value. Actions: sell_covered_call, sell_cash_secured_put, buy_call, buy_put, collect_premium, roll, close.

**LP** — Uniswap V3 / Aerodrome Slipstream concentrated liquidity. Tick-to-sqrt-price math, fee concentration multiplier for tighter ranges, in/out-of-range tracking, IL simulation, reward token accrual. Actions: add_liquidity, remove_liquidity, claim_rewards, compound, stake_gauge, unstake_gauge.

**Lending** — Supply APY accrual, borrow APY accrual, reward emissions. Actions: supply, withdraw, borrow, repay, claim_rewards. Archetype-based: `aave_v3`, `aave_v2`, `morpho`, `compound_v3`, `init_capital` — any Aave fork works with the right archetype + pool address. Addresses and chain specified per-deployment in the JSON, not hardcoded.

**Vaults** — ERC4626 yield-bearing vaults. Deposit APY accrual, reward emissions. Actions: deposit, withdraw, claim_rewards. Currently supports Morpho Vaults V2. Live executor uses the ERC4626 interface (deposit/withdraw).

**Pendle** — PT (principal token) price appreciation toward 1:1 at maturity. YT (yield token) variable yield accrual. Actions: mint_pt, redeem_pt, mint_yt, redeem_yt, claim_rewards.

**Movement** — Unified swap/bridge/swap+bridge node. Fixed slippage + fee model. Swap costs tracked as metric. Bridge fee deduction on cross-chain transfers. Three movement types: `swap` (same-chain token conversion), `bridge` (cross-chain same-token), `swap_bridge` (atomic cross-chain swap via LiFi). Providers: LiFi (swap, bridge, swap_bridge), Stargate (bridge only).

### Kelly Optimizer
- **Smooth Kelly**: maximizes E[log(1 + f*R)] with integrated risk — `(1-p_loss)*ln(1 + f*(return-cost)) + p_loss*ln(1 - f*severity)`. Grid search + golden-section refinement. Falls back to classic `f* = return/vol^2` when risk params are zero.
- Per-venue risk parameters: `p_loss` (annualized catastrophic loss probability), `loss_severity` (fraction lost), `rebalance_cost` (per-rebalance friction). Computed automatically from venue data (perp: liquidation probability from price vol; LP: out-of-range probability from tick history).
- **Grouped allocations**: `target_nodes` groups venues that share one Kelly allocation (e.g. spot+perp delta-neutral pair split equally). Group risk detects hedged positions and reduces severity accordingly.
- **Adaptive mode**: when `expected_return`/`volatility` are omitted from allocations, computed from venue `alpha_stats()` (funding rates, lending APY, fee yields, etc.)
- Fractional Kelly scaling (e.g. half-Kelly = 0.5), max allocation cap per venue
- Drift-based rebalancing: only rebalance when actual vs target allocation exceeds threshold
- Periodic rebalance via cron trigger

### Live Execution
- Daemon mode: continuous cron-scheduled execution
- Single-pass mode (`--once`): execute all due triggers then exit
- Paper trading (`--dry-run`): log actions without on-chain execution
- State persistence: deploy status, balances, last tick saved to JSON — safe restarts
- Hyperliquid perp executor via ferrofluid SDK (IOC orders, reduce-only closes)

### Hot Reload
- File watcher (notify crate) on workflow JSON in daemon mode
- On change: re-read, re-validate, check structural compatibility
- Parameter-only changes applied instantly (leverage, kelly_fraction, tick ranges, trigger intervals, etc.)
- Structural changes (add/remove nodes or edges) rejected with warning — requires restart
- Debounced for atomic saves (vim, emacs)

### Visualization
- ASCII: terminal-friendly layered DAG with edge labels
- DOT: Graphviz format with node type shapes (house/box/parallelogram/diamond), color-coded by venue, dashed borders for triggered nodes, dark theme
- SVG/PNG: shells out to system `dot` command
- Scoping: `--scope A:B` renders only nodes on paths from A to B (BFS intersection)

### Data Fetching
- Pull historical data from venue APIs (Hyperliquid, DefiLlama)
- Configurable time range and interval
- CSV output with manifest.json tracking node-to-file mappings
- Perp data: mark/index price, bid/ask, funding APY, rewards APY
- Options data: spot price, strike, expiry, delta, premium, APY
- LP data: token prices, current tick, fee APY, reward rate
- Lending data: supply/borrow/reward APY (via DefiLlama, keyed by defillama_slug)
- Vault data: base APY + reward APY (via DefiLlama)
- Pendle data: PT/YT/underlying price, implied APY

### Multi-Chain
- EVM chains: Ethereum, Base, Arbitrum, Optimism, Mantle, HyperEVM
- Non-EVM: Hyperliquid (HyperCore) — bridge-only, no swap via LiFi
- Custom chain support with chain_id + rpc_url
- Cross-chain validation enforced at the edge level

## Node Types

| Node | Purpose |
|------|---------|
| `wallet` | Source/sink for funds on a chain |
| `perp` | Perpetual futures — open, close, adjust, collect funding |
| `options` | Options via Rysk — covered calls, puts, premium, rolling |
| `spot` | Spot DEX trades |
| `lp` | Concentrated liquidity (Aerodrome Slipstream) |
| `movement` | Swap, bridge, or atomic swap+bridge (LiFi, Stargate) |
| `lending` | Supply, borrow, repay, withdraw, claim (Aave forks, Morpho, Compound, etc.) |
| `vault` | ERC4626 vault deposits — deposit, withdraw, claim (Morpho Vaults V2) |
| `pendle` | Yield tokenization — mint/redeem PT and YT |
| `optimizer` | Kelly Criterion capital allocation with drift-based rebalancing |

## Edges

Edges connect nodes and carry tokens. Amount types: `fixed` (exact value), `percentage`, or `all`.

## Triggers

Any venue node can have a `trigger` for periodic execution:
- `cron`: hourly, daily, weekly, monthly
- `on_event`: fire on external event

Triggered nodes can form cycles (e.g. claim rewards -> swap -> optimizer).

## Supported Venues

**Perps:** Hyperliquid, Hyena
**Options:** Rysk
**Spot:** Aerodrome
**LP:** Aerodrome Slipstream
**Movement (swap/bridge/swap+bridge):** LiFi, Stargate
**Lending:** Any Aave V3/V2 fork (HyperLend, Lendle, Granary, Seamless, Spark, etc.), Morpho Blue, Compound V3, Init Capital
**Vaults:** Morpho Vaults V2
**Yield:** Pendle

## Agent Wallets

All agents run on HyperEVM mainnet (chain ID 999).

### Quant Agents

| Agent | Model | Address |
|-------|-------|---------|
| quant-minimax | MiniMax-M2.5 | `0xb19e2b26b6777929b2E83360fB65cC7341a3418C` |
| quant-qwen | Qwen3-235B | `0x0392F819133e369E9dE9CD0c098b2055aB89Bba7` |
| quant-kimi | Kimi-K2 | `0x1F6142579af40F1CfEaEB029F258F33B352ed6b8` |
| quant-glm | GLM-5 | `0x99471c1523F0c1A4170B35c3862F293693325db4` |
| quant-opus | Claude Opus 4.6 | `0xE2A58d294b0a049D25AfD6A2C213AEB3a788fd32` |
| quant-gemini | Gemini 3.1 Pro | `0x0F446ae4f9C4E397C0c5862d1778088A2453ce60` |
| quant-grok | Grok 4.1 | `0x21CC816D120104cff7852Ba1d8251777021EBcFb` |

### Hedgefund Agents

| Agent | Model | Address |
|-------|-------|---------|
| hedgefund-minimax | MiniMax-M2.5 | `0x719c51838f191Ae77C2ad82BC42d03910Db9e860` |
| hedgefund-qwen | Qwen3-235B | `0x49B9a42E19bEde1B087e113BB44Edb5c015515c4` |
| hedgefund-kimi | Kimi-K2 | `0x8aF56201FD649b975fEcfbc8fB0A63FFCa471F96` |
| hedgefund-glm | GLM-5 | `0xEc02b5c21840753a73c63355A7Afa301f01003Ce` |
| hedgefund-opus | Claude Opus 4.6 | `0x7e95AeaDffcdc91225c6242D8A290e3258a0FFcA` |
| hedgefund-gemini | Gemini 3.1 Pro | `0xBC3F0e09dAF5D1887a736e08fA092Fa50fd5aaDf` |
| hedgefund-grok | Grok 4.1 | `0x39499dfa11A88e1a96bfA2e0d1eE935CEB17B45e` |
