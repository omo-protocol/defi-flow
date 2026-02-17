use anyhow::{bail, Result};

use crate::data::csv_types::LpCsvRow;
use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, SimMetrics, VenueSimulator};
use crate::model::node::{LpAction, Node};

// ── Uniswap V3 / Slipstream concentrated liquidity math ──────────────

/// Full tick range in Uniswap V3 (MIN_TICK to MAX_TICK).
const FULL_TICK_RANGE: f64 = 887_272.0 * 2.0;

/// Convert a Uniswap V3 tick to sqrt price.
///
/// Formula: `sqrt_price = 1.0001^(tick / 2)`
fn tick_to_sqrt_price(tick: i32) -> f64 {
    1.0001_f64.powf(tick as f64 / 2.0)
}

/// Calculate token amounts from a concentrated liquidity position.
///
/// Ported from keeper's `math_utils.py::calculate_amounts_from_liquidity`.
///
/// Returns `(amount0, amount1)` as floating-point token units.
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
        // Below range: only token0
        let amount0 = liquidity * (sp_hi - sp_lo) / (sp_hi * sp_lo);
        (amount0, 0.0)
    } else if sqrt_price_current >= sp_hi {
        // Above range: only token1
        let amount1 = liquidity * (sp_hi - sp_lo);
        (0.0, amount1)
    } else {
        // In range: both tokens
        let amount0 = liquidity * (sp_hi - sqrt_price_current) / (sqrt_price_current * sp_hi);
        let amount1 = liquidity * (sqrt_price_current - sp_lo);
        (amount0, amount1)
    }
}

/// Fee concentration multiplier for a concentrated liquidity position.
///
/// A position covering a narrow tick range earns proportionally more fees
/// per unit of liquidity than a full-range position:
///
///   multiplier ≈ sqrt(full_range_ticks / position_range_ticks)
///
/// Returns 1.0 for full-range positions.
fn fee_concentration_multiplier(tick_lower: i32, tick_upper: i32) -> f64 {
    let range = (tick_upper - tick_lower) as f64;
    if range <= 0.0 {
        return 1.0;
    }
    (FULL_TICK_RANGE / range).sqrt()
}

/// Whether the current tick falls within the position's range.
fn is_in_range(current_tick: i32, tick_lower: i32, tick_upper: i32) -> bool {
    current_tick >= tick_lower && current_tick < tick_upper
}

// ── LP Simulator ─────────────────────────────────────────────────────

/// Aerodrome Slipstream LP simulator — concentrated liquidity with Uniswap V3 math.
///
/// Models:
/// - Concentrated liquidity positions with tick ranges (NFT-based)
/// - Fee accrual with concentration multiplier (tighter range = more fees)
/// - AERO gauge reward emissions (proportional to liquidity)
/// - Position value via V3 amounts math (IL is implicit)
/// - Out-of-range detection (no fees when price leaves tick range)
pub struct LpSimulator {
    market_data: Vec<LpCsvRow>,
    cursor: usize,

    // Position state (V3 concentrated liquidity)
    /// Virtual liquidity (L) in the Uniswap V3 sense.
    virtual_liquidity: f64,
    /// Original USD deposit (for tracking purposes).
    deposit_usd: f64,
    /// Lower tick bound of the position.
    tick_lower: i32,
    /// Upper tick bound of the position.
    tick_upper: i32,
    /// Fee concentration multiplier: sqrt(full_range / position_range).
    concentration: f64,

    // Accruals
    pub accrued_fees: f64,
    pub accrued_rewards: f64,
    staked_in_gauge: bool,

    // Tracking
    ticks_in_range: u64,
    ticks_out_of_range: u64,
}

impl LpSimulator {
    pub fn new(market_data: Vec<LpCsvRow>) -> Self {
        Self {
            market_data,
            cursor: 0,
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

    fn advance_cursor(&mut self, clock: &SimClock) {
        let ts = clock.current_timestamp();
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= ts
        {
            self.cursor += 1;
        }
    }

    /// Compute the current USD value of the concentrated liquidity position
    /// using Uniswap V3 amounts math.
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

    /// Deposit USD into a concentrated liquidity position.
    ///
    /// Calculates virtual liquidity (L) from the USD amount based on
    /// current prices and tick range, following Uniswap V3 math.
    fn deposit(&mut self, usd_amount: f64, tick_lower: i32, tick_upper: i32) {
        let row = self.current_row().clone();

        self.tick_lower = tick_lower;
        self.tick_upper = tick_upper;
        self.concentration = fee_concentration_multiplier(tick_lower, tick_upper);

        let sqrt_current = tick_to_sqrt_price(row.current_tick);
        let sqrt_lower = tick_to_sqrt_price(tick_lower);
        let sqrt_upper = tick_to_sqrt_price(tick_upper);

        // Calculate USD value per unit of liquidity at current price/range.
        // L=1 produces (amount0_per_l, amount1_per_l) tokens.
        let (a0_per_l, a1_per_l) =
            calculate_amounts_from_liquidity(1.0, sqrt_current, sqrt_lower, sqrt_upper);
        let usd_per_l = a0_per_l * row.price_a + a1_per_l * row.price_b;

        let new_liquidity = if usd_per_l > 0.0 {
            usd_amount / usd_per_l
        } else {
            // Fallback: if out of range at deposit time, use deposit_usd as virtual L
            usd_amount
        };

        self.virtual_liquidity += new_liquidity;
        self.deposit_usd += usd_amount;
    }
}

impl VenueSimulator for LpSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
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

        self.advance_cursor(clock);

        match action {
            LpAction::AddLiquidity => {
                // Use tick bounds from node, or default to full range
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
                // Reinvest accrued fees into the position
                let fees = self.accrued_fees;
                if fees > 0.0 {
                    self.accrued_fees = 0.0;
                    // Re-deposit fees at current tick range
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

    fn total_value(&self, _clock: &SimClock) -> f64 {
        let pos_value = self.position_value_usd();
        let row = self.current_row();
        let reward_value = self.accrued_rewards * row.reward_token_price;
        pos_value + self.accrued_fees + reward_value
    }

    fn tick(&mut self, clock: &SimClock) -> Result<()> {
        self.advance_cursor(clock);

        if self.virtual_liquidity <= 0.0 {
            return Ok(());
        }

        let dt = clock.dt_years();
        if dt <= 0.0 {
            return Ok(());
        }

        let row = self.current_row().clone();
        let in_range = is_in_range(row.current_tick, self.tick_lower, self.tick_upper);

        if in_range {
            self.ticks_in_range += 1;
            // Accrue trading fees with concentration multiplier.
            // Pool-wide fee_apy is amplified for concentrated positions.
            let position_value = self.position_value_usd();
            self.accrued_fees += position_value * row.fee_apy * self.concentration * dt;
        } else {
            self.ticks_out_of_range += 1;
            // Out of range: no trading fees accrue
        }

        // AERO gauge rewards accrue regardless of price range
        // (gauge rewards are proportional to liquidity staked, not trading activity)
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
}
