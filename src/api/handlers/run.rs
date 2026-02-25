use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use tokio::sync::{broadcast, RwLock};

use crate::api::error::ApiError;
use crate::api::events::EngineEvent;
use crate::api::state::{AppState, RunSession};
use crate::api::types::{RunListEntry, RunStartRequest, RunStartResponse, RunStatusResponse};
use crate::engine::Engine;
use crate::run::config::RuntimeConfig;
use crate::run::scheduler::CronScheduler;
use crate::run::RunConfig;
use crate::venues::{self, BuildMode};

pub async fn start_run(
    State(state): State<AppState>,
    Json(req): Json<RunStartRequest>,
) -> Result<Json<RunStartResponse>, ApiError> {
    // Validate
    if let Err(errs) = crate::validate::validate(&req.workflow) {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        return Err(ApiError::Validation(msgs));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let (event_tx, _) = broadcast::channel::<EngineEvent>(256);

    // Build runtime config
    let cli_config = RunConfig {
        network: req.network.clone(),
        state_file: std::path::PathBuf::from("/dev/null"),
        dry_run: req.dry_run,
        once: false,
        slippage_bps: req.slippage_bps,
    };

    let config = RuntimeConfig::from_cli(&cli_config)
        .map_err(|e| ApiError::BadRequest(format!("invalid config: {:#}", e)))?;

    // Build venues
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let tokens = req.workflow.token_manifest();
    let contracts = req.workflow.contracts.clone().unwrap_or_default();
    let venue_map = venues::build_all(
        &req.workflow,
        &BuildMode::Live {
            config: &config,
            tokens: &tokens,
            contracts: &contracts,
        },
    )
    .map_err(|e| ApiError::Internal(format!("building venues: {:#}", e)))?;

    let engine = Engine::new(req.workflow.clone(), venue_map);
    let engine = Arc::new(RwLock::new(engine));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let session = RunSession {
        workflow: req.workflow.clone(),
        engine: engine.clone(),
        shutdown_tx: shutdown_tx.clone(),
        event_tx: event_tx.clone(),
        started_at: now,
        network: req.network.clone(),
        dry_run: req.dry_run,
    };

    {
        let mut state_inner = state.inner.write().await;
        state_inner
            .sessions
            .insert(session_id.clone(), session);
    }

    // Spawn daemon loop
    let sid = session_id.clone();
    let state_clone = state.clone();
    let mut shutdown_rx = shutdown_tx.subscribe();

    tokio::spawn(async move {
        run_daemon_loop(engine, event_tx, &mut shutdown_rx, &sid, state_clone).await;
    });

    Ok(Json(RunStartResponse {
        session_id,
        status: "running".to_string(),
    }))
}

async fn run_daemon_loop(
    engine: Arc<RwLock<Engine>>,
    event_tx: broadcast::Sender<EngineEvent>,
    shutdown_rx: &mut broadcast::Receiver<()>,
    session_id: &str,
    state: AppState,
) {
    // Deploy phase
    {
        let mut eng = engine.write().await;
        if let Err(e) = eng.deploy().await {
            let _ = event_tx.send(EngineEvent::Error {
                node_id: None,
                message: format!("deploy failed: {:#}", e),
            });
            return;
        }
        let tvl = eng.total_tvl().await;
        let nodes: Vec<String> = eng.deploy_order().iter().cloned().collect();
        let _ = event_tx.send(EngineEvent::Deployed { nodes, tvl });
    }

    // Build scheduler
    let scheduler_workflow = {
        let eng = engine.read().await;
        eng.workflow.clone()
    };
    let mut scheduler = CronScheduler::new(&scheduler_workflow);

    if !scheduler.has_triggers() {
        let _ = event_tx.send(EngineEvent::Stopped {
            reason: "no triggered nodes".to_string(),
        });
        return;
    }

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                let _ = event_tx.send(EngineEvent::Stopped {
                    reason: "user stopped".to_string(),
                });
                break;
            }
            triggered = scheduler.wait_for_next() => {
                let now = chrono::Utc::now().timestamp() as u64;
                let mut eng = engine.write().await;

                for node_id in &triggered {
                    match eng.execute_node(node_id).await {
                        Ok(()) => {
                            let _ = event_tx.send(EngineEvent::NodeExecuted {
                                node_id: node_id.clone(),
                                action: "execute".to_string(),
                                amount: 0.0,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(EngineEvent::Error {
                                node_id: Some(node_id.clone()),
                                message: format!("{:#}", e),
                            });
                        }
                    }
                }

                let tvl = eng.total_tvl().await;
                let _ = event_tx.send(EngineEvent::TickCompleted {
                    timestamp: now,
                    tvl,
                });
            }
        }
    }

    // Clean up session
    let mut state_inner = state.inner.write().await;
    state_inner.sessions.remove(session_id);
}

pub async fn list_runs(
    State(state): State<AppState>,
) -> Result<Json<Vec<RunListEntry>>, ApiError> {
    let state_inner = state.inner.read().await;
    let entries: Vec<RunListEntry> = state_inner
        .sessions
        .iter()
        .map(|(id, s)| RunListEntry {
            session_id: id.clone(),
            workflow_name: s.workflow.name.clone(),
            status: "running".to_string(),
            network: s.network.clone(),
            started_at: s.started_at,
        })
        .collect();
    Ok(Json(entries))
}

pub async fn get_run_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<RunStatusResponse>, ApiError> {
    let state_inner = state.inner.read().await;
    let session = state_inner
        .sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::NotFound(format!("session '{session_id}' not found")))?;

    let eng = session.engine.read().await;
    let tvl = eng.total_tvl().await;

    Ok(Json(RunStatusResponse {
        session_id: session_id.clone(),
        status: "running".to_string(),
        tvl,
        started_at: session.started_at,
        network: session.network.clone(),
        dry_run: session.dry_run,
        workflow_name: session.workflow.name.clone(),
    }))
}

pub async fn stop_run(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let state_inner = state.inner.read().await;
    let session = state_inner
        .sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::NotFound(format!("session '{session_id}' not found")))?;

    let _ = session.shutdown_tx.send(());

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "status": "stopping",
    })))
}
