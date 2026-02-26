use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, broadcast};

use crate::model::workflow::Workflow;

use super::events::EngineEvent;
use super::history::HistoryStore;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<RwLock<AppStateInner>>,
}

pub struct AppStateInner {
    pub sessions: HashMap<String, RunSession>,
    pub data_dir: PathBuf,
    pub history: HistoryStore,
}

pub struct RunSession {
    pub workflow: Workflow,
    pub shutdown_tx: broadcast::Sender<()>,
    pub event_tx: broadcast::Sender<EngineEvent>,
    /// All events emitted so far, for replay on SSE connect.
    pub event_log: Arc<Mutex<Vec<EngineEvent>>>,
    pub started_at: u64,
    pub network: String,
    pub dry_run: bool,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        let history_dir = data_dir.join("history");
        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                sessions: HashMap::new(),
                data_dir,
                history: HistoryStore::new(history_dir),
            })),
        }
    }
}
