pub mod error;
pub mod events;
pub mod handlers;
pub mod history;
pub mod state;
pub mod types;

use std::path::Path;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use tower_http::cors::{Any, CorsLayer};

use state::AppState;

pub async fn serve(host: &str, port: u16, data_dir: &Path) -> Result<()> {
    let data_dir = if data_dir.starts_with("~") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(data_dir.strip_prefix("~").unwrap())
    } else {
        data_dir.to_path_buf()
    };

    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;

    let state = AppState::new(data_dir.clone());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Health
        .route("/health", get(|| async { "ok" }))
        // Validate
        .route("/api/validate", post(handlers::validate::validate_workflow))
        // Backtest
        .route("/api/backtest", post(handlers::backtest::run_backtest))
        .route("/api/backtests", get(handlers::backtest::list_backtests))
        .route("/api/backtest/{id}", get(handlers::backtest::get_backtest))
        // Run
        .route("/api/run/start", post(handlers::run::start_run))
        .route("/api/runs", get(handlers::run::list_runs))
        .route("/api/run/{id}/status", get(handlers::run::get_run_status))
        .route("/api/run/{id}/stop", post(handlers::run::stop_run))
        .route("/api/run/{id}/events", get(handlers::events::event_stream))
        // Data
        .route("/api/data/fetch", post(handlers::data::fetch_data))
        .route("/api/data/upload", post(handlers::data::upload_data))
        .route("/api/data/manifest", get(handlers::data::get_manifest))
        // Schema
        .route("/api/schema", get(handlers::schema::get_schema))
        .layer(cors)
        .with_state(state);

    let addr = format!("{host}:{port}");
    println!("defi-flow API server listening on {addr}");
    println!("  Health:   GET  http://{addr}/health");
    println!("  Schema:   GET  http://{addr}/api/schema");
    println!("  Validate: POST http://{addr}/api/validate");
    println!("  Backtest: POST http://{addr}/api/backtest");
    println!("  Run:      POST http://{addr}/api/run/start");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;

    axum::serve(listener, app).await.context("running server")?;

    Ok(())
}
