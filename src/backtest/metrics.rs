use super::result::BacktestResult;

/// Backtest metrics collector — ported from markowitz's BacktestMetrics.
/// Tracks TWRR, drawdown, Sharpe ratio, and per-venue counters.
pub struct BacktestMetrics {
    periods_per_year: f64,
    // TWRR
    twrr_product: f64,
    period_start_tvl: f64,
    // Drawdown
    peak_tvl: f64,
    max_drawdown: f64,
    // History
    tvl_history: Vec<f64>,
}

impl BacktestMetrics {
    pub fn new(initial_tvl: f64, periods_per_year: f64) -> Self {
        Self {
            periods_per_year,
            twrr_product: 1.0,
            period_start_tvl: initial_tvl,
            peak_tvl: initial_tvl,
            max_drawdown: 0.0,
            tvl_history: Vec::new(),
        }
    }

    /// Record end-of-tick TVL: updates peak, drawdown, pushes to history.
    pub fn record_tick(&mut self, tvl: f64) {
        if tvl > self.peak_tvl {
            self.peak_tvl = tvl;
        }
        let drawdown = if self.peak_tvl > 0.0 {
            (self.peak_tvl - tvl) / self.peak_tvl
        } else {
            0.0
        };
        if drawdown > self.max_drawdown {
            self.max_drawdown = drawdown;
        }
        self.tvl_history.push(tvl);
    }

    /// Finalize metrics and produce the result.
    pub fn finalize(
        self,
        label: String,
        initial_capital: f64,
        rebalances: u32,
        liquidations: u32,
        funding_pnl: f64,
        premium_pnl: f64,
        lp_fees: f64,
        lending_interest: f64,
        swap_costs: f64,
    ) -> BacktestResult {
        let final_tvl = self.tvl_history.last().copied().unwrap_or(initial_capital);

        // Close final TWRR sub-period
        let mut twrr_product = self.twrr_product;
        if self.period_start_tvl > 0.0 {
            twrr_product *= final_tvl / self.period_start_tvl;
        }
        let twrr = twrr_product - 1.0;

        let num_periods = self.tvl_history.len() as f64;
        let annualized_return = if num_periods > 0.0 {
            (1.0 + twrr).powf(self.periods_per_year / num_periods) - 1.0
        } else {
            0.0
        };

        let sharpe = self.compute_sharpe();
        let net_pnl = final_tvl - initial_capital;

        BacktestResult {
            label,
            twrr_pct: twrr * 100.0,
            annualized_pct: annualized_return * 100.0,
            max_drawdown_pct: self.max_drawdown * 100.0,
            sharpe,
            net_pnl,
            rebalances,
            liquidations,
            funding_pnl,
            premium_pnl,
            lp_fees,
            lending_interest,
            swap_costs,
            ticks: self.tvl_history.len(),
        }
    }

    fn compute_sharpe(&self) -> f64 {
        let returns: Vec<f64> = self
            .tvl_history
            .windows(2)
            .map(|w| {
                if w[0] > 0.0 {
                    (w[1] - w[0]) / w[0]
                } else {
                    0.0
                }
            })
            .collect();

        if returns.is_empty() {
            return 0.0;
        }

        let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = if returns.len() > 1 {
            returns
                .iter()
                .map(|r| (r - mean_ret).powi(2))
                .sum::<f64>()
                / (returns.len() - 1) as f64
        } else {
            0.0
        };
        let std_ret = var.sqrt();

        if std_ret > 0.0 {
            mean_ret / std_ret * self.periods_per_year.sqrt()
        } else {
            0.0
        }
    }
}
