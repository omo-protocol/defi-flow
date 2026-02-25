use std::collections::HashMap;

use anyhow::{bail, Context, Result};
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
    positions: HashMap<String, PositionState>,
    metrics: SimMetrics,
}

impl HyperliquidPerp {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .private_key
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid private key: {e}"))?;

        let (exchange, info) = match config.network {
            Network::Mainnet => (
                ExchangeProvider::mainnet(signer),
                InfoProvider::mainnet(),
            ),
            Network::Testnet => (
                ExchangeProvider::testnet(signer),
                InfoProvider::testnet(),
            ),
        };

        Ok(HyperliquidPerp {
            exchange,
            info,
            wallet_address: config.wallet_address,
            dry_run: config.dry_run,
            slippage_bps: config.slippage_bps,
            asset_indices: HashMap::new(),
            sz_decimals: HashMap::new(),
            positions: HashMap::new(),
            metrics: SimMetrics::default(),
        })
    }

    pub async fn init_metadata(&mut self) -> Result<()> {
        let meta = self
            .info
            .meta()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch meta: {e}"))?;

        for (index, asset) in meta.universe.iter().enumerate() {
            self.asset_indices
                .insert(asset.name.clone(), index as u32);
            self.sz_decimals
                .insert(asset.name.clone(), asset.sz_decimals);
        }

        println!(
            "  HL: loaded {} asset indices",
            self.asset_indices.len()
        );
        Ok(())
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

    fn format_price(price: f64) -> String {
        if price >= 10000.0 {
            format!("{:.1}", price)
        } else if price >= 1000.0 {
            format!("{:.2}", price)
        } else if price >= 100.0 {
            format!("{:.3}", price)
        } else if price >= 10.0 {
            format!("{:.4}", price)
        } else if price >= 1.0 {
            format!("{:.5}", price)
        } else {
            format!("{:.6}", price)
        }
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
                                let fill_size: f64 =
                                    fill.total_sz.parse().unwrap_or(0.0);
                                let fill_price: f64 =
                                    fill.avg_px.parse().unwrap_or(mid_price);
                                println!(
                                    "  HL: FILLED {} {} @ {} (oid: {})",
                                    fill_size, coin, fill_price, fill.oid
                                );
                                self.positions.insert(
                                    coin.to_string(),
                                    PositionState {
                                        coin: coin.to_string(),
                                        size: if is_buy {
                                            fill_size
                                        } else {
                                            -fill_size
                                        },
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
                                let fill_price: f64 =
                                    fill.avg_px.parse().unwrap_or(mid_price);
                                realized_pnl =
                                    (fill_price - position.entry_price) * position.size;
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
                let margin_returned =
                    position.entry_price * close_size / position.leverage;
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

    async fn execute_collect_funding(
        &self,
        coin: &str,
        _margin: &str,
    ) -> Result<ExecutionResult> {
        println!(
            "  HL: funding for {} is auto-credited to margin (no action needed)",
            coin
        );
        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for HyperliquidPerp {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
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
                        let dir = direction.with_context(|| {
                            "Perp open requires direction"
                        })?;
                        let lev = leverage.unwrap_or(1.0);
                        self.execute_perp_open(coin, dir, lev, input_amount)
                            .await
                    }
                    PerpAction::Close => self.execute_perp_close(coin, margin).await,
                    PerpAction::Adjust => {
                        println!("  HL: ADJUST leverage for {} (not yet implemented)", coin);
                        Ok(ExecutionResult::Noop)
                    }
                    PerpAction::CollectFunding => {
                        self.execute_collect_funding(coin, margin).await
                    }
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

                println!("  HL SPOT: full spot execution TBD — treating as price lookup");
                Ok(ExecutionResult::Noop)
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

        let state = self
            .info
            .user_state(self.wallet_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch user state: {e}"))?;

        let account_value: f64 = state
            .margin_summary
            .account_value
            .parse()
            .unwrap_or(0.0);

        Ok(account_value)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
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
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
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
                f * 100.0, formatted_size, coin, formatted_price,
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
                                    let fill_size: f64 =
                                        fill.total_sz.parse().unwrap_or(0.0);
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
}
