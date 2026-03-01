use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::engine::Engine;
use crate::run::config::RuntimeConfig;
use crate::run::state::RunState;
use crate::venues::{self, evm, BuildMode};

/// Entry point for the `query` command.
/// Parses the strategy JSON, builds live venues, queries on-chain TVL per venue,
/// and outputs a JSON object with per-node breakdown + wallet token balances.
pub fn run(workflow_path: &Path, state_file: Option<&Path>) -> Result<()> {
    let workflow = crate::validate::load_and_validate(workflow_path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
    })?;

    // Build RuntimeConfig — needs DEFI_FLOW_PRIVATE_KEY for wallet address.
    // Uses dry_run=false so venues will attempt on-chain queries in total_value().
    let config = RuntimeConfig::from_args(
        resolve_private_key()?,
        "mainnet",
        false, // dry_run=false → on-chain queries
        50.0,
    )?;

    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    rt.block_on(query_async(&workflow, &config, state_file))
}

fn resolve_private_key() -> Result<String> {
    if let Ok(pk) = std::env::var("DEFI_FLOW_PRIVATE_KEY") {
        return Ok(pk);
    }
    if let Ok(path) = std::env::var("DEFI_FLOW_PRIVATE_KEY_FILE") {
        return Ok(std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read private key from {path}: {e}"))?
            .trim()
            .to_string());
    }
    Err(anyhow::anyhow!(
        "DEFI_FLOW_PRIVATE_KEY or DEFI_FLOW_PRIVATE_KEY_FILE required"
    ))
}

async fn query_async(
    workflow: &crate::model::workflow::Workflow,
    config: &RuntimeConfig,
    state_file: Option<&Path>,
) -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let tokens = workflow.token_manifest();
    let contracts = workflow.contracts.clone().unwrap_or_default();
    let venue_map = venues::build_all(
        workflow,
        &BuildMode::Live {
            config,
            tokens: &tokens,
            contracts: &contracts,
        },
    )?;

    let engine = Engine::new(workflow.clone(), venue_map);

    // Query per-venue on-chain values
    let mut venues_json = serde_json::Map::new();
    let mut venue_total = 0.0;

    for node in &workflow.nodes {
        let id = node.id();
        let node_type = node.type_name();
        if let Some(venue) = engine.venues.get(id) {
            let val = venue.total_value().await.unwrap_or(0.0);
            venue_total += val;
            if val > 0.001 {
                venues_json.insert(
                    id.to_string(),
                    json!({ "type": node_type, "value": round2(val) }),
                );
            }
        }
    }

    // Query wallet token balances
    let wallet_tokens =
        crate::run::query_wallet_all_tokens(workflow, config, &tokens).await;
    let mut wallet_json = serde_json::Map::new();
    let mut wallet_total = 0.0;
    for (sym, bal) in &wallet_tokens {
        wallet_total += bal;
        wallet_json.insert(sym.clone(), json!(round2(*bal)));
    }

    let total_tvl = venue_total + wallet_total;

    let mut result = serde_json::Map::new();
    result.insert("strategy".into(), json!(workflow.name));
    result.insert("wallet".into(), json!(format!("{:?}", config.wallet_address)));
    result.insert("total_tvl".into(), json!(round2(total_tvl)));
    result.insert("venue_tvl".into(), json!(round2(venue_total)));
    result.insert("wallet_balance".into(), json!(round2(wallet_total)));
    result.insert("venues".into(), json!(venues_json));
    result.insert("wallet_tokens".into(), json!(wallet_json));

    // Include state file metrics if available
    let sf = state_file.or_else(|| {
        // Try to find state file from registry
        None
    });
    if let Some(path) = sf {
        if let Ok(state) = RunState::load_or_new(path) {
            result.insert(
                "state".into(),
                json!({
                    "deploy_completed": state.deploy_completed,
                    "initial_capital": round2(state.initial_capital),
                    "last_tvl": round2(state.last_tvl),
                    "peak_tvl": round2(state.peak_tvl),
                    "last_tick": state.last_tick,
                    "cumulative_funding": round2(state.cumulative_funding),
                    "cumulative_interest": round2(state.cumulative_interest),
                    "cumulative_rewards": round2(state.cumulative_rewards),
                    "cumulative_costs": round2(state.cumulative_costs),
                }),
            );
        }
    }

    println!("{}", serde_json::to_string(&serde_json::Value::Object(result))?);
    Ok(())
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
