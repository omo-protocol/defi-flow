# defi-flow

A workflow engine for DeFi quantitative strategies. Describe a strategy as a JSON DAG of nodes (venues) and edges (token flows) — the engine validates, backtests with Monte Carlo simulation, visualizes, and executes it on-chain. Includes a visual strategy builder UI and an autonomous multi-agent experiment where 7 LLMs compete as quant strategists and hedgefund managers.

## Project Overview

defi-flow has four main components:

1. **CLI Engine** (Rust) — Core workflow engine for validating, backtesting, and executing DeFi strategies. Supports 10 venue types across 6+ EVM chains.
2. **Web UI** (Next.js + WASM) — Visual drag-and-drop strategy builder with wallet auth, real-time validation, and daemon controls.
3. **API Server** (Rust/Axum) — Backend for the UI with JWT auth, encrypted wallet storage, strategy CRUD, and SSE streaming for live daemon output.
4. **Agent Experiment** (Docker + OpenClaw) — 14 autonomous AI agents (7 quant + 7 hedgefund) running on different LLM providers, competing to build and manage DeFi strategies on HyperEVM mainnet.

## Technology Stack

| Layer | Technology |
|-------|-----------|
| Engine | Rust 2024, petgraph (DAG), alloy (EVM), ferrofluid (Hyperliquid SDK) |
| Backtesting | GBM, Ornstein-Uhlenbeck, AR(1) Monte Carlo, Kelly Criterion optimizer |
| API | Axum, SQLite (rusqlite), JWT (jsonwebtoken), AES-256-GCM wallet encryption |
| UI | Next.js 16, React 19, React Flow, Jotai, Tailwind CSS 4, ethers.js |
| WASM | wasm-bindgen — Rust validation compiled to WebAssembly for client-side use |
| Agents | OpenClaw agent framework, 7 LLM providers (Anthropic, Google, MiniMax, Moonshot, Zhipu, OpenRouter) |
| Deployment | Docker, GitHub Actions CI/CD, GHCR, Caddy reverse proxy |
| On-chain | HyperEVM, Base, Arbitrum, Optimism, Mantle, Ethereum |

## Installation

### Prerequisites

- Rust (edition 2024)
- Node.js 18+ and npm
- [Foundry](https://getfoundry.sh/) (`cast` for on-chain interactions)
- wasm-pack (optional, for WASM builds)

### Build the CLI

```bash
cargo build --release
# Binary at target/release/defi-flow
```

### Run the UI

```bash
cd ui
npm install
npx next dev -p 3001
```

### Build WASM (optional)

```bash
wasm-pack build --target web --out-dir ui/pkg --no-default-features --features wasm
```

### Run the API Server

```bash
cargo run -- api --port 8080
```

## Usage

### How LLMs Use It

1. `defi-flow schema` — get the JSON schema. Feed it to the LLM as context.
2. `defi-flow list-nodes` — get all node types and their parameters.
3. LLM outputs a workflow JSON matching the schema.
4. `defi-flow validate workflow.json` — catch errors before execution.
5. `defi-flow fetch-data workflow.json` — pull historical venue data.
6. `defi-flow backtest --data-dir data/ workflow.json` — simulate against historical data.
7. `defi-flow run workflow.json --dry-run` — paper trade, or remove `--dry-run` for real execution.

### Commands

| Command | What it does |
|---------|-------------|
| `schema` | Print JSON schema for workflow definitions |
| `validate <file>` | Validate a workflow JSON |
| `visualize <file>` | Render workflow DAG (ASCII, DOT, SVG, PNG) |
| `list-nodes` | Print all node types and their parameters |
| `example` | Print an example workflow JSON |
| `fetch-data <file>` | Fetch historical venue data for backtesting |
| `backtest <file>` | Backtest against historical CSV data |
| `run <file>` | Execute live on-chain (daemon or single-pass) |
| `ps` | List running strategy daemons |
| `stop <name>` | Stop a running daemon |
| `logs <name>` | View daemon logs |
| `api` | Start the REST API server |

#### backtest

```
--data-dir <dir>       CSV data directory (default: data)
--capital <f64>        Initial capital USD (default: 10000)
--slippage-bps <f64>   Slippage in basis points (default: 10)
--monte-carlo <N>      Run N Monte Carlo simulations
--block-size <N>       Bootstrap block size (default: 10)
--gbm-vol-scale <f64>  GBM volatility scale (default: 1.0)
--verbose              Tick-by-tick output
```

#### run

```
--network mainnet|testnet   (default: testnet)
--dry-run                   Paper trading mode
--once                      Execute once then exit
--state-file <path>         Persist state across restarts (default: state.json)
--slippage-bps <f64>        (default: 50)
```

## Features

### Validation
- Token + chain flow safety: every edge is validated for both token and chain compatibility. Mismatches produce actionable error messages telling the LLM exactly what movement nodes to insert (swap, bridge, or swap_bridge).
- DAG cycle detection (triggered nodes exempt — they form periodic cycles by design)
- Optimizer constraints: kelly_fraction/max_allocation in [0,1], allocations wired to targets
- Perp constraints: direction + leverage required for open/adjust actions

### Backtesting
- Two-phase execution: deploy (topological order) then tick loop
- Per-venue metrics: TWRR, annualized return, max drawdown, Sharpe ratio, net PnL
- Breakdown columns: funding, rewards, premium, LP fees, lending interest, swap costs, liquidations
- JSON output: trajectory (timestamp + TVL per tick), full metrics, optional Monte Carlo results

### Monte Carlo Simulation
- **Parametric**: estimates model parameters from historical data, generates synthetic paths
  - Prices: GBM (Geometric Brownian Motion) with drift + volatility from log-returns
  - Funding rates: OU (Ornstein-Uhlenbeck) mean-reverting process
  - Lending/vault yields: AR(1) autoregressive process
  - LP: tick OU + fee/reward AR(1) + price from shared GBM
- Shared GBM across correlated venues (spot + perp use the same price path)
- Percentile output: 5th/25th/50th/75th/95th for TWRR, drawdown, Sharpe, net PnL
- Value-at-Risk at 95% and 99% confidence

### Kelly Criterion Optimizer
- **Smooth Kelly**: maximizes E[log(1 + f*R)] with integrated risk — `(1-p_loss)*ln(1 + f*(return-cost)) + p_loss*ln(1 - f*severity)`
- Per-venue risk parameters computed automatically from venue data
- Grouped allocations for delta-neutral pairs (spot + perp share one Kelly allocation)
- Adaptive mode: expected return/volatility derived from venue alpha stats
- Fractional Kelly scaling, max allocation cap, drift-based rebalancing

### Live Execution
- Daemon mode: continuous cron-scheduled execution with hot reload
- Paper trading (`--dry-run`): log actions without on-chain execution
- State persistence: deploy status, balances, last tick saved to JSON
- Hyperliquid perp executor via ferrofluid SDK (IOC orders, reduce-only closes)
- Hot reload: parameter-only changes applied instantly without restart

### Venue Simulators

| Venue | Description |
|-------|-------------|
| **Perp** | Isolated margin, funding accrual, rewards, liquidation. Actions: open, close, adjust, collect_funding |
| **Options** | Covered calls/puts via Rysk. Delta-targeted strike selection, expiry settlement |
| **LP** | Uniswap V3 / Aerodrome concentrated liquidity. Fee concentration, IL simulation, gauge rewards |
| **Lending** | Supply/borrow APY, reward emissions. Archetypes: aave_v3, aave_v2, morpho, compound_v3, init_capital |
| **Vault** | ERC4626 yield-bearing vaults (Morpho V2). Deposit APY, reward emissions |
| **Pendle** | PT price appreciation toward par at maturity, YT variable yield |
| **Movement** | Swap, bridge, or atomic swap+bridge via LiFi/Stargate |
| **Spot** | Spot DEX trades |

### Node Types

| Node | Purpose |
|------|---------|
| `wallet` | Source/sink for funds on a chain |
| `perp` | Perpetual futures |
| `options` | Options via Rysk |
| `spot` | Spot DEX trades |
| `lp` | Concentrated liquidity (Aerodrome Slipstream) |
| `movement` | Swap, bridge, or atomic swap+bridge |
| `lending` | Supply, borrow, repay, withdraw, claim |
| `vault` | ERC4626 vault deposits |
| `pendle` | Yield tokenization (PT/YT) |
| `optimizer` | Kelly Criterion capital allocation |

### Supported Chains

EVM: Ethereum, Base, Arbitrum, Optimism, Mantle, HyperEVM
Non-EVM: Hyperliquid (HyperCore) — bridge-only

### Supported Protocols

**Perps:** Hyperliquid, Hyena | **Options:** Rysk | **Spot:** Aerodrome | **LP:** Aerodrome Slipstream | **Swap/Bridge:** LiFi, Stargate | **Lending:** Any Aave V3/V2 fork, Morpho Blue, Compound V3, Init Capital | **Vaults:** Morpho V2 | **Yield:** Pendle

## Visual Strategy Builder (UI)

The web UI at `ui/` provides a drag-and-drop canvas for building strategies visually:

- **React Flow canvas** with 10 node types — drag, connect, configure
- **Real-time validation** via WASM (Rust compiled to WebAssembly, runs client-side)
- **Wallet authentication** — register/login, encrypted private key storage (AES-256-GCM)
- **Strategy management** — save, load, import/export JSON
- **Daemon controls** — start/stop strategy daemons, SSE streaming for live output
- **Example strategies** — pre-built templates in `ui/public/examples/`
- Fully client-side rendering, no backend required for strategy building (only for execution)

```bash
cd ui && npm install && npx next dev -p 3001
```

## Agent Experiment

The `experiment/` directory contains an autonomous multi-agent system where 7 different LLMs compete as DeFi strategists on HyperEVM mainnet.

### Architecture

- **14 agents** total: 7 quant agents + 7 hedgefund agents
- **7 LLM providers**: MiniMax, Qwen, Kimi, GLM, Claude Opus, Gemini, Grok
- **3 vault strategies**: Lending (HyperLend), Delta-Neutral (spot ETH + short perp), PT Fixed Yield (Pendle)
- All agents run as Docker containers via OpenClaw agent framework
- Each agent gets its own wallet, MongoDB database, and memory volume
- Reasoning logs shipped to MongoDB every 15 minutes for analysis

### Agent Types

**Quant Agents** — Autonomous DeFi strategists that scan yield opportunities (DeFiLlama, Hyperliquid), build strategy JSONs, backtest with Monte Carlo, and deploy winning strategies.

**Hedgefund Agents** — Vault managers that deposit capital into Morpho V2 vaults, monitor reserve ratios, track strategy performance, and manage vault health.

### Deployment

```bash
# Build and deploy all containers (via CI/CD)
git push origin main  # triggers GitHub Actions → GHCR → VPS deploy

# Manual deployment
cd experiment
docker compose up -d
```

### Agent Wallets (HyperEVM Mainnet)

#### Quant Agents

| Agent | Model | Address |
|-------|-------|---------|
| quant-minimax | MiniMax-M2.5 | `0xb19e2b26b6777929b2E83360fB65cC7341a3418C` |
| quant-qwen | Qwen3-235B | `0x0392F819133e369E9dE9CD0c098b2055aB89Bba7` |
| quant-kimi | Kimi-K2 | `0x1F6142579af40F1CfEaEB029F258F33B352ed6b8` |
| quant-glm | GLM-5 | `0x99471c1523F0c1A4170B35c3862F293693325db4` |
| quant-opus | Claude Opus 4.6 | `0xE2A58d294b0a049D25AfD6A2C213AEB3a788fd32` |
| quant-gemini | Gemini 3.1 Pro | `0x0F446ae4f9C4E397C0c5862d1778088A2453ce60` |
| quant-grok | Grok 4.1 | `0x21CC816D120104cff7852Ba1d8251777021EBcFb` |

#### Hedgefund Agents

| Agent | Model | Address |
|-------|-------|---------|
| hedgefund-minimax | MiniMax-M2.5 | `0x719c51838f191Ae77C2ad82BC42d03910Db9e860` |
| hedgefund-qwen | Qwen3-235B | `0x49B9a42E19bEde1B087e113BB44Edb5c015515c4` |
| hedgefund-kimi | Kimi-K2 | `0x8aF56201FD649b975fEcfbc8fB0A63FFCa471F96` |
| hedgefund-glm | GLM-5 | `0xEc02b5c21840753a73c63355A7Afa301f01003Ce` |
| hedgefund-opus | Claude Opus 4.6 | `0x7e95AeaDffcdc91225c6242D8A290e3258a0FFcA` |
| hedgefund-gemini | Gemini 3.1 Pro | `0xBC3F0e09dAF5D1887a736e08fA092Fa50fd5aaDf` |
| hedgefund-grok | Grok 4.1 | `0x39499dfa11A88e1a96bfA2e0d1eE935CEB17B45e` |

### Vault Strategies (Mainnet)

| Strategy | Vault | Description |
|----------|-------|-------------|
| Lending | `0x58D0F36A87177a4F1Aa8C2eB6e91d424D7248f1C` | USDT0 → HyperLend supply yield |
| Delta-Neutral | `0x41B5FBB5c6E3938A8536B1d8828a45f7fd839ab6` | Spot ETH + short perp for funding income |
| PT Fixed Yield | `0xe600EB6913376B4Ac7eD645B2bFF8A20B4F8cfB0` | Pendle PT-kHYPE at discount, hold to maturity |

## Project Structure

```
defi-flow/
├── src/                    # Rust engine
│   ├── api/                # Axum REST API + auth
│   ├── backtest/           # Historical simulation
│   ├── engine/             # Workflow execution, reserve management
│   ├── fetch_data/         # Venue data fetching
│   ├── model/              # Data models, workflow types
│   ├── run/                # Live execution, daemon loop, valuer
│   ├── validate/           # DAG validation, token/chain checks
│   └── venues/             # Venue simulators (perp, lp, lending, etc.)
├── ui/                     # Next.js visual strategy builder
│   ├── app/                # Pages and layouts
│   ├── components/         # React components (canvas, panels, toolbar)
│   ├── lib/                # State management, WASM bridge, converters
│   └── pkg/                # WASM build output
├── experiment/             # Agent experiment
│   ├── quant-agent/        # Quant agent config (SOUL, HEARTBEAT, skills)
│   ├── hedgefund-agent/    # Hedgefund agent config
│   ├── vault-strategies/   # Vault strategy configs
│   └── docker-compose.yml  # 14 agents + 3 strategies + API
├── strategies/             # Strategy JSON files
├── data/                   # Historical CSV data
├── ferrofluid/             # Hyperliquid SDK (local copy)
└── .github/workflows/      # CI/CD pipeline
```

## License

MIT
