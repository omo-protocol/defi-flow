use anyhow::{bail, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::data::csv_types::PerpCsvRow;
use crate::engine::clock::SimClock;
use crate::engine::venue::{ExecutionResult, SimMetrics, VenueSimulator};
use crate::model::node::{Node, PerpAction, PerpDirection};

/// Maintenance margin rate — liquidation triggers when equity <= notional * this.
const MAINT_MARGIN_RATE: f64 = 0.01;
/// Liquidation penalty as fraction of remaining equity.
const LIQUIDATION_FEE: f64 = 0.02;
/// Periods per year for funding accrual (8-hour periods).
const PERIODS_PER_YEAR: f64 = 365.0 * 3.0;

#[derive(Default)]
struct SimulatedPosition {
    position_amt: f64,
    entry_price: f64,
    leverage: f64,
    isolated_margin: f64,
}

/// Perp simulator — ported from markowitz's MockPerpsClient.
/// Tracks one position per instance, with funding accrual, liquidation, and slippage.
pub struct PerpSimulator {
    market_data: Vec<PerpCsvRow>,
    cursor: usize,
    position: SimulatedPosition,
    balance: f64,
    max_slippage_bps: f64,
    rng: StdRng,
    pub liquidation_count: u32,
    pub cumulative_funding: f64,
}

impl PerpSimulator {
    pub fn new(market_data: Vec<PerpCsvRow>, max_slippage_bps: f64, seed: u64) -> Self {
        Self {
            market_data,
            cursor: 0,
            position: SimulatedPosition::default(),
            balance: 0.0,
            max_slippage_bps,
            rng: StdRng::seed_from_u64(seed),
            liquidation_count: 0,
            cumulative_funding: 0.0,
        }
    }

    fn current_market(&self) -> &PerpCsvRow {
        &self.market_data[self.cursor.min(self.market_data.len() - 1)]
    }

    fn advance_cursor(&mut self, clock: &SimClock) {
        let ts = clock.current_timestamp();
        // Find the latest row with timestamp <= ts
        while self.cursor + 1 < self.market_data.len()
            && self.market_data[self.cursor + 1].timestamp <= ts
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
        let equity = pos.isolated_margin + unrealized_pnl;
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

    fn accrue_funding(&mut self) {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return;
        }

        let row = self.current_market();
        let funding_per_period = row.funding_apy / PERIODS_PER_YEAR;
        let notional = pos.position_amt.abs() * row.mark_price;

        // Shorts earn positive funding, longs pay
        let funding = if pos.position_amt > 0.0 {
            -notional * funding_per_period // longs pay
        } else {
            notional * funding_per_period // shorts earn
        };

        self.cumulative_funding += funding;
        self.balance += funding;
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
            // Closing/flipping
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

impl VenueSimulator for PerpSimulator {
    fn execute(
        &mut self,
        node: &Node,
        input_amount: f64,
        clock: &SimClock,
    ) -> Result<ExecutionResult> {
        let (action, direction, leverage) = match node {
            Node::Perp {
                action,
                direction,
                leverage,
                ..
            } => (action, direction, leverage),
            _ => bail!("PerpSimulator called on non-perp node"),
        };

        self.advance_cursor(clock);

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
                    token: "USDC".to_string(),
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
                // Funding has already been accrued via tick(). Return accumulated balance.
                let available = self.balance.max(0.0);
                if available > 0.0 {
                    self.balance -= available;
                    Ok(ExecutionResult::TokenOutput {
                        token: "USDC".to_string(),
                        amount: available,
                    })
                } else {
                    Ok(ExecutionResult::Noop)
                }
            }
        }
    }

    fn total_value(&self, _clock: &SimClock) -> f64 {
        let pos = &self.position;
        if pos.position_amt.abs() < 1e-12 {
            return self.balance;
        }
        let row = self.current_market();
        let unrealized_pnl = pos.position_amt * (row.mark_price - pos.entry_price);
        self.balance + pos.isolated_margin + unrealized_pnl
    }

    fn tick(&mut self, clock: &SimClock) -> Result<()> {
        self.advance_cursor(clock);
        self.accrue_funding();
        self.check_and_liquidate();
        Ok(())
    }

    fn metrics(&self) -> SimMetrics {
        SimMetrics {
            funding_pnl: self.cumulative_funding,
            liquidations: self.liquidation_count,
            ..Default::default()
        }
    }
}
