use anyhow::{bail, Result};
use async_trait::async_trait;

use super::data::LpCsvRow;
use crate::model::node::{LpAction, Node};
use crate::venues::{ExecutionResult, RiskParams, SimMetrics, Venue};

const SECS_PER_YEAR: f64 = 365.25 * 86400.0;

// ── Uniswap V3 / Slipstream concentrated liquidity math ──────────────

const FULL_TICK_RANGE: f64 = 887_272.0 * 2.0;

fn tick_to_sqrt_price(tick: i32) -> f64 {
    1.0001_f64.powf(tick as f64 / 2.0)
}

fn calculate_amounts_from_liquidity(
    liquidity: f64,
    sqrt_price_current: f64,
    sqrt_price_lower: f64,
    sqrt_price_upper: f64,
) -> (f64, f64) {
    let (sp_lo, sp_hi) = if sqrt_price_lower <= sqrt_price_upper {
        (sqrt_price_lower, sqrt_price_upper)
    } else {
        (sqrt_price_upper, sqrt_price_lower)
    };

    if sqrt_price_current <= sp_lo {
        let amount0 = liquidity * (sp_hi - sp_lo) / (sp_hi * sp_lo);
        (amount0, 0.0)
    } else if sqrt_price_current >= sp_hi {
        let amount1 = liquidity * (sp_hi - sp_lo);
        (0.0, amount1)
    } else {
        let amount0 = liquidity * (sp_hi - sqrt_price_current) / (sqrt_price_current * sp_hi);
        let amount1 = liquidity * (sqrt_price_current - sp_lo);
        (amount0, amount1)
    }
}

fn fee_concentration_multiplier(tick_lower: i32, tick_upper: i32) -> f64 {
    let range = (tick_upper - tick_lower) as f64;
    if range <= 0.0 {
        return 1.0;
    }
    (FULL_TICK_RANGE / range).sqrt()
}

fn is_in_range(current_tick: i32, tick_lower: i32, tick_upper: i32) -> bool {
    current_tick >= tick_lower && current_tick < tick_upper
}

// ── LP Simulator ─────────────────────────────────────────────────────

pub struct LpSimulator {
    market_data: Vec<LpCsvRow>,
    cursor: usize,
    current_ts: u64,

    virtual_liquidity: f64,
    deposit_usd: f64,
    tick_lower: i32,
    tick_upper: i32,
    concentration: f64,

    pub accrued_fees: f64,
    pub accrued_rewards: f64,
    staked_in_gauge: bool,

    ticks_in_range: u64,
    ticks_out_of_range: u64,
}

impl LpSimulator {
    pub fn new(market_data: Vec<LpCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
            current_ts: 0,
            virtual_liquidity: 0.0,
            deposit_usd: 0.0,
            tick_lower: -887_272,
            tick_upper: 887_272,
            concentration: 1.0,
            accrued_fees: 0.0,
            accrued_rewards: 0.0,
            staked_in_gauge: false,
            ticks_in_range: 0,
            ticks_out_of_range: 0,
        }
    }

    fn current_row(&self) -> &LpCsvRow {
        &self.market_data[self.cursor.min(self.market_data.len() - 1)]
    }

    fn advance_cursor(&mut self) {
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= self.current_ts
        {
            self.cursor += 1;
        }
    }

    fn position_value_usd(&self) -> f64 {
        if self.virtual_liquidity <= 0.0 {
            return 0.0;
        }

        let row = self.current_row();
        let sqrt_current = tick_to_sqrt_price(row.current_tick);
        let sqrt_lower = tick_to_sqrt_price(self.tick_lower);
        let sqrt_upper = tick_to_sqrt_price(self.tick_upper);

        let (amount0, amount1) = calculate_amounts_from_liquidity(
            self.virtual_liquidity,
            sqrt_current,
            sqrt_lower,
            sqrt_upper,
        );

        amount0 * row.price_a + amount1 * row.price_b
    }

    fn deposit(&mut self, usd_amount: f64, tick_lower: i32, tick_upper: i32) {
        let row = self.current_row().clone();

        self.tick_lower = tick_lower;
        self.tick_upper = tick_upper;
        self.concentration = fee_concentration_multiplier(tick_lower, tick_upper);

        let sqrt_current = tick_to_sqrt_price(row.current_tick);
        let sqrt_lower = tick_to_sqrt_price(tick_lower);
        let sqrt_upper = tick_to_sqrt_price(tick_upper);

        let (a0_per_l, a1_per_l) =
            calculate_amounts_from_liquidity(1.0, sqrt_current, sqrt_lower, sqrt_upper);
        let usd_per_l = a0_per_l * row.price_a + a1_per_l * row.price_b;

        let new_liquidity = if usd_per_l > 0.0 {
            usd_amount / usd_per_l
        } else {
            usd_amount
        };

        self.virtual_liquidity += new_liquidity;
        self.deposit_usd += usd_amount;
    }
}

#[async_trait]
impl Venue for LpSimulator {
    async fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
    ) -> Result<ExecutionResult> {
        let (action, tick_lower, tick_upper) = match node {
            Node::Lp {
                action,
                tick_lower,
                tick_upper,
                ..
            } => (action, *tick_lower, *tick_upper),
            _ => bail!("LpSimulator called on non-lp node"),
        };

        self.advance_cursor();

        match action {
            LpAction::AddLiquidity => {
                let tl = tick_lower.unwrap_or(-887_272);
                let tu = tick_upper.unwrap_or(887_272);
                self.deposit(input_amount, tl, tu);
                Ok(ExecutionResult::PositionUpdate {
                    consumed: input_amount,
                    output: None,
                })
            }
            LpAction::RemoveLiquidity => {
                let pos_value = self.position_value_usd();
                let total = pos_value + self.accrued_fees;
                self.virtual_liquidity = 0.0;
                self.deposit_usd = 0.0;
                self.accrued_fees = 0.0;
                Ok(ExecutionResult::TokenOutput {
                    token: "USDC".to_string(),
                    amount: total.max(0.0),
                })
            }
            LpAction::ClaimRewards => {
                let rewards = self.accrued_rewards;
                self.accrued_rewards = 0.0;
                if rewards > 0.0 {
                    let row = self.current_row();
                    let reward_value = rewards * row.reward_token_price;
                    Ok(ExecutionResult::TokenOutput {
                        token: "AERO".to_string(),
                        amount: reward_value,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
            LpAction::Compound => {
                let fees = self.accrued_fees;
                if fees > 0.0 {
                    self.accrued_fees = 0.0;
                    let tl = self.tick_lower;
                    let tu = self.tick_upper;
                    self.deposit(fees, tl, tu);
                }
                Ok(ExecutionResult::Noop)
            }
            LpAction::StakeGauge => {
                self.staked_in_gauge = true;
                Ok(ExecutionResult::Noop)
            }
            LpAction::UnstakeGauge => {
                self.staked_in_gauge = false;
                Ok(ExecutionResult::Noop)
            }
        }
    }

    async fn total_value(&self) -> Result<f64> {
        let pos_value = self.position_value_usd();
        let row = self.current_row();
        let reward_value = self.accrued_rewards * row.reward_token_price;
        Ok(pos_value + self.accrued_fees + reward_value)
    }

    async fn tick(&mut self, now: u64, dt_secs: f64) -> Result<()> {
        self.current_ts = now;
        self.advance_cursor();

        if self.virtual_liquidity <= 0.0 {
            return Ok(());
        }

        let dt = dt_secs / SECS_PER_YEAR;
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();
        let in_range = is_in_range(row.current_tick, self.tick_lower, self.tick_upper);

        if in_range {
            self.ticks_in_range += 1;
            let position_value = self.position_value_usd();
            self.accrued_fees += position_value * row.fee_apy * self.concentration * dt;
        } else {
            self.ticks_out_of_range += 1;
        }

        let position_value = self.position_value_usd();
        if position_value > 0.0 {
            self.accrued_rewards += position_value * row.reward_rate * dt;
        }

        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            lp_fees: self.accrued_fees,
            ..Default::default()
        }
    }

    fn alpha_stats(&self) -> Option<(f64, f64)> {
        if self.market_data.len() < 10 {
            return None;
        }

        let slice = &self.market_data;
        let n = slice.len() as f64;

        // fee_apy scaled by concentration + reward_rate (both annualized)
        let apys: Vec<f64> = slice
            .iter()
            .map(|r| r.fee_apy * self.concentration + r.reward_rate)
            .collect();
        let mean = apys.iter().sum::<f64>() / n;
        let var = apys.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);

        Some((mean, var.sqrt()))
    }

    fn risk_params(&self) -> Option<RiskParams> {
        if self.market_data.len() < 20 {
            return None;
        }

        // Estimate P(out of range) from historical tick moves
        let data = &self.market_data;
        let out_of_range_count = data
            .iter()
            .filter(|r| !is_in_range(r.current_tick, self.tick_lower, self.tick_upper))
            .count();
        let p_out_of_range = out_of_range_count as f64 / data.len() as f64;

        // IL severity: concentrated positions amplify IL
        // Full-range IL at 2x price move ≈ 5.7%
        // Concentrated IL ≈ base_il * sqrt(concentration)
        let base_il = 0.057;
        let severity = (base_il * self.concentration.sqrt()).min(1.0);

        // p_loss: probability of being out of range (loss of fee income + IL)
        // Scale by severity to get expected loss event probability
        let p_loss = p_out_of_range.min(1.0);

        Some(RiskParams {
            p_loss,
            loss_severity: severity,
            rebalance_cost: 0.003, // ~0.3% (swap + gas for LP repositioning)
        })
    }
}
