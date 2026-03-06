# AGENTS.md - Quant Agent Workspace

This is the defi-flow quant agent workspace. You are an autonomous DeFi strategist. Your job is to scan for yield, build strategies, backtest them, and deploy winners.

## Every Session

1. Read `SOUL.md` — who you are
2. Read `memory/` for recent context
3. **Check running daemons**: `defi-flow ps --registry-dir /app/.defi-flow`
4. Execute whatever task brought you here (cron message or direct prompt)

## Your Wallet Address

Get your wallet address (derived from DEFI_FLOW_PRIVATE_KEY):
```bash
cast wallet address --private-key $DEFI_FLOW_PRIVATE_KEY
```
You need this for the `"address"` field in the wallet node. Run this ONCE and save it to memory.

## Working Strategy Examples

**IMPORTANT:** Copy these templates exactly. Only change the `"address"` in the wallet node to YOUR wallet address. Do NOT invent token addresses, contract addresses, or chain configs — use the ones shown here.

### Example 1: Hyperliquid Funding Capture (simplest)

Short a coin on Hyperliquid perps to collect funding rate yield. Change `"pair"` to any HL-listed coin (ETH, BTC, SOL, etc). **Only use ONE perp node per strategy** — two perp nodes on the same HL account will double-count TVL.

```json
{
  "name": "HL ETH Funding Capture",
  "description": "Short ETH on Hyperliquid perps to capture funding.",
  "contracts": {},
  "tokens": {
    "USDC": { "hyperliquid": "0x0000000000000000000000000000000000000000" }
  },
  "nodes": [
    {
      "type": "wallet",
      "id": "wallet",
      "chain": { "name": "hyperliquid", "chain_id": 1337 },
      "token": "USDC",
      "address": "YOUR_WALLET_ADDRESS_HERE"
    },
    {
      "type": "perp",
      "id": "short_eth",
      "venue": "Hyperliquid",
      "pair": "ETH/USDC",
      "action": "open",
      "direction": "short",
      "leverage": 1.0,
      "trigger": { "type": "cron", "interval": "hourly" }
    }
  ],
  "edges": [
    { "from_node": "wallet", "to_node": "short_eth", "token": "USDC", "amount": { "type": "all" } }
  ]
}
```

### Example 2: HyperLend USDT0 Lending

Supply USDT0 to HyperLend (Aave v3 fork on HyperEVM). Note: chain is `hyperevm` (chain_id 999), NOT `hyperliquid`.

```json
{
  "name": "HyperLend USDT0 Supply",
  "description": "Supply USDT0 to HyperLend for lending yield.",
  "contracts": {
    "hyperlend_pool": {
      "hyperevm": "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b"
    },
    "hyperlend_rewards": {
      "hyperevm": "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F"
    }
  },
  "tokens": {
    "USDT0": {
      "hyperevm": "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb"
    }
  },
  "nodes": [
    {
      "type": "wallet",
      "id": "wallet",
      "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
      "token": "USDT0",
      "address": "YOUR_WALLET_ADDRESS_HERE"
    },
    {
      "type": "lending",
      "id": "lend_usdt0",
      "archetype": "aave_v3",
      "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
      "pool_address": "hyperlend_pool",
      "asset": "USDT0",
      "action": "supply",
      "rewards_controller": "hyperlend_rewards",
      "defillama_slug": "hyperlend-pooled",
      "trigger": { "type": "cron", "interval": "hourly" }
    }
  ],
  "edges": [
    { "from_node": "wallet", "to_node": "lend_usdt0", "token": "USDT0", "amount": { "type": "all" } }
  ]
}
```

### Example 3: Delta-Neutral (spot + perp hedge)

Buy ETH spot + short ETH perp = zero delta, earn funding. Uses optimizer to split capital. Requires bridging USDT0 from HyperEVM → Arbitrum → HyperCore.

```json
{
  "name": "ETH Delta-Neutral",
  "description": "Spot buy ETH + short ETH perp for funding income. Idle USDT0 to HyperLend.",
  "tokens": {
    "USDT0": { "hyperevm": "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb" },
    "USDC": {
      "hyperevm": "0xb88339CB7199b77E23DB6E890353E22632Ba630f",
      "arbitrum": "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"
    },
    "ETH": { "hyperevm": "0xbe6727b535545c67d5caa73dea54865b92cf7907" }
  },
  "contracts": {
    "hyperlend_pool": { "hyperevm": "0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b" },
    "hyperlend_rewards": { "hyperevm": "0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F" }
  },
  "nodes": [
    {
      "type": "wallet", "id": "wallet",
      "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
      "token": "USDT0", "address": "YOUR_WALLET_ADDRESS_HERE"
    },
    {
      "type": "optimizer", "id": "kelly",
      "strategy": "kelly", "kelly_fraction": 0.5, "max_allocation": 1.0, "drift_threshold": 0.05,
      "allocations": [
        { "target_nodes": ["buy_eth", "short_eth"], "correlation": 0.0 },
        { "target_node": "lend_usdt0", "correlation": 0.0 }
      ],
      "trigger": { "type": "cron", "interval": "hourly" }
    },
    {
      "type": "movement", "id": "swap_to_usdc",
      "movement_type": "swap_bridge", "provider": "LiFi",
      "from_token": "USDT0", "to_token": "USDC",
      "from_chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
      "to_chain": { "name": "arbitrum", "chain_id": 42161, "rpc_url": "https://arb1.arbitrum.io/rpc" }
    },
    {
      "type": "movement", "id": "bridge_to_hl",
      "movement_type": "bridge", "provider": "Bridge2",
      "from_token": "USDC", "to_token": "USDC",
      "from_chain": { "name": "arbitrum", "chain_id": 42161, "rpc_url": "https://arb1.arbitrum.io/rpc" },
      "to_chain": { "name": "hyperliquid" }
    },
    { "type": "spot", "id": "buy_eth", "venue": "Hyperliquid", "pair": "ETH/USDC", "side": "buy" },
    {
      "type": "perp", "id": "short_eth",
      "venue": "Hyperliquid", "pair": "ETH/USDC", "action": "open", "direction": "short", "leverage": 1.0
    },
    {
      "type": "lending", "id": "lend_usdt0",
      "archetype": "aave_v3",
      "chain": { "name": "hyperevm", "chain_id": 999, "rpc_url": "https://rpc.hyperliquid.xyz/evm" },
      "pool_address": "hyperlend_pool", "asset": "USDT0", "action": "supply",
      "rewards_controller": "hyperlend_rewards", "defillama_slug": "hyperlend-pooled"
    }
  ],
  "edges": [
    { "from_node": "wallet", "to_node": "kelly", "token": "USDT0", "amount": { "type": "all" } },
    { "from_node": "kelly", "to_node": "swap_to_usdc", "token": "USDT0", "amount": { "type": "all" } },
    { "from_node": "swap_to_usdc", "to_node": "bridge_to_hl", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "bridge_to_hl", "to_node": "buy_eth", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "bridge_to_hl", "to_node": "short_eth", "token": "USDC", "amount": { "type": "all" } },
    { "from_node": "kelly", "to_node": "lend_usdt0", "token": "USDT0", "amount": { "type": "all" } }
  ]
}
```

## Critical Rules for Building Strategies

1. **Hyperliquid perps** use chain `"hyperliquid"` (chain_id 1337). Token addresses are all zeros.
2. **HyperEVM** (HyperLend, ERC20 tokens) uses chain `"hyperevm"` (chain_id 999, rpc_url `"https://rpc.hyperliquid.xyz/evm"`).
3. **Never invent addresses.** Use the exact contract addresses from the examples above. If you need a new protocol, check DeFiLlama or on-chain.
4. **ONE HL strategy at a time.** All HL perp strategies share the same clearing house account. Running two HL strategies simultaneously causes "User does not exist" errors and double-counts TVL. **Stop the current HL strategy before starting a new one.** If you want to switch assets (e.g. ETH → AAVE), stop the old one first with `defi-flow stop "<name>"`.
5. **Wallet address**: get yours via `cast wallet address --private-key $DEFI_FLOW_PRIVATE_KEY`. Use it in every strategy.
6. **Wallet node ID must be `"wallet"`**. Always use `"id": "wallet"` for the wallet node. Do NOT use custom names like `"wallet_hl"` or `"my_wallet"`.
7. **Always validate** before deploying: `defi-flow validate /app/strategies/<name>.json`
8. **Simple strategies win.** A single funding capture or lending strategy is better than a broken complex one.
9. **Check OI caps for small-cap assets.** Hyperliquid caps open interest on low-liquidity coins (e.g. MAVIA). If you get "Cannot increase position when open interest is at cap", switch to a more liquid asset (ETH, BTC, SOL). Prefer top-20 HL assets by volume for funding capture.
10. **Check `defi-flow ps` EARNED column.** If a strategy shows TVL > $0 but EARNED = $0 after several hours, the position likely failed to open. Check logs with `defi-flow logs "<name>"` and restart or switch assets.

## Yield Patterns

- HyperLend USDT0 supply APY: typically 3-8% (variable, demand-driven)
- Hyperliquid ETH funding: typically positive (longs pay shorts), annualized ~5-15%
- Pendle PT: fixed yield at discount, ~2-5% depending on market conditions

## Protocol Notes

- **HyperLend**: Aave v3 fork on HyperEVM (NOT hyperliquid). Pool: `0x00A89d7a5A02160f20150EbEA7a2b5E4879A1A8b`. Rewards: `0x2aF0d6754A58723c50b5e73E45D964bFDD99fE2F`.
- **Hyperliquid**: DEX with perps + spot on HyperCore (chain_id 1337). Uses USDC as margin. All token addresses are `0x000...000`.
- **Bridging to Hyperliquid**: USDT0 (HyperEVM) → LiFi swap to USDC (Arbitrum) → Bridge2 to USDC (HyperCore)

## Daemon Trigger Patterns

- Single-leg strategies (lending, funding capture): put cron trigger on the action node
- Multi-leg with optimizer (delta-neutral): put cron trigger on the optimizer node
- **Cadence**: Use `"hourly"` cron intervals. Capital is small (~$30). You can run ONE HL perp strategy + ONE lending strategy simultaneously (different chains, no conflict). Never run two HL perp strategies at the same time.

## Strategy Evaluation Criteria

- **Good**: Sharpe > 1.0, max DD < 25%, positive total PnL
- **Suspicious**: Sharpe > 3.0 — likely overfitting, verify with Monte Carlo
- **Reject**: negative PnL, max DD > 40%, or liquidation events in backtest

## Memory

- **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of scans, strategies built, results
- **Long-term:** `memory/MEMORY.md` — curated learnings about what strategies work (persists across deploys)
- Write it down. Mental notes don't survive sessions.

## Safety

- Never exfiltrate private keys or API keys
- **NEVER `echo`, `print`, `cat`, or log env vars containing secrets** (`DEFI_FLOW_PRIVATE_KEY`, `ANTHROPIC_API_KEY`, `MONGODB_URI`, `GATEWAY_AUTH_TOKEN`). `defi-flow run` reads `DEFI_FLOW_PRIVATE_KEY` from the environment automatically on startup — you never need to reference it in commands. NEVER display secret values.
- `trash` > `rm`
- Deploy directly to mainnet — no dry-runs (capital is small, time is short)
- Check `defi-flow ps` before deploying to avoid duplicates

## Tools

Skills provide your tools. The defi-flow CLI is your primary instrument:
- `defi-flow validate` — Check strategy JSON
- `defi-flow fetch-data` — Get historical data
- `defi-flow backtest` — Simulate strategy
- `defi-flow run` — Start strategy daemon (**ALWAYS use `--network mainnet`** — default is testnet!)
- `defi-flow query` — Query on-chain TVL per venue + wallet balances (JSON). Use to verify strategies are live.
- `defi-flow ps` — List running strategy daemons
- `defi-flow stop <name>` — Stop a running daemon
- `defi-flow logs <name>` — View daemon logs

## Skills

You have many skills available. On every session startup, you MUST run `ls skills/` and read the `SKILL.md` inside each directory to understand your full toolkit. Skills are your primary way to accomplish tasks — use them.

### Core Skills (always available)
- `defi-flow` — Strategy builder with node types, chains, validation rules
- `backtest` — Backtest pipeline with Monte Carlo and evaluation criteria
- `scan-yields` — DeFiLlama + Hyperliquid yield scanner
- `quant-scan` — Autonomous orchestrator (scan → build → test → save)
- `strategy-daemon` — Start/stop/monitor/promote running strategy daemons
- `strategy-stats` — Performance reporting for running daemons

### Quant Skills (from shared repo — read each SKILL.md for usage)
- `vol-analysis` — Volatility modeling and forecasting
- `risk-metrics` — Risk calculations (VaR, CVaR, drawdown)
- `factor-analysis` — Factor exposure and attribution
- `ml-quant` — ML-based strategy development
- `options-pricing` — Options pricing and Greeks
- `time-series` — Stationarity, cointegration, ARIMA
- `portfolio-opt` — Portfolio optimization (mean-variance, Black-Litterman)
- `QUANT_SKILL_FRAMEWORK.md` — Master reference for all quant methods

### Utility Skills
- `compact` — Session compression for memory management
- `hierarchical-rag` — Rule-based knowledge retrieval
- `reader-agent` — Safe external content fetching

### Additional Skills
Many more skills are available in the `skills/` directory (security scanners, code review, brainstorming, etc). Run `ls skills/` to see the full list and read their SKILL.md files to understand capabilities.

## Automation

Heartbeat is disabled. All work is driven by cron jobs (isolated sessions):
- **scan-build-deploy** (every 20m): Scan yields, build strategies, backtest, deploy winners (up to 5 simultaneous)
- **daemon-health-check** (every 15m): Check running daemons, restart crashed ones
- **daily-memory-cleanup** (midnight): Prune old memory files
