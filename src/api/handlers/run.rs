use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::broadcast;

use crate::api::error::ApiError;
use crate::api::events::EngineEvent;
use crate::api::state::{AppState, RunSession};
use crate::api::types::{RunListEntry, RunStartRequest, RunStartResponse, RunStatusResponse};

pub async fn start_run(
    State(state): State<AppState>,
    Json(req): Json<RunStartRequest>,
) -> Result<Json<RunStartResponse>, ApiError> {
    // Validate
    if let Err(errs) = crate::validate::validate(&req.workflow) {
        let msgs: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
        return Err(ApiError::Validation(msgs));
    }

    // Resolve private key: request body > env var
    let private_key = req
        .private_key
        .or_else(|| std::env::var("DEFI_FLOW_PRIVATE_KEY").ok())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "private_key required (pass in request or set DEFI_FLOW_PRIVATE_KEY env var)"
                    .to_string(),
            )
        })?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let (event_tx, _) = broadcast::channel::<EngineEvent>(256);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let event_log = Arc::new(tokio::sync::Mutex::new(Vec::<EngineEvent>::new()));

    // Write workflow JSON to temp file for the CLI
    let tmp_dir = std::env::temp_dir().join("defi-flow-runs");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| ApiError::Internal(format!("creating temp dir: {e}")))?;
    let workflow_path = tmp_dir.join(format!("{}.json", session_id));
    let workflow_json = serde_json::to_string_pretty(&req.workflow)
        .map_err(|e| ApiError::Internal(format!("serializing workflow: {e}")))?;
    std::fs::write(&workflow_path, &workflow_json)
        .map_err(|e| ApiError::Internal(format!("writing temp workflow: {e}")))?;

    // Build CLI args
    let binary =
        std::env::current_exe().map_err(|e| ApiError::Internal(format!("finding binary: {e}")))?;
    let state_file = tmp_dir.join(format!("{}-state.json", session_id));

    let mut args = vec![
        "run".to_string(),
        workflow_path.to_string_lossy().to_string(),
        "--network".to_string(),
        req.network.clone(),
        "--state-file".to_string(),
        state_file.to_string_lossy().to_string(),
        "--slippage-bps".to_string(),
        req.slippage_bps.to_string(),
    ];
    if req.dry_run {
        args.push("--dry-run".to_string());
        args.push("--once".to_string());
    }

    let session = RunSession {
        workflow: req.workflow.clone(),
        shutdown_tx: shutdown_tx.clone(),
        event_tx: event_tx.clone(),
        event_log: event_log.clone(),
        started_at: now,
        network: req.network.clone(),
        dry_run: req.dry_run,
    };

    {
        let mut state_inner = state.inner.write().await;
        state_inner.sessions.insert(session_id.clone(), session);
    }

    // Spawn CLI process
    let sid = session_id.clone();
    let state_clone = state.clone();
    let mut shutdown_rx = shutdown_tx.subscribe();

    tokio::spawn(async move {
        run_cli_process(
            binary,
            args,
            private_key,
            event_tx,
            event_log,
            &mut shutdown_rx,
            &sid,
            state_clone,
        )
        .await;
    });

    Ok(Json(RunStartResponse {
        session_id,
        status: "running".to_string(),
    }))
}

/// Helper: send event to broadcast channel AND store in event log for replay.
async fn emit(
    event_tx: &broadcast::Sender<EngineEvent>,
    event_log: &tokio::sync::Mutex<Vec<EngineEvent>>,
    event: EngineEvent,
) {
    event_log.lock().await.push(event.clone());
    let _ = event_tx.send(event);
}

async fn run_cli_process(
    binary: std::path::PathBuf,
    args: Vec<String>,
    private_key: String,
    event_tx: broadcast::Sender<EngineEvent>,
    event_log: Arc<tokio::sync::Mutex<Vec<EngineEvent>>>,
    shutdown_rx: &mut broadcast::Receiver<()>,
    session_id: &str,
    state: AppState,
) {
    // Spawn the CLI child process
    let child = TokioCommand::new(&binary)
        .args(&args)
        .env("DEFI_FLOW_PRIVATE_KEY", &private_key)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            emit(
                &event_tx,
                &event_log,
                EngineEvent::Error {
                    node_id: None,
                    message: format!("failed to spawn CLI: {e}"),
                },
            )
            .await;
            return;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    // Stream stdout/stderr as events until process exits or shutdown signal
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                let _ = child.kill().await;
                emit(&event_tx, &event_log, EngineEvent::Stopped {
                    reason: "user stopped".to_string(),
                }).await;
                break;
            }
            line = stdout_reader.next_line() => {
                match line {
                    Ok(Some(text)) => {
                        let event = parse_cli_line(&text);
                        emit(&event_tx, &event_log, event).await;
                    }
                    Ok(None) => {
                        // stdout closed — process exited
                        break;
                    }
                    Err(e) => {
                        emit(&event_tx, &event_log, EngineEvent::Error {
                            node_id: None,
                            message: format!("reading stdout: {e}"),
                        }).await;
                        break;
                    }
                }
            }
            line = stderr_reader.next_line() => {
                match line {
                    Ok(Some(text)) => {
                        let event = parse_stderr_line(&text);
                        emit(&event_tx, &event_log, event).await;
                    }
                    Ok(None) => {} // stderr closed
                    Err(_) => {}
                }
            }
        }
    }

    // Wait for process to fully exit
    let exit_status = child.wait().await;
    let reason = match exit_status {
        Ok(s) if s.success() => "process exited successfully".to_string(),
        Ok(s) => format!("process exited with code {}", s.code().unwrap_or(-1)),
        Err(e) => format!("process error: {e}"),
    };

    emit(&event_tx, &event_log, EngineEvent::Stopped { reason }).await;

    // Don't remove session — keep events around for UI to read
    // Clean up after 5 minutes
    let state_clone = state.clone();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let mut state_inner = state_clone.inner.write().await;
        state_inner.sessions.remove(&sid);
    });
}

/// Parse a CLI stdout line into an EngineEvent.
/// The CLI prints structured output like:
///   "Deploy order: [...]"
///   "  Execute: node_id"
///   "[HH:MM:SS] TVL: $1234.56"
///   "[HH:MM:SS] Triggered: [...]"
///   "Deploy complete. State saved."
fn parse_cli_line(line: &str) -> EngineEvent {
    let trimmed = line.trim();

    // Deploy order line
    if trimmed.starts_with("Deploy order:") {
        let nodes_str = trimmed.trim_start_matches("Deploy order:").trim();
        // Parse the debug format ["a", "b", ...]
        let nodes: Vec<String> = nodes_str
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        return EngineEvent::Deployed { nodes, tvl: 0.0 };
    }

    // TVL line
    if trimmed.contains("TVL:") {
        if let Some(tvl_str) = trimmed.split("TVL: $").nth(1) {
            if let Ok(tvl) = tvl_str.trim().parse::<f64>() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                return EngineEvent::TickCompleted {
                    timestamp: now,
                    tvl,
                };
            }
        }
    }

    // Execute line
    if trimmed.starts_with("Execute:") {
        let node_id = trimmed.trim_start_matches("Execute:").trim().to_string();
        return EngineEvent::NodeExecuted {
            node_id,
            action: "execute".to_string(),
            amount: 0.0,
        };
    }

    // Reserve action
    if trimmed.starts_with("[reserve]") {
        return EngineEvent::ReserveAction {
            action: trimmed.to_string(),
            amount: 0.0,
        };
    }

    // Explicit ERROR lines (e.g. "  ERROR executing node 'x': ...")
    if trimmed.starts_with("ERROR") {
        return EngineEvent::Error {
            node_id: None,
            message: trimmed.to_string(),
        };
    }

    // Everything else → generic log as NodeExecuted with the line as action
    EngineEvent::NodeExecuted {
        node_id: "cli".to_string(),
        action: trimmed.to_string(),
        amount: 0.0,
    }
}

/// Parse a CLI stderr line. Not all stderr is errors — the CLI prints
/// optimizer diagnostics, Kelly stats, etc. to stderr.
fn parse_stderr_line(line: &str) -> EngineEvent {
    let trimmed = line.trim();

    // Kelly optimizer diagnostics (not errors)
    if trimmed.starts_with("[kelly]") || trimmed.starts_with("[optimizer]") {
        return EngineEvent::NodeExecuted {
            node_id: "kelly".to_string(),
            action: trimmed.to_string(),
            amount: 0.0,
        };
    }

    // Reserve diagnostics
    if trimmed.starts_with("[reserve]") {
        return EngineEvent::ReserveAction {
            action: trimmed.to_string(),
            amount: 0.0,
        };
    }

    // Reload diagnostics
    if trimmed.starts_with("[reload]") {
        return EngineEvent::NodeExecuted {
            node_id: "cli".to_string(),
            action: trimmed.to_string(),
            amount: 0.0,
        };
    }

    // Actual errors
    EngineEvent::Error {
        node_id: None,
        message: trimmed.to_string(),
    }
}

pub async fn list_runs(State(state): State<AppState>) -> Result<Json<Vec<RunListEntry>>, ApiError> {
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

    // Read TVL from last TickCompleted event in the log
    let tvl = {
        let log = session.event_log.lock().await;
        log.iter()
            .rev()
            .find_map(|e| match e {
                EngineEvent::TickCompleted { tvl, .. } => Some(*tvl),
                _ => None,
            })
            .unwrap_or(0.0)
    };

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
