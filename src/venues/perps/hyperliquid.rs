use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;

use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use ferrofluid::types::responses::{ExchangeDataStatus, ExchangeResponseStatus};
use ferrofluid::{ExchangeProvider, InfoProvider, Network};

use crate::model::node::{Node, PerpAction, PerpDirection};
use crate::run::config::RuntimeConfig;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

/// Tracks a single open perp position.
#[derive(Debug, Clone)]
struct PositionState {
    coin: String,
    size: f64,
    entry_price: f64,
    leverage: f64,
}

/// Live executor for Hyperliquid perps and spot via ferrofluid.
pub struct HyperliquidPerp {
    exchange: ExchangeProvider<PrivateKeySigner>,
    info: InfoProvider,
    wallet_address: Address,
    dry_run: bool,
    slippage_bps: f64,
    asset_indices: HashMap<String, u32>,
    sz_decimals: HashMap<String, u32>,
    /// Spot pair indices (e.g. "ETH" → 10000 + pair_index)
    spot_indices: HashMap<String, u32>,
    spot_sz_decimals: HashMap<String, u32>,
    positions: HashMap<String, PositionState>,
    metrics: SimMetrics,
    /// The coin this venue trades (e.g. "ETH"). Set at build time.
    coin: Option<String>,
    /// True if this venue is for a Spot node (not a Perp node).
    is_spot: bool,
    /// Total margin deposited into HL positions (for PnL tracking).
    margin_deposited: f64,
    /// Cached alpha stats from funding history. Updated during tick().
    cached_alpha: Option<(f64, f64)>,
    /// Timestamp of last funding history fetch (avoid hammering API).
    last_alpha_fetch: u64,
    /// Whether we've reconciled positions from HL on-chain state.
    reconciled: bool,
}

impl HyperliquidPerp {
    pub fn new(config: &RuntimeConfig, coin: Option<String>, is_spot: bool) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;

        let (exchange, info) = match config.network {
            Network::Mainnet => (ExchangeProvider::mainnet(signer), InfoProvider::mainnet()),
            Network::Testnet => (ExchangeProvider::testnet(signer), InfoProvider::testnet()),
        };

        Ok(HyperliquidPerp {
            exchange,
            info,
            wallet_address: config.wallet_address,
            dry_run: config.dry_run,
            slippage_bps: config.slippage_bps,
            asset_indices: HashMap::new(),
            sz_decimals: HashMap::new(),
            spot_indices: HashMap::new(),
            spot_sz_decimals: HashMap::new(),
            positions: HashMap::new(),
            metrics: SimMetrics::default(),
            coin,
            is_spot,
            margin_deposited: 0.0,
            cached_alpha: None,
            last_alpha_fetch: 0,
            reconciled: false,
        })
    }

    pub async fn init_metadata(&mut self) -> Result<()> {
        let meta = self
            .info
            .meta()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch meta: {e}"))?;

        for (index, asset) in meta.universe.iter().enumerate() {
            self.asset_indices.insert(asset.name.clone(), index as u32);
            self.sz_decimals
                .insert(asset.name.clone(), asset.sz_decimals);
        }

        println!("  HL: loaded {} asset indices", self.asset_indices.len());

        // Load spot meta for spot order asset indices
        if self.is_spot {
            if let Ok(spot_meta) = self.info.spot_meta().await {
                // Build token index → name map
                let token_names: HashMap<u32, &str> = spot_meta
                    .tokens
                    .iter()
                    .map(|t| (t.index, t.name.as_str()))
                    .collect();

                for pair in &spot_meta.universe {
                    // Resolve base token name from token indices
                    if let Some(base_name) = token_names.get(&pair.tokens[0]) {
                        // Spot asset index = 10000 + pair index
                        let quote = token_names.get(&pair.tokens[1]).copied().unwrap_or("?");
                        if quote == "USDC" {
                            self.spot_indices
                                .insert(base_name.to_string(), 10000 + pair.index);
                        }
                    }
                }
                for token in &spot_meta.tokens {
                    self.spot_sz_decimals
                        .insert(token.name.clone(), token.sz_decimals);
                }
                println!("  HL: loaded {} spot pairs", self.spot_indices.len());
            }
        }
        Ok(())
    }

    /// Reconcile in-memory positions from on-chain HL clearinghouse state.
    /// Called on startup so restarted daemons know about existing positions.
    async fn reconcile_positions(&mut self) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }

        if self.is_spot {
            // Spot: query spot clearinghouse for token balances
            match self.info.user_token_balances(self.wallet_address).await {
                Ok(state) => {
                    for bal in &state.balances {
                        let total: f64 = bal.total.parse().unwrap_or(0.0);
                        if total.abs() > 1e-12 && bal.coin != "USDC" {
                            // Check if this is the coin we trade (handle aliases like ETH↔UETH)
                            if let Some(ref our_coin) = self.coin {
                                let spot_name = self.resolve_spot_coin(our_coin);
                                if bal.coin == *our_coin || bal.coin == spot_name {
                                    let mids = self.get_mids().await.unwrap_or_default();
                                    // Use perp coin name for price lookup (mids keys are perp names)
                                    let price = mids.get(our_coin.as_str())
                                        .or_else(|| mids.get(&bal.coin))
                                        .copied()
                                        .unwrap_or(1.0);
                                    // Store under perp coin name so total_value() can look up mids
                                    self.positions.insert(
                                        our_coin.clone(),
                                        PositionState {
                                            coin: our_coin.clone(),
                                            size: total,
                                            entry_price: price,
                                            leverage: 1.0,
                                        },
                                    );
                                    eprintln!(
                                        "  [reconcile] HL spot: {} {} (${:.2})",
                                        total, bal.coin, total * price,
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  [reconcile] HL spot state unavailable: {e}");
                }
            }
        } else {
            // Perp: query clearinghouse for open positions
            match self.info.user_state(self.wallet_address).await {
                Ok(state) => {
                    let account_value: f64 =
                        state.margin_summary.account_value.parse().unwrap_or(0.0);

                    for ap in &state.asset_positions {
                        let pos = &ap.position;
                        let size: f64 = pos.szi.parse().unwrap_or(0.0);
                        if size.abs() < 1e-12 {
                            continue;
                        }
                        // Check if this is the coin we trade
                        if let Some(ref our_coin) = self.coin {
                            if pos.coin != *our_coin {
                                continue;
                            }
                        }
                        let entry_price: f64 = pos.entry_px.as_deref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0.0);
                        let leverage: f64 = pos.leverage.value as f64;
                        let margin_used: f64 = pos.margin_used.parse().unwrap_or(0.0);

                        self.positions.insert(
                            pos.coin.clone(),
                            PositionState {
                                coin: pos.coin.clone(),
                                size,
                                entry_price,
                                leverage,
                            },
                        );
                        self.margin_deposited += margin_used;
                        eprintln!(
                            "  [reconcile] HL perp: {} {} @ {:.2} (margin=${:.2})",
                            if size > 0.0 { "LONG" } else { "SHORT" },
                            pos.coin,
                            entry_price,
                            margin_used,
                        );
                    }

                    // Track idle USDC on HyperCore (for reporting, not as position value)
                    if self.positions.is_empty() && account_value > 0.5 {
                        eprintln!(
                            "  [reconcile] HL perp: no positions, ${:.2} idle USDC on HyperCore",
                            account_value,
                        );
                    }
                }
                Err(e) => {
                    eprintln!("  [reconcile] HL perp state unavailable: {e}");
                }
            }
        }

        Ok(())
    }

    /// Resolve perp coin name to HL spot token name.
    fn resolve_spot_coin(&self, coin: &str) -> String {
        // Try exact match first
        if self.spot_indices.contains_key(coin) {
            return coin.to_string();
        }
        // Common aliases: ETH→UETH, BTC→UBTC, etc.
        let prefixed = format!("U{coin}");
        if self.spot_indices.contains_key(&prefixed) {
            return prefixed;
        }
        coin.to_string()
    }

    async fn get_mids(&self) -> Result<HashMap<String, f64>> {
        let mids = self
            .info
            .all_mids()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch mids: {e}"))?;

        let mut result = HashMap::new();
        for (coin, price_str) in &mids {
            if let Ok(price) = price_str.parse::<f64>() {
                result.insert(coin.clone(), price);
            }
        }
        Ok(result)
    }

    fn coin_from_pair(pair: &str) -> &str {
        pair.split('/').next().unwrap_or(pair)
    }

    fn format_size(&self, coin: &str, size: f64) -> String {
        let decimals = self.sz_decimals.get(coin).copied().unwrap_or(3);
        format!("{:.prec$}", size, prec = decimals as usize)
    }

    /// Round price to 5 significant figures (Hyperliquid requirement).
    fn format_price(price: f64) -> String {
        if price == 0.0 {
            return "0".to_string();
        }
        let magnitude = price.abs().log10().floor() as i32;
        let decimals = (4 - magnitude).max(0) as usize;
        format!("{:.prec$}", price, prec = decimals)
    }

    async fn execute_perp_open(
        &mut self,
        coin: &str,
        direction: PerpDirection,
        leverage: f64,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let asset = *self
            .asset_indices
            .get(coin)
            .with_context(|| format!("Unknown asset '{coin}' — not in Hyperliquid meta"))?;

        let mids = self.get_mids().await?;
        let mid_price = *mids
            .get(coin)
            .with_context(|| format!("No mid price for '{coin}'"))?;

        let is_buy = matches!(direction, PerpDirection::Long);
        let slippage_mult = self.slippage_bps / 10000.0;
        let limit_price = if is_buy {
            mid_price * (1.0 + slippage_mult)
        } else {
            mid_price * (1.0 - slippage_mult)
        };

        let notional = input_amount * leverage;
        let size = notional / mid_price;

        let formatted_size = self.format_size(coin, size);
        let formatted_price = Self::format_price(limit_price);

        println!(
            "  HL: {} {} {} @ {} (notional ${:.2}, {:.1}x leverage)",
            if is_buy { "BUY" } else { "SELL" },
            formatted_size,
            coin,
            formatted_price,
            notional,
            leverage,
        );

        if self.dry_run {
            println!("  HL: [DRY RUN] order would be placed");
            self.positions.insert(
                coin.to_string(),
                PositionState {
                    coin: coin.to_string(),
                    size: if is_buy { size } else { -size },
                    entry_price: mid_price,
                    leverage,
                },
            );
            self.margin_deposited += input_amount;
            return Ok(ExecutionResult::PositionUpdate {
                consumed: input_amount,
                output: None,
            });
        }

        let order = ferrofluid::types::OrderRequest::limit(
            asset,
            is_buy,
            &formatted_price,
            &formatted_size,
            "Ioc",
        );

        let response = self
            .exchange
            .place_order(&order)
            .await
            .map_err(|e| anyhow::anyhow!("Order failed: {e}"))?;

        match response {
            ExchangeResponseStatus::Ok(resp) => {
                if let Some(data) = &resp.data {
                    for status in &data.statuses {
                        match status {
                            ExchangeDataStatus::Filled(fill) => {
                                let fill_size: f64 = fill.total_sz.parse().unwrap_or(0.0);
                                let fill_price: f64 = fill.avg_px.parse().unwrap_or(mid_price);
                                println!(
                                    "  HL: FILLED {} {} @ {} (oid: {})",
                                    fill_size, coin, fill_price, fill.oid
                                );
                                self.positions.insert(
                                    coin.to_string(),
                                    PositionState {
                                        coin: coin.to_string(),
                                        size: if is_buy { fill_size } else { -fill_size },
                                        entry_price: fill_price,
                                        leverage,
                                    },
                                );
                            }
                            ExchangeDataStatus::Resting(rest) => {
                                println!(
                                    "  HL: RESTING oid: {} (IOC should not rest — partial fill?)",
                                    rest.oid
                                );
                            }
                            ExchangeDataStatus::Error(msg) => {
                                bail!("HL order error: {msg}");
                            }
                            _ => {}
                        }
                    }
                }
                self.margin_deposited += input_amount;
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            ExchangeResponseStatus::Err(err) => {
                bail!("HL exchange error: {err}");
            }
        }
    }

    async fn execute_perp_close(&mut self, coin: &str, margin: &str) -> Result<ExecutionResult> {
        let asset = *self
            .asset_indices
            .get(coin)
            .with_context(|| format!("Unknown asset '{coin}'"))?;

        let position = self
            .positions
            .get(coin)
            .cloned()
            .with_context(|| format!("No position to close for '{coin}'"))?;

        let mids = self.get_mids().await?;
        let mid_price = *mids
            .get(coin)
            .with_context(|| format!("No mid price for '{coin}'"))?;

        let is_buy = position.size < 0.0;
        let slippage_mult = self.slippage_bps / 10000.0;
        let limit_price = if is_buy {
            mid_price * (1.0 + slippage_mult)
        } else {
            mid_price * (1.0 - slippage_mult)
        };

        let close_size = position.size.abs();
        let formatted_size = self.format_size(coin, close_size);
        let formatted_price = Self::format_price(limit_price);

        println!(
            "  HL: CLOSE {} {} @ {} (reduce_only)",
            formatted_size, coin, formatted_price,
        );

        if self.dry_run {
            println!("  HL: [DRY RUN] close order would be placed");
            let pnl = (mid_price - position.entry_price) * position.size;
            let output_value = (position.entry_price * close_size / position.leverage) + pnl;
            self.positions.remove(coin);
            return Ok(ExecutionResult::PositionUpdate {
                consumed: 0.0,
                output: Some((margin.to_string(), output_value.max(0.0))),
            });
        }

        let order = ferrofluid::types::OrderRequest::limit(
            asset,
            is_buy,
            &formatted_price,
            &formatted_size,
            "Ioc",
        )
        .reduce_only(true);

        let response = self
            .exchange
            .place_order(&order)
            .await
            .map_err(|e| anyhow::anyhow!("Close order failed: {e}"))?;

        match response {
            ExchangeResponseStatus::Ok(resp) => {
                let mut realized_pnl = 0.0;
                if let Some(data) = &resp.data {
                    for status in &data.statuses {
                        match status {
                            ExchangeDataStatus::Filled(fill) => {
                                let fill_price: f64 = fill.avg_px.parse().unwrap_or(mid_price);
                                realized_pnl = (fill_price - position.entry_price) * position.size;
                                println!(
                                    "  HL: CLOSED {} @ {} (PnL: ${:.2})",
                                    coin, fill_price, realized_pnl
                                );
                            }
                            ExchangeDataStatus::Error(msg) => {
                                bail!("HL close error: {msg}");
                            }
                            _ => {}
                        }
                    }
                }
                let margin_returned = position.entry_price * close_size / position.leverage;
                let output_value = (margin_returned + realized_pnl).max(0.0);
                self.positions.remove(coin);
                Ok(ExecutionResult::PositionUpdate {
                    consumed: 0.0,
                    output: Some((margin.to_string(), output_value)),
                })
            }
            ExchangeResponseStatus::Err(err) => {
                bail!("HL exchange error on close: {err}");
            }
        }
    }

    /// Fetch 7 days of funding history and compute annualized return + volatility.
    async fn fetch_funding_alpha(&self, coin: &str) -> Option<(f64, f64)> {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let week_ago_ms = now_ms.saturating_sub(7 * 24 * 3600 * 1000);

        let history = match self
            .info
            .funding_history(coin.to_string())
            .start_time(week_ago_ms)
            .send()
            .await
        {
            Ok(h) if h.len() >= 10 => h,
            Ok(h) => {
                eprintln!(
                    "  HL: funding history for {} has only {} entries, need >=10",
                    coin,
                    h.len()
                );
                return None;
            }
            Err(e) => {
                eprintln!("  HL: failed to fetch funding history for {}: {}", coin, e);
                return None;
            }
        };

        // Each entry has funding_rate (per-hour) and time (ms).
        // Compute per-period returns and annualize.
        let mut returns = Vec::with_capacity(history.len() - 1);
        for i in 1..history.len() {
            let dt_hours =
                (history[i].time.saturating_sub(history[i - 1].time)) as f64 / 3_600_000.0;
            if dt_hours <= 0.0 {
                continue;
            }
            let rate: f64 = history[i].funding_rate.parse().unwrap_or(0.0);
            // funding_rate is per-hour; for shorts: positive rate = income
            returns.push(rate * dt_hours);
        }

        if returns.len() < 10 {
            return None;
        }

        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let std = var.sqrt();

        let total_ms =
            (history.last().unwrap().time.saturating_sub(history[0].time)) as f64;
        let avg_period_hours = total_ms / (history.len() - 1) as f64 / 3_600_000.0;
        let periods_per_year = 8760.0 / avg_period_hours;

        let annualized_return = mean * periods_per_year;
        let annualized_vol = std * periods_per_year.sqrt();

        eprintln!(
            "  HL: {} funding alpha: {:.2}% return, {:.2}% vol (from {} samples)",
            coin,
            annualized_return * 100.0,
            annualized_vol * 100.0,
            history.len(),
        );

        Some((annualized_return, annualized_vol))
    }

    async fn execute_collect_funding(&self, coin: &str, _margin: &str) -> Result<ExecutionResult> {
        println!(
            "  HL: funding for {} is auto-credited to margin (no action needed)",
            coin
        );
        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for HyperliquidPerp {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        if self.asset_indices.is_empty() {
            self.init_metadata().await?;
        }

        let margin = node.margin_token().unwrap_or("USDC");

        match node {
            Node::Perp {
                pair,
                action,
                direction,
                leverage,
                ..
            } => {
                let coin = Self::coin_from_pair(pair);
                match action {
                    PerpAction::Open => {
                        let dir = direction.with_context(|| "Perp open requires direction")?;
                        let lev = leverage.unwrap_or(1.0);
                        self.execute_perp_open(coin, dir, lev, input_amount).await
                    }
                    PerpAction::Close => self.execute_perp_close(coin, margin).await,
                    PerpAction::Adjust => {
                        println!("  HL: ADJUST leverage for {} (not yet implemented)", coin);
                        Ok(ExecutionResult::Noop)
                    }
                    PerpAction::CollectFunding => self.execute_collect_funding(coin, margin).await,
                }
            }
            Node::Spot { pair, side, .. } => {
                let coin = Self::coin_from_pair(pair);
                let is_buy = matches!(side, crate::model::node::SpotSide::Buy);

                if self.asset_indices.is_empty() {
                    self.init_metadata().await?;
                }

                let mids = self.get_mids().await?;
                let mid_price = mids.get(coin).copied().unwrap_or(0.0);

                println!(
                    "  HL SPOT: {} {} with ${:.2} (mid: {:.4})",
                    if is_buy { "BUY" } else { "SELL" },
                    coin,
                    input_amount,
                    mid_price,
                );

                if mid_price <= 0.0 {
                    bail!("No mid price for spot asset '{coin}'");
                }

                if self.dry_run {
                    println!("  HL SPOT: [DRY RUN]");
                    let output_amount = if is_buy {
                        input_amount / mid_price
                    } else {
                        input_amount * mid_price
                    };
                    return Ok(ExecutionResult::TokenOutput {
                        token: if is_buy {
                            coin.to_string()
                        } else {
                            "USDC".to_string()
                        },
                        amount: output_amount,
                    });
                }

                // HL spot uses different token names (e.g. UETH instead of ETH)
                let spot_coin = self.resolve_spot_coin(coin);
                let spot_asset = *self
                    .spot_indices
                    .get(&spot_coin)
                    .with_context(|| format!("Unknown spot asset '{coin}' (tried '{spot_coin}') — not in HL spot meta"))?;

                let spot_decimals = self.spot_sz_decimals.get(&spot_coin).copied().unwrap_or(4);
                let slippage_mult = self.slippage_bps / 10000.0;
                let limit_price = if is_buy {
                    mid_price * (1.0 + slippage_mult)
                } else {
                    mid_price * (1.0 - slippage_mult)
                };

                // For spot buy: size = USDC amount / price (in base token units)
                // For spot sell: size = input_amount (already in base token units)
                let size = if is_buy {
                    input_amount / mid_price
                } else {
                    input_amount
                };

                let formatted_size = format!("{:.prec$}", size, prec = spot_decimals as usize);
                let formatted_price = Self::format_price(limit_price);

                println!(
                    "  HL SPOT: {} {} {} @ {} (${:.2})",
                    if is_buy { "BUY" } else { "SELL" },
                    formatted_size, coin, formatted_price, input_amount,
                );

                let order = ferrofluid::types::OrderRequest::limit(
                    spot_asset,
                    is_buy,
                    &formatted_price,
                    &formatted_size,
                    "Ioc",
                );

                let response = self
                    .exchange
                    .place_order(&order)
                    .await
                    .map_err(|e| anyhow::anyhow!("Spot order failed: {e}"))?;

                match response {
                    ExchangeResponseStatus::Ok(resp) => {
                        if let Some(data) = &resp.data {
                            for status in &data.statuses {
                                match status {
                                    ExchangeDataStatus::Filled(fill) => {
                                        let fill_size: f64 = fill.total_sz.parse().unwrap_or(0.0);
                                        let fill_price: f64 = fill.avg_px.parse().unwrap_or(mid_price);
                                        println!(
                                            "  HL SPOT: FILLED {} {} @ {} (oid: {})",
                                            fill_size, coin, fill_price, fill.oid
                                        );
                                        self.positions.insert(
                                            coin.to_string(),
                                            PositionState {
                                                coin: coin.to_string(),
                                                size: fill_size,
                                                entry_price: fill_price,
                                                leverage: 1.0,
                                            },
                                        );
                                    }
                                    ExchangeDataStatus::Error(msg) => {
                                        bail!("HL spot order error: {msg}");
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(ExecutionResult::PositionUpdate {
                            consumed: input_amount,
                            output: None,
                        })
                    }
                    ExchangeResponseStatus::Err(err) => {
                        bail!("HL spot exchange error: {err}");
                    }
                }
            }
            _ => {
                println!(
                    "  HL: unsupported node type '{}' for HyperliquidPerp",
                    node.type_name()
                );
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        if self.dry_run {
            let mids = self.get_mids().await.unwrap_or_default();
            let mut total = 0.0;
            for pos in self.positions.values() {
                let current_price = mids.get(&pos.coin).copied().unwrap_or(pos.entry_price);
                let pnl = (current_price - pos.entry_price) * pos.size;
                let margin = pos.entry_price * pos.size.abs() / pos.leverage;
                total += margin + pnl;
            }
            return Ok(total.max(0.0));
        }

        // Only report value for actual positions we hold — idle USDC on HyperCore
        // should NOT count as venue value (the optimizer would think it's deployed).
        // Spot and perp venues sharing the same HL wallet would double-count otherwise.
        if self.positions.is_empty() {
            return Ok(0.0);
        }


        if self.is_spot {
            // Spot: value = quantity * mid price
            let mids = self.get_mids().await.unwrap_or_default();
            let mut total = 0.0;
            for pos in self.positions.values() {
                let price = mids.get(&pos.coin).copied().unwrap_or(pos.entry_price);
                total += pos.size.abs() * price;
            }
            return Ok(total);
        }

        // Perp: use HL's margin_used + unrealizedPnl per position
        let state = self
            .info
            .user_state(self.wallet_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch user state: {e}"))?;

        let mut total = 0.0;
        for ap in &state.asset_positions {
            let pos = &ap.position;
            let size: f64 = pos.szi.parse().unwrap_or(0.0);
            if size.abs() < 1e-12 {
                continue;
            }
            // Only count positions we're tracking
            if !self.positions.contains_key(&pos.coin) {
                continue;
            }
            let margin_used: f64 = pos.margin_used.parse().unwrap_or(0.0);
            let upnl: f64 = pos.unrealized_pnl.parse().unwrap_or(0.0);
            total += margin_used + upnl;
        }

        Ok(total.max(0.0))
    }

    async fn tick(&mut self, now: u64, _dt_secs: f64) -> Result<()> {
        // Reconcile positions from HL on first tick (populates self.positions
        // so total_value() and unwind() work correctly after restart).
        if !self.reconciled {
            self.reconciled = true;
            if let Err(e) = self.reconcile_positions().await {
                eprintln!("  [reconcile] HL reconcile failed: {e}");
            }
        }

        // Track total PnL (funding + unrealized) for perp positions
        if !self.is_spot && !self.dry_run && self.margin_deposited > 0.0 {
            if let Ok(account_value) = self.total_value().await {
                if account_value > 0.0 {
                    self.metrics.funding_pnl = account_value - self.margin_deposited;
                }
            }
        }

        if !self.positions.is_empty() {
            let mids = self.get_mids().await.unwrap_or_default();
            for pos in self.positions.values() {
                let current_price = mids.get(&pos.coin).copied().unwrap_or(pos.entry_price);
                let pnl = (current_price - pos.entry_price) * pos.size;
                let direction = if pos.size > 0.0 { "LONG" } else { "SHORT" };
                println!(
                    "  HL TICK: {} {} {:.4} @ {:.2} (entry: {:.2}, PnL: ${:.2})",
                    direction,
                    pos.coin,
                    pos.size.abs(),
                    current_price,
                    pos.entry_price,
                    pnl,
                );
            }
        }

        // Fetch funding history for alpha stats (perp only, every hour)
        if !self.is_spot && now.saturating_sub(self.last_alpha_fetch) >= 3600 {
            if let Some(ref coin) = self.coin {
                self.cached_alpha = self.fetch_funding_alpha(coin).await;
                self.last_alpha_fetch = now;
            }
        }
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }

        // No positions to close → nothing to unwind (don't return phantom freed)
        if self.positions.is_empty() {
            return Ok(0.0);
        }

        let f = fraction.min(1.0);
        let freed = total * f;

        if self.asset_indices.is_empty() {
            self.init_metadata().await?;
        }

        // Close fraction of each position
        let coins: Vec<String> = self.positions.keys().cloned().collect();
        for coin in coins {
            let pos = match self.positions.get(&coin) {
                Some(p) => p.clone(),
                None => continue,
            };
            let close_size = pos.size.abs() * f;
            if close_size < 1e-12 {
                continue;
            }

            let mids = self.get_mids().await?;
            let mid_price = mids.get(&coin).copied().unwrap_or(pos.entry_price);
            let is_buy = pos.size < 0.0; // close short = buy, close long = sell

            let slippage_mult = self.slippage_bps / 10000.0;
            let limit_price = if is_buy {
                mid_price * (1.0 + slippage_mult)
            } else {
                mid_price * (1.0 - slippage_mult)
            };

            let formatted_size = self.format_size(&coin, close_size);
            let formatted_price = Self::format_price(limit_price);

            println!(
                "  HL: UNWIND {:.1}% {} {} @ {} (reduce_only)",
                f * 100.0,
                formatted_size,
                coin,
                formatted_price,
            );

            if self.dry_run {
                println!("  HL: [DRY RUN] unwind order would be placed");
                let remaining_size = pos.size.abs() - close_size;
                if remaining_size < 1e-12 {
                    self.positions.remove(&coin);
                } else {
                    let entry = self.positions.get_mut(&coin).unwrap();
                    entry.size = remaining_size * pos.size.signum();
                }
                continue;
            }

            let asset = *self
                .asset_indices
                .get(&coin)
                .with_context(|| format!("Unknown asset '{coin}'"))?;

            let order = ferrofluid::types::OrderRequest::limit(
                asset,
                is_buy,
                &formatted_price,
                &formatted_size,
                "Ioc",
            )
            .reduce_only(true);

            match self.exchange.place_order(&order).await {
                Ok(ExchangeResponseStatus::Ok(resp)) => {
                    if let Some(data) = &resp.data {
                        for status in &data.statuses {
                            match status {
                                ExchangeDataStatus::Filled(fill) => {
                                    let fill_size: f64 = fill.total_sz.parse().unwrap_or(0.0);
                                    println!(
                                        "  HL: UNWIND FILLED {} {} (oid: {})",
                                        fill_size, coin, fill.oid
                                    );
                                }
                                ExchangeDataStatus::Error(msg) => {
                                    eprintln!("  HL: UNWIND error for {}: {}", coin, msg);
                                }
                                _ => {}
                            }
                        }
                    }
                    let remaining_size = pos.size.abs() - close_size;
                    if remaining_size < 1e-12 {
                        self.positions.remove(&coin);
                    } else {
                        if let Some(entry) = self.positions.get_mut(&coin) {
                            entry.size = remaining_size * pos.size.signum();
                        }
                    }
                }
                Ok(ExchangeResponseStatus::Err(err)) => {
                    eprintln!("  HL: UNWIND exchange error for {}: {}", coin, err);
                }
                Err(e) => {
                    eprintln!("  HL: UNWIND failed for {}: {:#}", coin, e);
                }
            }
        }

        Ok(freed)
    }

    fn metrics(&self) -> SimMetrics {
        self.metrics.clone()
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        if self.is_spot {
            // Spot has no inherent yield — directional only.
            // In a DN group, this contributes (0, 0) so the group stats
            // are driven entirely by the perp's funding alpha.
            return Some((0.0, 0.0));
        }
        self.cached_alpha
    }
}
