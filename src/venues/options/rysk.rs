use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::model::node::{Node, OptionsAction, RyskAsset};
use crate::run::config::RuntimeConfig;
use crate::venues::{ExecutionResult, SimMetrics, Venue};

const RYSK_API_URL: &str = "https://v12.rysk.finance/api/inventory";

// ── Rysk API types ─────────────────────────────────────────────────

type InventoryResponse = HashMap<String, AssetInventory>;

#[derive(Debug, Deserialize)]
struct AssetInventory {
    combinations: HashMap<String, OptionEntry>,
}

#[derive(Debug, Deserialize)]
struct OptionEntry {
    strike: Option<f64>,
    #[serde(rename = "expiration_timestamp")]
    expiration_timestamp: Option<u64>,
    #[serde(rename = "isPut")]
    is_put: Option<bool>,
    #[serde(rename = "bidIv")]
    bid_iv: Option<f64>,
    #[serde(rename = "askIv")]
    ask_iv: Option<f64>,
    index: Option<f64>,
}

// ── Tracked option position ────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct OptionPosition {
    key: String,
    asset: String,
    is_put: bool,
    strike: f64,
    expiry: u64,
    premium: f64,
    collateral: f64,
    size: f64,
    is_short: bool,
}

// ── Rysk Options ──────────────────────────────────────────────────

pub struct RyskOptions {
    client: reqwest::Client,
    dry_run: bool,
    positions: Vec<OptionPosition>,
    total_premium_collected: f64,
    total_collateral_locked: f64,
    metrics: SimMetrics,
}

impl RyskOptions {
    pub fn new(config: &RuntimeConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("defi-flow/0.1")
            .build()
            .context("creating Rysk HTTP client")?;

        Ok(RyskOptions {
            client,
            dry_run: config.dry_run,
            positions: Vec::new(),
            total_premium_collected: 0.0,
            total_collateral_locked: 0.0,
            metrics: SimMetrics::default(),
        })
    }

    async fn fetch_inventory(&self) -> Result<InventoryResponse> {
        let resp = self
            .client
            .get(RYSK_API_URL)
            .send()
            .await
            .context("fetching Rysk inventory")?
            .error_for_status()
            .context("Rysk API error")?
            .json::<InventoryResponse>()
            .await
            .context("parsing Rysk inventory")?;
        Ok(resp)
    }

    fn asset_key(asset: &RyskAsset) -> &str {
        match asset {
            RyskAsset::ETH => "ETH",
            RyskAsset::BTC => "BTC",
            RyskAsset::HYPE => "HYPE",
            RyskAsset::SOL => "SOL",
        }
    }

    fn select_option(
        entries: &HashMap<String, OptionEntry>,
        is_put: bool,
        delta_target: Option<f64>,
        days_to_expiry: Option<u32>,
        min_apy: Option<f64>,
    ) -> Option<(String, &OptionEntry)> {
        let now = chrono::Utc::now().timestamp() as u64;
        let target_dte = days_to_expiry.unwrap_or(30) as f64;
        let _target_delta = delta_target.unwrap_or(0.3);

        let mut best: Option<(String, &OptionEntry, f64)> = None;

        for (key, entry) in entries {
            let entry_is_put = entry.is_put.unwrap_or(false);
            if entry_is_put != is_put {
                continue;
            }

            let strike = entry.strike.unwrap_or(0.0);
            let expiry = entry.expiration_timestamp.unwrap_or(0);
            let spot = entry.index.unwrap_or(0.0);

            if strike <= 0.0 || spot <= 0.0 || expiry <= now {
                continue;
            }

            let dte = (expiry - now) as f64 / 86400.0;
            if dte < target_dte * 0.5 || dte > target_dte * 1.5 {
                continue;
            }

            let mid_iv =
                ((entry.bid_iv.unwrap_or(50.0) + entry.ask_iv.unwrap_or(50.0)) / 2.0) / 100.0;
            let time_factor = (dte / 365.0).sqrt();
            let premium = spot * mid_iv * time_factor * 0.4;

            let collateral = if is_put { strike } else { spot };
            let apy = if collateral > 0.0 && dte > 0.0 {
                (premium / collateral) * (365.0 / dte)
            } else {
                0.0
            };

            if let Some(min) = min_apy {
                if apy < min {
                    continue;
                }
            }

            let is_better = best
                .as_ref()
                .map_or(true, |(_, _, best_apy)| apy > *best_apy);
            if is_better {
                best = Some((key.clone(), entry, apy));
            }
        }

        best.map(|(key, entry, _)| (key, entry))
    }

    async fn execute_sell(
        &mut self,
        asset: &RyskAsset,
        is_put: bool,
        delta_target: Option<f64>,
        days_to_expiry: Option<u32>,
        min_apy: Option<f64>,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let asset_key = Self::asset_key(asset);
        let option_type = if is_put { "PUT" } else { "CALL" };

        println!(
            "  RYSK SELL_{}: {} with ${:.2} collateral",
            option_type, asset_key, input_amount,
        );

        let inventory = self.fetch_inventory().await?;
        let asset_inv = inventory
            .iter()
            .find(|(k, _)| k.to_uppercase().contains(asset_key));

        let entries = match asset_inv {
            Some((_, inv)) => &inv.combinations,
            None => {
                println!("  RYSK: no inventory found for {asset_key}");
                return Ok(ExecutionResult::Noop);
            }
        };

        let selected = Self::select_option(entries, is_put, delta_target, days_to_expiry, min_apy);

        match selected {
            Some((key, entry)) => {
                let strike = entry.strike.unwrap_or(0.0);
                let expiry = entry.expiration_timestamp.unwrap_or(0);
                let spot = entry.index.unwrap_or(0.0);
                let mid_iv =
                    ((entry.bid_iv.unwrap_or(50.0) + entry.ask_iv.unwrap_or(50.0)) / 2.0) / 100.0;
                let dte = (expiry as f64 - chrono::Utc::now().timestamp() as f64) / 86400.0;
                let time_factor = (dte / 365.0).sqrt();
                let premium_per_unit = spot * mid_iv * time_factor * 0.4;
                let collateral_per_unit = if is_put { strike } else { spot };
                let size = input_amount / collateral_per_unit;
                let total_premium = premium_per_unit * size;
                let apy = if collateral_per_unit > 0.0 && dte > 0.0 {
                    (premium_per_unit / collateral_per_unit) * (365.0 / dte)
                } else {
                    0.0
                };

                println!(
                    "  RYSK SELECTED: {} strike={:.0} expiry={} DTE={:.0} IV={:.1}% APY={:.1}%",
                    key,
                    strike,
                    expiry,
                    dte,
                    mid_iv * 100.0,
                    apy * 100.0,
                );
                println!(
                    "  RYSK: size={:.4} premium=${:.2} collateral=${:.2}",
                    size, total_premium, input_amount,
                );

                if self.dry_run {
                    println!("  RYSK: [DRY RUN] would submit RFQ order");
                    self.positions.push(OptionPosition {
                        key: key.clone(),
                        asset: asset_key.to_string(),
                        is_put,
                        strike,
                        expiry,
                        premium: total_premium,
                        collateral: input_amount,
                        size,
                        is_short: true,
                    });
                    self.total_premium_collected += total_premium;
                    self.total_collateral_locked += input_amount;
                    self.metrics.premium_pnl += total_premium;

                    return Ok(ExecutionResult::PositionUpdate {
                        consumed: input_amount,
                        output: Some(("USDC".to_string(), total_premium)),
                    });
                }

                println!("  RYSK: live RFQ submission not yet implemented");
                self.positions.push(OptionPosition {
                    key,
                    asset: asset_key.to_string(),
                    is_put,
                    strike,
                    expiry,
                    premium: total_premium,
                    collateral: input_amount,
                    size,
                    is_short: true,
                });
                self.total_premium_collected += total_premium;
                self.total_collateral_locked += input_amount;

                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: Some(("USDC".to_string(), total_premium)),
                })
            }
            None => {
                println!(
                    "  RYSK: no suitable {} found matching criteria (delta={:?}, DTE={:?}, min_apy={:?})",
                    option_type, delta_target, days_to_expiry, min_apy,
                );
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn execute_collect_premium(&mut self, asset: &RyskAsset) -> Result<ExecutionResult> {
        let asset_key = Self::asset_key(asset);
        let now = chrono::Utc::now().timestamp() as u64;

        println!("  RYSK COLLECT_PREMIUM: {}", asset_key);

        let mut settled_pnl = 0.0;
        let mut settled_collateral = 0.0;

        self.positions.retain(|pos| {
            if pos.asset == asset_key && pos.expiry <= now && pos.is_short {
                settled_pnl += pos.premium;
                settled_collateral += pos.collateral;
                println!(
                    "  RYSK: settled expired {} strike={:.0} premium=${:.2}",
                    pos.key, pos.strike, pos.premium,
                );
                false
            } else {
                true
            }
        });

        if settled_collateral > 0.0 {
            self.total_collateral_locked -= settled_collateral;
            if self.dry_run {
                println!("  RYSK: [DRY RUN] would claim settled premium");
            }
            return Ok(ExecutionResult::TokenOutput {
                token: "USDC".to_string(),
                amount: settled_collateral + settled_pnl,
            });
        }

        println!("  RYSK: no expired positions to settle");
        Ok(ExecutionResult::Noop)
    }

    async fn execute_roll(
        &mut self,
        asset: &RyskAsset,
        delta_target: Option<f64>,
        days_to_expiry: Option<u32>,
        min_apy: Option<f64>,
        roll_days_before: Option<u32>,
    ) -> Result<ExecutionResult> {
        let asset_key = Self::asset_key(asset);
        let now = chrono::Utc::now().timestamp() as u64;
        let roll_threshold = roll_days_before.unwrap_or(3) as u64 * 86400;

        println!(
            "  RYSK ROLL: {} (roll {}d before expiry)",
            asset_key,
            roll_days_before.unwrap_or(3)
        );

        let near_expiry: Vec<OptionPosition> = self
            .positions
            .iter()
            .filter(|p| {
                p.asset == asset_key
                    && p.is_short
                    && p.expiry > 0
                    && p.expiry - now <= roll_threshold
            })
            .cloned()
            .collect();

        if near_expiry.is_empty() {
            println!("  RYSK: no positions near expiry to roll");
            return Ok(ExecutionResult::Noop);
        }

        for pos in &near_expiry {
            println!(
                "  RYSK: rolling {} strike={:.0} (expires in {:.1}d)",
                pos.key,
                pos.strike,
                (pos.expiry - now) as f64 / 86400.0,
            );

            if self.dry_run {
                println!("  RYSK: [DRY RUN] would close + reopen at next expiry");
            }

            self.positions.retain(|p| p.key != pos.key || !p.is_short);
            self.total_collateral_locked -= pos.collateral;

            self.execute_sell(
                asset,
                pos.is_put,
                delta_target,
                days_to_expiry,
                min_apy,
                pos.collateral,
            )
            .await?;
        }

        Ok(ExecutionResult::Noop)
    }

    async fn execute_close(&mut self, asset: &RyskAsset) -> Result<ExecutionResult> {
        let asset_key = Self::asset_key(asset);

        println!("  RYSK CLOSE: all {} positions", asset_key);

        let mut returned = 0.0;
        self.positions.retain(|p| {
            if p.asset == asset_key && p.is_short {
                returned += p.collateral;
                println!(
                    "  RYSK: closed {} (returning ${:.2} collateral)",
                    p.key, p.collateral
                );
                false
            } else {
                true
            }
        });

        if returned > 0.0 {
            self.total_collateral_locked -= returned;
            if self.dry_run {
                println!("  RYSK: [DRY RUN] would close positions on-chain");
            }
            return Ok(ExecutionResult::TokenOutput {
                token: "USDC".to_string(),
                amount: returned,
            });
        }

        Ok(ExecutionResult::Noop)
    }
}

#[async_trait]
impl Venue for RyskOptions {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        match node {
            Node::Options {
                asset,
                action,
                delta_target,
                days_to_expiry,
                min_apy,
                roll_days_before,
                ..
            } => match action {
                OptionsAction::SellCoveredCall => {
                    self.execute_sell(
                        asset,
                        false,
                        *delta_target,
                        *days_to_expiry,
                        *min_apy,
                        input_amount,
                    )
                    .await
                }
                OptionsAction::SellCashSecuredPut => {
                    self.execute_sell(
                        asset,
                        true,
                        *delta_target,
                        *days_to_expiry,
                        *min_apy,
                        input_amount,
                    )
                    .await
                }
                OptionsAction::BuyCall | OptionsAction::BuyPut => {
                    let is_put = matches!(action, OptionsAction::BuyPut);
                    println!(
                        "  RYSK BUY_{}: {} with ${:.2}",
                        if is_put { "PUT" } else { "CALL" },
                        Self::asset_key(asset),
                        input_amount,
                    );
                    if self.dry_run {
                        println!("  RYSK: [DRY RUN] would buy option via RFQ");
                    }
                    Ok(ExecutionResult::PositionUpdate {
                        consumed: input_amount,
                        output: None,
                    })
                }
                OptionsAction::CollectPremium => self.execute_collect_premium(asset).await,
                OptionsAction::Roll => {
                    self.execute_roll(
                        asset,
                        *delta_target,
                        *days_to_expiry,
                        *min_apy,
                        *roll_days_before,
                    )
                    .await
                }
                OptionsAction::Close => self.execute_close(asset).await,
            },
            _ => {
                println!("  RYSK: unsupported node type '{}'", node.type_name());
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        Ok(self.total_collateral_locked + self.total_premium_collected)
    }

    async fn tick(&mut self, _now: u64, _dt_secs: f64) -> Result<()> {
        if !self.positions.is_empty() {
            let now = chrono::Utc::now().timestamp() as u64;
            for pos in &self.positions {
                let dte = if pos.expiry > now {
                    (pos.expiry - now) as f64 / 86400.0
                } else {
                    0.0
                };
                println!(
                    "  RYSK TICK: {} {} strike={:.0} DTE={:.1} premium=${:.2}",
                    if pos.is_short { "SHORT" } else { "LONG" },
                    pos.key,
                    pos.strike,
                    dte,
                    pos.premium,
                );
            }
        }
        Ok(())
    }

    async fn unwind(&mut self, _fraction: f64) -> Result<f64> {
        // European options cannot be closed before expiry.
        // Collateral is locked until settlement — nothing to free.
        if !self.positions.is_empty() {
            println!(
                "  RYSK: UNWIND skipped — {} EU option(s) locked until expiry",
                self.positions.len()
            );
        }
        Ok(0.0)
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            premium_pnl: self.metrics.premium_pnl,
            ..SimMetrics::default()
        }
    }
}
