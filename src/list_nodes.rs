/// Print a human-readable listing of all available node types and their parameters.
pub fn run() -> anyhow::Result<()> {
    let listing = r#"Available Node Types
====================

1. wallet
   Source or sink for funds on a specific chain.
   Parameters:
     - chain:   Chain   (object: {"name":"ethereum","chain_id":1,"rpc_url":"https://eth.llamarpc.com"} — chain_id and rpc_url optional for non-EVM chains like {"name":"hyperliquid"})
     - address: String  (0x-prefixed wallet address)

2. perp
   Perpetual futures — full lifecycle: open, close, adjust, collect funding.
   Parameters:
     - venue:     PerpVenue     (Hyperliquid | Hyena)
     - pair:      String        (e.g. "ETH/USDC")
     - action:    PerpAction    (open | close | adjust | collect_funding)
     - direction: PerpDirection (long | short)       — required for open/adjust
     - leverage:  f64           (e.g. 5.0)           — required for open/adjust
     - trigger:   Trigger?      (optional, for periodic actions)

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
     - venue:   SpotVenue (Aerodrome)
     - pair:    String    (e.g. "ETH/USDC")
     - side:    SpotSide  (buy | sell)
     - trigger: Trigger?  (optional)

5. lp
   Liquidity provision — Aerodrome Slipstream concentrated liquidity.
   Uses Uniswap V3-style NFT positions with tick ranges. Tighter ranges
   earn more fees (concentration multiplier) but risk going out of range.
   Parameters:
     - venue:        LpVenue  (Aerodrome)
     - pool:         String   (e.g. "cbBTC/WETH")
     - action:       LpAction (add_liquidity | remove_liquidity | claim_rewards | compound | stake_gauge | unstake_gauge)
     - tick_lower:   i32?     (lower tick bound, e.g. -500. Omit for full-range)
     - tick_upper:   i32?     (upper tick bound, e.g.  500. Omit for full-range)
     - tick_spacing:  i32?    (pool tick spacing, e.g. 100 for CL100, 200 for CL200)
     - trigger:      Trigger? (optional, e.g. claim_rewards daily)

6. swap
   Token swap via aggregator.
   Parameters:
     - provider:   SwapProvider (LiFi)
     - from_token: String       (e.g. "USDe")
     - to_token:   String       (e.g. "USDC")
     - chain:      Chain?       (optional — chain this swap executes on, for cross-chain validation)
     - trigger:    Trigger?     (optional)

7. bridge
   Cross-chain bridge transfer.
   Parameters:
     - provider:   BridgeProvider (LiFi | Stargate)
     - from_chain: Chain          (object: {"name":"mantle","chain_id":5000,"rpc_url":"https://rpc.mantle.xyz"})
     - to_chain:   Chain          (same format — non-EVM chains omit chain_id/rpc_url: {"name":"hyperliquid"})
     - token:      String         (e.g. "USDe")
     - trigger:    Trigger?       (optional)

8. lending
   Lending protocol — supply, borrow, repay, withdraw, claim rewards.
   Execution layer handles protocol-specific details (Morpho market IDs,
   HyperLend E-mode, Init Capital vault shares, etc.)
   Parameters:
     - venue:   LendingVenue (Aave | Lendle | Morpho | Compound | InitCapital | HyperLend)
     - asset:   String       (e.g. "USDC", "WETH", "USDe")
     - action:  LendingAction (supply | withdraw | borrow | repay | claim_rewards)
     - trigger: Trigger?     (optional, e.g. claim_rewards daily)

9. pendle
   Pendle yield tokenization — mint/redeem principal tokens (PT) for fixed yield
   or yield tokens (YT) for variable yield. Used in strategies like PT-kHYPE looping.
   Execution layer handles Pendle router interactions and market lookups.
   Parameters:
     - market:  String       (e.g. "PT-kHYPE", "PT-stETH", "PT-eETH")
     - action:  PendleAction (mint_pt | redeem_pt | mint_yt | redeem_yt | claim_rewards)
     - trigger: Trigger?     (optional, e.g. claim_rewards weekly)

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
     - target_node:    String  (node ID of downstream venue)
     - expected_return: f64    (annualized, e.g. 0.15 = 15%)
     - volatility:      f64    (annualized, e.g. 0.30 = 30%)
     - correlation:     f64    (with reference asset, default 0.0)

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
