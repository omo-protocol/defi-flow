use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub label: String,
    pub twrr_pct: f64,
    pub annualized_pct: f64,
    pub max_drawdown_pct: f64,
    pub sharpe: f64,
    pub net_pnl: f64,
    pub rebalances: u32,
    pub liquidations: u32,
    pub funding_pnl: f64,
    pub premium_pnl: f64,
    pub lp_fees: f64,
    pub lending_interest: f64,
    pub swap_costs: f64,
    pub ticks: usize,
}

impl BacktestResult {
    pub fn print_table(results: &[Self]) {
        println!("\n{}", "═".repeat(130));
        println!("  Backtest Results");
        println!("{}", "═".repeat(130));
        println!(
            "  {:<30} {:>7} {:>7} {:>7} {:>7} {:>6} {:>5} {:>10} {:>10} {:>8} {:>8} {:>8}",
            "Strategy",
            "TWRR%",
            "Ann.%",
            "MxDD%",
            "Sharpe",
            "Rebal",
            "Liqs",
            "Funding",
            "Premium",
            "LP Fees",
            "Lending",
            "NetPnL",
        );
        println!("  {}", "-".repeat(124));
        for r in results {
            println!(
                "  {:<30} {:>+7.2} {:>+7.2} {:>7.2} {:>7.3} {:>6} {:>5} {:>+10.2} {:>+10.2} {:>+8.2} {:>+8.2} {:>+8.2}",
                r.label,
                r.twrr_pct,
                r.annualized_pct,
                r.max_drawdown_pct,
                r.sharpe,
                r.rebalances,
                r.liquidations,
                r.funding_pnl,
                r.premium_pnl,
                r.lp_fees,
                r.lending_interest,
                r.net_pnl,
            );
        }
        println!("{}", "═".repeat(130));
        if let Some(r) = results.first() {
            println!("  {} ticks, swap costs: {:.2}", r.ticks, r.swap_costs);
        }
    }
}
