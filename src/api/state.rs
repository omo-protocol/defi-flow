use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, broadcast};

use crate::model::workflow::Workflow;

use super::db::Db;
use super::events::EngineEvent;
use super::history::HistoryStore;
use super::rate_limit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<RwLock<AppStateInner>>,
}

pub struct AppStateInner {
    pub sessions: HashMap<String, RunSession>,
    pub data_dir: PathBuf,
    pub history: HistoryStore,
    pub db: Db,
    pub auth_secret: String,
    pub ai_api_key: String,
    pub ai_base_url: String,
    pub ai_model: String,
    pub rate_limiter: RateLimiter,
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
    pub fn new(
        data_dir: PathBuf,
        db: Db,
        auth_secret: String,
        ai_api_key: String,
        ai_base_url: String,
        ai_model: String,
    ) -> Self {
        let history_dir = data_dir.join("history");
        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                sessions: HashMap::new(),
                data_dir,
                history: HistoryStore::new(history_dir),
                db,
                auth_secret,
                ai_api_key,
                ai_base_url,
                ai_model,
                rate_limiter: RateLimiter::new(),
            })),
        }
    }
}

