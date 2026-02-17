use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// DeFi workflow engine — validate, visualize, and generate schemas
/// for LLM-produced DeFi quant strategy workflows.
#[derive(Parser)]
#[command(name = "defi-flow", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Output the JSON schema for workflow definitions (for LLM consumption)
    Schema,

    /// Validate a workflow JSON file
    Validate {
        /// Path to the workflow JSON file
        file: PathBuf,
    },

    /// Visualize a workflow as ASCII, DOT, SVG, or PNG
    Visualize {
        /// Path to the workflow JSON file
        file: PathBuf,

        /// Output format: ascii (default), dot, svg, or png
        #[arg(long, default_value = "ascii")]
        format: String,

        /// Render only the subgraph between two nodes (format: "from_node:to_node")
        #[arg(long)]
        scope: Option<String>,

        /// Output file path (default: stdout for ascii/dot, required for svg/png)
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },

    /// List all available node types and their parameters
    ListNodes,

    /// Output an example workflow JSON to stdout
    Example,

    /// Backtest a workflow against historical data
    Backtest {
        /// Path to the workflow JSON file
        file: PathBuf,

        /// Directory containing CSV data files and manifest.json
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,

        /// Initial capital in USD
        #[arg(long, default_value = "10000.0")]
        capital: f64,

        /// Default slippage for swaps/bridges (basis points)
        #[arg(long, default_value = "10.0")]
        slippage_bps: f64,

        /// Random seed for slippage simulation
        #[arg(long, default_value = "42")]
        seed: u64,

        /// Print verbose tick-by-tick output
        #[arg(long)]
        verbose: bool,

        /// Output results as JSON to this file
        #[arg(long)]
        output: Option<PathBuf>,

        /// Run N Monte Carlo simulations (block bootstrap + GBM perturbation)
        #[arg(long)]
        monte_carlo: Option<u32>,

        /// Block size for bootstrap resampling (default: 10)
        #[arg(long, default_value = "10")]
        block_size: usize,

        /// GBM volatility scale factor (0.0 = no perturbation, 1.0 = full historical vol)
        #[arg(long, default_value = "1.0")]
        gbm_vol_scale: f64,
    },

    /// Run a workflow live with on-chain execution
    Run {
        /// Path to the workflow JSON file
        file: PathBuf,

        /// Network to execute on (mainnet or testnet)
        #[arg(long, default_value = "testnet")]
        network: String,

        /// Path to state file for persistence across restarts
        #[arg(long, default_value = "state.json")]
        state_file: PathBuf,

        /// Log actions without executing (paper trading)
        #[arg(long)]
        dry_run: bool,

        /// Execute once then exit (for external cron)
        #[arg(long)]
        once: bool,

        /// Slippage tolerance in basis points for market orders
        #[arg(long, default_value = "50")]
        slippage_bps: f64,
    },

    /// Fetch historical data from venue APIs for backtesting
    FetchData {
        /// Path to the workflow JSON file
        file: PathBuf,

        /// Output directory for CSV files and manifest.json
        #[arg(long, default_value = "data")]
        output_dir: PathBuf,

        /// Number of days of history to fetch
        #[arg(long, default_value = "365")]
        days: u32,

        /// Interval between data points (e.g. "8h", "1d")
        #[arg(long, default_value = "8h")]
        interval: String,
    },
}
