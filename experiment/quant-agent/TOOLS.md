# TOOLS.md — Local Tool Notes

Skills define *how* tools work. This file has the specifics unique to this deployment.

## defi-flow CLI

**Binary:** `/usr/local/bin/defi-flow` (in Docker) or `./defi-flow` (local)

### Commands
```
defi-flow schema              # Print JSON schema for workflow files
defi-flow validate <file>     # Validate strategy JSON (includes on-chain checks)
defi-flow fetch-data <file>   # Download historical data for all nodes
defi-flow backtest <file>     # Run backtest simulation
defi-flow run <file>          # Execute strategy live (ALWAYS use --network mainnet)
defi-flow query <file>        # Query on-chain TVL per venue + wallet balances (JSON output)
defi-flow ps                  # List running strategy daemons
defi-flow stop <name>         # Stop a running daemon
defi-flow logs <name>         # View daemon logs
defi-flow list-nodes          # Show supported node types
defi-flow example             # Print example strategy JSON
```

### Key Flags
```
--network mainnet             # REQUIRED for defi-flow run — default is testnet!
--capital <amount>            # Starting capital (default: 10000)
--monte-carlo <runs>          # MC simulation count (default: 0 = off)
--days <n>                    # Historical data fetch window
--interval <period>           # Data interval (1h, 4h, 1d)
--tick-csv <path>             # Export per-tick venue values
--output <path>               # Export results as JSON
--dry-run                     # Paper trade only (NOT recommended — deploy to mainnet)
--data-dir <path>             # Override data directory
--registry-dir /app/.defi-flow  # ALWAYS pass this for daemon commands (ps, stop, logs, run)
--state-file <path>           # Persistence across restarts
```

### CRITICAL: Always use `--network mainnet`
The default network is **testnet**. If you forget `--network mainnet`, the strategy will run on testnet and no real trades will execute. Every `defi-flow run` command MUST include `--network mainnet`.

### Verifying strategies are live
After deploying a strategy, verify it's actually executing:
```bash
# Check daemon is registered and running
defi-flow ps --registry-dir /app/.defi-flow

# Query on-chain TVL (should be > 0 if deploy succeeded)
defi-flow query /app/strategies/<name>.json \
  --state-file /app/.defi-flow/state/<name>.state.json

# Check logs for errors
defi-flow logs <name> -n 100 --registry-dir /app/.defi-flow
```

### Environment Variables
- `DEFI_FLOW_PRIVATE_KEY` — Wallet private key for on-chain execution. **Read automatically by `defi-flow run` on startup** — you never need to reference or pass it. Set in container env via docker-compose. **NEVER echo, print, or display.**
- `ANTHROPIC_API_KEY` — LLM provider key (used by OpenClaw, not defi-flow). **NEVER echo.**
- `MONGODB_URI` — Log shipping connection string. **NEVER echo.**
- `GATEWAY_AUTH_TOKEN` — OpenClaw gateway auth. **NEVER echo.**

### Critical Gotchas
- Each node gets its OWN venue simulator — two perp nodes with same pair don't share state
- Manifest maps node IDs to CSV files — re-run `fetch-data` after renaming nodes
- Chain names are **case-sensitive**: use lowercase (`hyperevm` not `HyperEVM`)
- Missing manifest entries bail! — don't silently fall back

## Data Sources

### DeFiLlama
- Yields: `https://yields.llama.fi/pools`
- Protocols: `https://api.llama.fi/protocols`
- TVL: `https://api.llama.fi/tvl/{protocol}`

### Hyperliquid
- Funding rates: `https://api.hyperliquid.xyz/info` (POST, `{"type": "metaAndAssetCtxs"}`)
- L1 state: `https://api.hyperliquid.xyz/info` (POST, `{"type": "clearinghouseState", "user": "0x..."}`)

### Chain Name Mapping (DeFiLlama → defi-flow)
| DeFiLlama | defi-flow |
|-----------|-----------|
| Ethereum | ethereum |
| Arbitrum | arbitrum |
| Base | base |
| Optimism | optimism |
| Hyperliquid | hyperliquid |
| HyperEVM | hyperevm |

## Strategies Directory

Saved strategies: `/app/strategies/` (Docker) or `./strategies/` (local)
Historical data: `/app/data/` (Docker) or `./data/` (local)
