use clap::Parser;

mod api;
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
mod validate;
mod venues;
mod visualize;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    match cli.command {
        cli::Command::Schema => schema::run(),
        cli::Command::Validate { file } => validate::run(&file),
        cli::Command::Visualize { file, format, scope, output } => {
            visualize::run(&file, &format, scope.as_deref(), output.as_deref())
        }
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
            tick_csv,
            monte_carlo,
        } => backtest::run(&backtest::BacktestConfig {
            workflow_path: file,
            data_dir,
            capital,
            slippage_bps,
            seed,
            verbose,
            output,
            tick_csv,
            monte_carlo: monte_carlo.map(|n| backtest::monte_carlo::MonteCarloConfig {
                n_simulations: n,
            }),
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
        cli::Command::Api {
            host,
            port,
            data_dir,
        } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(api::serve(&host, port, &data_dir))
        }
        cli::Command::FetchData {
            file,
            output_dir,
            days,
            interval,
        } => fetch_data::run(&file, &output_dir, days, &interval),
    }
}
