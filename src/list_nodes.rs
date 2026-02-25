/// Print a human-readable listing of all available node types and their parameters.
pub fn run() -> anyhow::Result<()> {
    let listing = r#"Available Node Types
====================

1. wallet
   Source or sink for funds on a specific chain.
   Parameters:
     - chain:   Chain   (object: {"name":"ethereum","chain_id":1,"rpc_url":"https://eth.llamarpc.com"} — chain_id and rpc_url optional for non-EVM chains like {"name":"hyperliquid"})
     - token:   String  (token symbol, e.g. "USDC", "USDT0" — must match an entry in the workflow tokens manifest)
     - address: String  (0x-prefixed wallet address)

2. perp
   Perpetual futures — full lifecycle: open, close, adjust, collect funding.
   Parameters:
     - venue:     PerpVenue     (Hyperliquid | Hyena)
     - pair:      String        (e.g. "ETH/USDC")
     - action:    PerpAction    (open | close | adjust | collect_funding)
     - direction:     PerpDirection (long | short)       — required for open/adjust
     - leverage:      f64           (e.g. 5.0)           — required for open/adjust
     - margin_token:  String?       (margin/collateral token, default: "USDC" for Hyperliquid, "USDe" for Hyena)
                                     Note: Hyperliquid USDC lives on HyperCore; must be bridged via
                                     Stargate, not swappable via LiFi.
     - trigger:       Trigger?      (optional, for periodic actions)

3. options
   Options trading via Rysk on HyperEVM.
   Covered calls, cash-secured puts, premium collection, rolling.
   The LLM specifies strategy params; execution layer picks strike/expiry via RFQ.
   Parameters:
     - venue:            OptionsVenue (Rysk)
     - asset:            RyskAsset    (ETH | BTC | HYPE | SOL)
     - action:           OptionsAction (sell_covered_call | sell_cash_secured_put | buy_call | buy_put | collect_premium | roll | close)
     - delta_target:     f64?         (0.0-1.0, e.g. 0.3 = 30-delta) — for sell/buy/roll
     - days_to_expiry:   u32?         (e.g. 30 for monthly) — for sell/buy/roll
     - min_apy:          f64?         (e.g. 0.05 = 5% min) — skip options below this
     - batch_size:       u32?         (e.g. 10 = 10 token increments for RFQ)
     - roll_days_before: u32?         (e.g. 3 = roll 3 days before expiry)
     - trigger:          Trigger?     (e.g. sell weekly, collect premium daily, roll weekly)

4. spot
   Spot trade on a decentralized exchange.
   Parameters:
     - venue:   SpotVenue (Hyperliquid)
     - pair:    String    (e.g. "ETH/USDC")
     - side:    SpotSide  (buy | sell)
     - trigger: Trigger?  (optional)

5. movement
   Token movement — swap, bridge, or atomic swap+bridge.
   Unifies same-chain swaps, cross-chain bridges, and atomic cross-chain swaps.
   Parameters:
     - movement_type: MovementType     (swap | bridge | swap_bridge)
     - provider:      MovementProvider (LiFi | Stargate)
     - from_token:    String           (e.g. "USDe", "AERO")
     - to_token:      String           (e.g. "USDC" — same as from_token for bridge)
     - from_chain:    Chain?           (source chain, optional for same-chain swaps)
     - to_chain:      Chain?           (destination chain — required for bridge/swap_bridge)
     - trigger:       Trigger?         (optional)
   Movement types:
     - swap:        Same-chain token conversion (e.g. AERO → USDC on Base).
                    Providers: LiFi.
     - bridge:      Cross-chain same-token transfer (e.g. USDe Mantle → USDe Hyperliquid).
                    Providers: LiFi, Stargate.
     - swap_bridge: Atomic cross-chain swap + bridge (e.g. AERO on Base → USDC on HyperEVM).
                    Providers: LiFi.

6. lending
   Lending protocol — supply, borrow, repay, withdraw, claim rewards.
   Execution layer handles protocol-specific details (Morpho market IDs,
   HyperLend E-mode, Init Capital vault shares, etc.)
   Parameters:
     - archetype:           LendingArchetype (aave_v3 | aave_v2 | morpho | compound_v3 | init_capital)
     - chain:               Chain   (e.g. {{"name":"hyperevm","chain_id":999}})
     - pool_address:        String  (contracts manifest key, e.g. "hyperlend_pool")
     - asset:               String  (e.g. "USDC", "WETH", "USDe")
     - action:              LendingAction (supply | withdraw | borrow | repay | claim_rewards)
     - rewards_controller:  String? (optional contracts manifest key, e.g. "hyperlend_rewards")
     - defillama_slug:      String? (optional DefiLlama project slug for fetch-data)
     - trigger:             Trigger? (optional, e.g. claim_rewards daily)

7. vault
   Yield-bearing vault — deposit, withdraw, claim rewards.
   Execution layer handles protocol-specific details (ERC4626 interface, etc.)
   Parameters:
     - archetype:           VaultArchetype (morpho_v2)
     - chain:               Chain   (e.g. {{"name":"ethereum","chain_id":1}})
     - vault_address:       String  (contracts manifest key, e.g. "morpho_usdc_vault")
     - asset:               String  (e.g. "USDC", "WETH")
     - action:              VaultAction (deposit | withdraw | claim_rewards)
     - defillama_slug:      String? (optional DefiLlama project slug for fetch-data)
     - trigger:             Trigger? (optional, e.g. claim_rewards daily)

8. pendle
   Pendle yield tokenization — mint/redeem principal tokens (PT) for fixed yield
   or yield tokens (YT) for variable yield. Used in strategies like PT-kHYPE looping.
   Execution layer handles Pendle router interactions and market lookups.
   Parameters:
     - market:  String       (e.g. "PT-kHYPE", "PT-stETH", "PT-eETH")
     - action:  PendleAction (mint_pt | redeem_pt | mint_yt | redeem_yt | claim_rewards)
     - trigger: Trigger?     (optional, e.g. claim_rewards weekly)

9. lp
   Concentrated liquidity provision on Aerodrome Slipstream (Base).
   Deposit into LP positions, claim gauge rewards, compound fees.
   The pool field is "TOKEN0/TOKEN1" — both tokens must be in the tokens manifest.
   Requires "aerodrome_position_manager" in the contracts manifest for live execution.
   Parameters:
     - venue:        LpVenue    (Aerodrome)
     - pool:         String     (e.g. "WETH/USDC", "cbBTC/WETH")
     - action:       LpAction   (add_liquidity | remove_liquidity | claim_rewards | compound | stake_gauge | unstake_gauge)
     - tick_lower:   i32?       (lower tick bound, full range if omitted)
     - tick_upper:   i32?       (upper tick bound, full range if omitted)
     - tick_spacing: i32?       (tick spacing for the pool, e.g. 100)
     - chain:        Chain?     (defaults to Base if omitted)
     - trigger:      Trigger?   (optional, e.g. compound daily, claim_rewards weekly)

10. optimizer
   Capital allocation optimizer using Kelly Criterion.
   With a trigger, periodically checks allocations and rebalances if drift > threshold.
   Parameters:
     - strategy:        OptimizerStrategy (kelly)
     - kelly_fraction:  f64               (0.0-1.0, e.g. 0.5 = half-Kelly)
     - max_allocation:  f64?              (optional cap per venue, 0.0-1.0)
     - drift_threshold: f64               (0.0-1.0, e.g. 0.05 = rebalance if >5% drift)
     - allocations:     VenueAllocation[] (per-venue risk parameters)
     - trigger:         Trigger?          (e.g. daily rebalance check)
   VenueAllocation:
     - target_node:     String? (node ID — use this OR target_nodes)
     - target_nodes:    String[] (group of node IDs that share one Kelly allocation, split equally)
     - expected_return: f64?   (annualized, e.g. 0.15 = 15%. If omitted, derived from venue data)
     - volatility:      f64?   (annualized, e.g. 0.30 = 30%. If omitted, derived from venue data)
     - correlation:     f64    (with reference asset, default 0.0)
   Adaptive mode: when expected_return/volatility are omitted, the optimizer
   computes them from venue alpha_stats (funding rates, lending APY, etc.)

Trigger (optional on any venue node)
=====================================
  Makes a node execute periodically instead of once.
  Triggered nodes can form cycles (e.g. claim -> swap -> optimizer).
  Types:
    - cron:     { "type": "cron", "interval": "hourly|daily|weekly|monthly" }
    - on_event: { "type": "on_event", "event": "<event description>" }

Edge (token flow between nodes)
===============================
  - from_node: String  (source node id)
  - to_node:   String  (destination node id)
  - token:     String  (e.g. "USDC", "USDe", "AERO")
  - amount:    Amount  (fixed { value: "1000" } | percentage { value: 50.0 } | all)
"#;
    println!("{listing}");
    Ok(())
}
