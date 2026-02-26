use anyhow::{Result, bail};
use async_trait::async_trait;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::data::{PerpCsvRow, PriceCsvRow};
use crate::model::node::{Node, PerpAction, PerpDirection, SpotSide};
use crate::venues::{ExecutionResult, RiskParams, SimMetrics, Venue};

/// Maintenance margin rate — liquidation triggers when equity <= notional * this.
const MAINT_MARGIN_RATE: f64 = 0.01;
/// Liquidation penalty as fraction of remaining equity.
const LIQUIDATION_FEE: f64 = 0.02;
const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

#[derive(Default)]
struct SimulatedPosition {
    position_amt: f64,
    entry_price: f64,
    leverage: f64,
    isolated_margin: f64,
}

/// Perp simulator — tracks one position per instance, with funding accrual, liquidation, and slippage.
pub struct PerpSimulator {
    market_data: Vec<PerpCsvRow>,
    cursor: usize,
    current_ts: u64,
    position: SimulatedPosition,
    balance: f64,
    max_slippage_bps: f64,
    rng: StdRng,
    pub liquidation_count: u32,
    pub cumulative_funding: f64,
    pub cumulative_rewards: f64,
}

impl PerpSimulator {
    pub fn new(market_data: Vec<PerpCsvRow>, max_slippage_bps: f64, seed: u64) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            position: SimulatedPosition::default(),
            balance: 0.0,
            max_slippage_bps,
            rng: StdRng::seed_from_u64(seed),
            liquidation_count: 0,
            cumulative_funding: 0.0,
            cumulative_rewards: 0.0,
        }
    }

    fn current_market(&self) -> &PerpCsvRow {
        &self.market_data[self.cursor.min(self.market_data.len() - 1)]
    }

    fn advance_cursor(&mut self) {
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= self.current_ts
        {
            self.cursor += 1;
        }
    }

    fn compute_slippage(&mut self) -> f64 {
        if self.max_slippage_bps <= 0.0 {
            return 0.0;
        }
        let frac: f64 = self.rng.random();
        frac * self.max_slippage_bps / 10_000.0
    }

    fn check_and_liquidate(&mut self) {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return;
        }

        let row = self.current_market();
        let unrealized_pnl = pos.position_amt * (row.mark_price - pos.entry_price);
        let equity = pos.isolated_margin + unrealized_pnl + self.balance;
        let notional = pos.position_amt.abs() * row.mark_price;
        let maintenance_margin = notional * MAINT_MARGIN_RATE;

        if equity > maintenance_margin {
            return;
        }

        let remaining_equity = equity.max(0.0);
        let fee = remaining_equity * LIQUIDATION_FEE;
        let returned = (remaining_equity - fee).max(0.0);

        self.balance += returned;
        self.position = SimulatedPosition::default();
        self.liquidation_count += 1;
    }

    fn accrue_funding(&mut self, dt_secs: f64) {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return;
        }

        let row = self.current_market();
        // funding_rate is per-hour (Hyperliquid settles hourly).
        // Scale by actual elapsed hours for clock-cadence independence.
        let dt_hours = dt_secs / 3600.0;
        let notional = pos.position_amt.abs() * row.mark_price;

        let funding = if pos.position_amt > 0.0 {
            -notional * row.funding_rate * dt_hours
        } else {
            notional * row.funding_rate * dt_hours
        };

        self.cumulative_funding += funding;
        self.balance += funding;
    }

    fn accrue_rewards(&mut self, dt_secs: f64) {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return;
        }

        let row = self.current_market();
        if row.rewards_apy <= 0.0 {
            return;
        }

        let dt = dt_secs / SECS_PER_YEAR;
        let notional = pos.position_amt.abs() * row.mark_price;
        let reward = notional * row.rewards_apy * dt;

        self.cumulative_rewards += reward;
        self.balance += reward;
    }

    fn place_order(&mut self, direction: PerpDirection, leverage: f64, amount: f64) {
        let ask = self.current_market().ask;
        let bid = self.current_market().bid;
        let slippage = self.compute_slippage();

        let fill_price = match direction {
            PerpDirection::Long => ask * (1.0 + slippage),
            PerpDirection::Short => bid * (1.0 - slippage),
        };

        let signed_sz = match direction {
            PerpDirection::Long => amount / fill_price,
            PerpDirection::Short => -(amount / fill_price),
        };
        let order_sz = signed_sz.abs();

        let pos = &self.position;
        let old_amt = pos.position_amt;
        let same_direction =
            (old_amt >= 0.0 && signed_sz >= 0.0) || (old_amt <= 0.0 && signed_sz <= 0.0);

        if old_amt.abs() < 1e-12 || same_direction {
            let required_margin = amount / leverage;
            if self.balance < required_margin {
                return;
            }

            let old_notional = old_amt.abs() * self.position.entry_price;
            let new_notional = order_sz * fill_price;
            let new_amt = old_amt + signed_sz;
            self.position.entry_price = if new_amt.abs() > 1e-12 {
                (old_notional + new_notional) / new_amt.abs()
            } else {
                0.0
            };
            self.position.position_amt = new_amt;
            self.position.leverage = leverage;
            self.balance -= required_margin;
            self.position.isolated_margin += required_margin;
        } else {
            let close_amt = order_sz.min(old_amt.abs());
            let pnl_per_unit = if old_amt > 0.0 {
                fill_price - self.position.entry_price
            } else {
                self.position.entry_price - fill_price
            };
            let realized_pnl = close_amt * pnl_per_unit;
            let margin_fraction = close_amt / old_amt.abs();
            let released_margin = self.position.isolated_margin * margin_fraction;

            self.balance += released_margin + realized_pnl;
            self.position.isolated_margin -= released_margin;

            let remaining = old_amt.abs() - close_amt;
            if remaining < 1e-12 {
                self.position = SimulatedPosition::default();
            } else {
                self.position.position_amt = if old_amt > 0.0 { remaining } else { -remaining };
            }
        }
    }
}

#[async_trait]
impl Venue for PerpSimulator {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        let (action, direction, leverage, margin) = match node {
            Node::Perp {
                action,
                direction,
                leverage,
                ..
            } => (
                action,
                direction,
                leverage,
                node.margin_token().unwrap_or("USDC").to_string(),
            ),
            _ => bail!("PerpSimulator called on non-perp node"),
        };

        self.advance_cursor();

        match action {
            PerpAction::Open => {
                let dir = direction.unwrap_or(PerpDirection::Long);
                let lev = leverage.unwrap_or(1.0);
                self.balance += input_amount;
                self.place_order(dir, lev, input_amount);
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            PerpAction::Close => {
                let pos_amt = self.position.position_amt;
                if pos_amt.abs() > 1e-12 {
                    let dir = if pos_amt > 0.0 {
                        PerpDirection::Short
                    } else {
                        PerpDirection::Long
                    };
                    let close_value = pos_amt.abs() * self.current_market().mark_price;
                    self.place_order(dir, self.position.leverage, close_value);
                }
                let available = self.balance;
                self.balance = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: margin.clone(),
                    amount: available,
                })
            }
            PerpAction::Adjust => {
                if let Some(lev) = leverage {
                    self.position.leverage = *lev;
                }
                Ok(ExecutionResult::Noop)
            }
            PerpAction::CollectFunding => {
                let available = self.balance.max(0.0);
                if available > 0.0 {
                    self.balance -= available;
                    Ok(ExecutionResult::TokenOutput {
                        token: margin.clone(),
                        amount: available,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return Ok(self.balance);
        }
        let row = self.current_market();
        let unrealized_pnl = pos.position_amt * (row.mark_price - pos.entry_price);
        Ok(self.balance + pos.isolated_margin + unrealized_pnl)
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();
        self.accrue_funding(dt_secs);
        self.accrue_rewards(dt_secs);
        self.check_and_liquidate();
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            funding_pnl: self.cumulative_funding,
            rewards_pnl: self.cumulative_rewards,
            liquidations: self.liquidation_count,
            ..Default::default()
        }
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        let total = self.total_value().await?;
        if total <= 0.0 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);

        // Scale down position proportionally — same pattern as lending/vault/LP.
        // total_value() = balance + margin + unrealized_pnl, so scaling all
        // components by (1-f) guarantees freed + remaining == original total.
        if self.position.position_amt.abs() > 1e-12 {
            let remaining = self.position.position_amt.abs() * (1.0 - f);
            if remaining < 1e-12 {
                self.position = SimulatedPosition::default();
            } else {
                let sign = self.position.position_amt.signum();
                self.position.position_amt = remaining * sign;
                self.position.isolated_margin *= 1.0 - f;
            }
        }

        self.balance *= 1.0 - f;

        Ok(total * f)
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        // Use full dataset — in production, historical data is available before strategy starts.
        compute_funding_stats(&self.market_data)
    }

    fn risk_params(&self) -> Option<RiskParams> {
        compute_perp_risk(&self.market_data, self.max_slippage_bps)
    }

    fn margin_ratio(&self) -> Option<f64> {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return None; // no position
        }
        let row = self.current_market();
        let unrealized_pnl = pos.position_amt * (row.mark_price - pos.entry_price);
        let equity = pos.isolated_margin + unrealized_pnl + self.balance;
        let notional = pos.position_amt.abs() * row.mark_price;
        if notional <= 0.0 {
            return None;
        }
        Some(equity / notional)
    }

    fn add_margin(&mut self, amount: f64) {
        self.position.isolated_margin += amount;
    }
}

// ── Spot Simulator ───────────────────────────────────────────────────

/// Spot trade simulator — uses price data with slippage.
/// Holds purchased tokens internally and tracks mark-to-market value.
pub struct SpotSimulator {
    market_data: Vec<PriceCsvRow>,
    cursor: usize,
    current_ts: u64,
    slippage_bps: f64,
    /// Amount of base token held (e.g. ETH from a USDC→ETH buy).
    held_amount: f64,
    /// Weighted-average entry price.
    entry_price: f64,
}

impl SpotSimulator {
    pub fn new(market_data: Vec<PriceCsvRow>, slippage_bps: f64) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            slippage_bps,
            held_amount: 0.0,
            entry_price: 0.0,
        }
    }

    fn current_row(&self) -> &PriceCsvRow {
        &self.market_data[self.cursor.min(self.market_data.len() - 1)]
    }

    fn advance_cursor(&mut self) {
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= self.current_ts
        {
            self.cursor += 1;
        }
    }
}

#[async_trait]
impl Venue for SpotSimulator {
    async fn execute(&mut self, node: &Node, input_amount: f64) -> Result<ExecutionResult> {
        let (pair, side) = match node {
            Node::Spot { pair, side, .. } => (pair.clone(), *side),
            _ => bail!("SpotSimulator called on non-spot node"),
        };

        self.advance_cursor();
        let row = self.current_row();
        let slippage = self.slippage_bps / 10_000.0;

        let parts: Vec<&str> = pair.split('/').collect();
        match side {
            SpotSide::Buy => {
                let price = row.ask * (1.0 + slippage);
                let token_amount = input_amount / price;
                let old_notional = self.held_amount * self.entry_price;
                self.held_amount += token_amount;
                self.entry_price = if self.held_amount > 1e-12 {
                    (old_notional + input_amount) / self.held_amount
                } else {
                    0.0
                };
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            SpotSide::Sell => {
                let price = row.bid * (1.0 - slippage);
                // Sell from held tokens if available, otherwise treat input as token amount.
                let sell_tokens = if self.held_amount > 1e-12 {
                    self.held_amount
                } else {
                    input_amount / price
                };
                let output_usd = sell_tokens * price;
                self.held_amount = 0.0;
                self.entry_price = 0.0;
                let quote = parts.get(1).unwrap_or(&"USDC").to_string();
                Ok(ExecutionResult::TokenOutput {
                    token: quote,
                    amount: output_usd,
                })
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        if self.held_amount > 1e-12 {
            Ok(self.held_amount * self.current_row().price)
        } else {
            Ok(0.0)
        }
    }

    async fn tick(&mut self, now: u64, _dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();
        Ok(())
    }

    async fn unwind(&mut self, fraction: f64) -> Result<f64> {
        if self.held_amount <= 1e-12 || fraction <= 0.0 {
            return Ok(0.0);
        }
        let f = fraction.min(1.0);
        let sell_amount = self.held_amount * f;
        let price = self.current_row().price;
        let freed = sell_amount * price;
        self.held_amount -= sell_amount;
        if self.held_amount < 1e-12 {
            self.held_amount = 0.0;
            self.entry_price = 0.0;
        }
        Ok(freed)
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        // Spot has no inherent yield — it's purely directional.
        // In a delta-neutral group, this contributes (0, 0) so the
        // group stats are driven entirely by the perp's funding alpha.
        Some((0.0, 0.0))
    }
}

// ── Alpha stats helpers ────────────────────────────────────────────────

/// Compute perp risk parameters from historical price data.
///
/// - p_loss: annualized probability of liquidation (price move > threshold)
/// - loss_severity: fraction of margin lost at liquidation (~0.98)
/// - rebalance_cost: slippage per rebalance
fn compute_perp_risk(data: &[PerpCsvRow], slippage_bps: f64) -> Option<RiskParams> {
    if data.len() < 20 {
        return None;
    }

    // Compute annualized price volatility from mark_price log-returns
    let mut log_returns = Vec::with_capacity(data.len() - 1);
    for i in 1..data.len() {
        if data[i].mark_price > 0.0 && data[i - 1].mark_price > 0.0 {
            log_returns.push((data[i].mark_price / data[i - 1].mark_price).ln());
        }
    }

    if log_returns.len() < 10 {
        return None;
    }

    let n = log_returns.len() as f64;
    let mean = log_returns.iter().sum::<f64>() / n;
    let var = log_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let per_period_vol = var.sqrt();

    // Annualize using actual period length
    let total_secs = (data.last().unwrap().timestamp - data[0].timestamp) as f64;
    let avg_period_secs = total_secs / (data.len() - 1) as f64;
    let periods_per_year = (365.25 * 86400.0) / avg_period_secs;
    let annual_vol = per_period_vol * periods_per_year.sqrt();

    if annual_vol <= 0.0 {
        return None;
    }

    // For short at 1x leverage: liquidation when price rises by ~98%
    // threshold = (1 + 1/L) / (1 + MAINT_MARGIN_RATE)
    // At 1x: threshold ≈ 1.98, ln(1.98) ≈ 0.683
    // P(liquidation) = Φ(-ln(threshold) / annual_vol)
    let leverage = 1.0_f64; // conservative: assume 1x
    let threshold = (1.0 + 1.0 / leverage) / (1.0 + MAINT_MARGIN_RATE);
    let z = -(threshold.ln()) / annual_vol;
    let p_liq = normal_cdf(z);

    Some(RiskParams {
        p_loss: p_liq,
        loss_severity: 0.98, // lose almost all margin on liquidation
        rebalance_cost: slippage_bps / 10_000.0,
    })
}

/// Standard normal CDF — Abramowitz & Stegun approximation (max error ~7.5e-8).
fn normal_cdf(x: f64) -> f64 {
    if x >= 8.0 {
        return 1.0;
    }
    if x <= -8.0 {
        return 0.0;
    }

    let t = 1.0 / (1.0 + 0.2316419 * x.abs());
    let d = 0.3989422804014327; // 1/sqrt(2*pi)
    let p = d * (-x * x / 2.0).exp();
    let c = t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));

    if x >= 0.0 { 1.0 - p * c } else { p * c }
}

/// Compute annualized funding rate stats from perp data up to `cursor`.
/// Returns (annualized_return, annualized_volatility) of funding income.
///
/// Funding rate is per-hour on Hyperliquid. For a short position, positive
/// funding = income. We compute the mean and std of per-period funding rates,
/// then annualize.
fn compute_funding_stats(data: &[PerpCsvRow]) -> Option<(f64, f64)> {
    if data.len() < 10 {
        return None; // not enough data
    }

    let slice = data;

    // Compute per-period funding rates (already per-hour from Hyperliquid)
    // Include rewards_apy contribution per period
    let mut returns = Vec::with_capacity(slice.len());
    for i in 1..slice.len() {
        let dt_hours = (slice[i].timestamp.saturating_sub(slice[i - 1].timestamp)) as f64 / 3600.0;
        if dt_hours <= 0.0 {
            continue;
        }
        // funding_rate is per-hour; for short: positive rate = income
        let funding = slice[i].funding_rate * dt_hours;
        // rewards_apy is annualized; convert to per-period
        let rewards = slice[i].rewards_apy * dt_hours / 8760.0;
        returns.push(funding + rewards);
    }

    if returns.len() < 10 {
        return None;
    }

    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let std = var.sqrt();

    // Compute average period in hours for annualization
    let total_hours = (slice.last().unwrap().timestamp - slice[0].timestamp) as f64 / 3600.0;
    let avg_period_hours = total_hours / (slice.len() - 1) as f64;
    let periods_per_year = 8760.0 / avg_period_hours;

    let annualized_return = mean * periods_per_year;
    let annualized_vol = std * periods_per_year.sqrt();

    Some((annualized_return, annualized_vol))
}
