use clap::Parser;

mod backtest;
mod cli;
mod data;
mod engine;
mod example;
mod fetch_data;
mod list_nodes;
mod model;
mod run;
mod schema;
mod sim;
mod validate;
mod visualize;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Command::Schema => schema::run(),
        cli::Command::Validate { file } => validate::run(&file),
        cli::Command::Visualize { file } => visualize::run(&file),
        cli::Command::ListNodes => list_nodes::run(),
        cli::Command::Example => example::run(),
        cli::Command::Backtest {
            file,
            data_dir,
            capital,
            slippage_bps,
            seed,
            verbose,
            output,
        } => backtest::run(&backtest::BacktestConfig {
            workflow_path: file,
            data_dir,
            capital,
            slippage_bps,
            seed,
            verbose,
            output,
        }),
        cli::Command::Run {
            file,
            network,
            state_file,
            dry_run,
            once,
            slippage_bps,
        } => run::run(&file, &run::RunConfig {
            network,
            state_file,
            dry_run,
            once,
            slippage_bps,
        }),
        cli::Command::FetchData {
            file,
            output_dir,
            days,
            interval,
        } => fetch_data::run(&file, &output_dir, days, &interval),
    }
}
