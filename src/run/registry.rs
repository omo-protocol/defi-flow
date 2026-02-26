use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::run::state::RunState;

/// Where the registry lives by default.
fn default_registry_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".defi-flow")
}

/// A single running (or recently-crashed) daemon entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub pid: u32,
    pub strategy_file: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
    pub mode: String,
    pub network: String,
    pub capital: f64,
    pub started_at: String,
}

/// The full registry of daemon processes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    pub daemons: HashMap<String, RegistryEntry>,
}

/// Status of a daemon after liveness check.
#[derive(Debug)]
pub enum DaemonStatus {
    Running,
    Crashed,
}

/// Combined info for display in `defi-flow ps`.
#[derive(Debug)]
pub struct DaemonInfo {
    pub name: String,
    pub entry: RegistryEntry,
    pub status: DaemonStatus,
    pub tvl: Option<f64>,
    pub last_tick: Option<u64>,
}

impl Registry {
    /// Path to the registry JSON file.
    pub fn path(registry_dir: Option<&Path>) -> PathBuf {
        registry_dir
            .map(|d| d.to_path_buf())
            .unwrap_or_else(default_registry_dir)
            .join("registry.json")
    }

    /// Load registry from disk, or return empty if it doesn't exist.
    pub fn load(registry_dir: Option<&Path>) -> Result<Self> {
        let path = Self::path(registry_dir);
        if !path.exists() {
            return Ok(Registry::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading registry at {}", path.display()))?;
        let registry: Registry = serde_json::from_str(&contents)
            .with_context(|| format!("parsing registry at {}", path.display()))?;
        Ok(registry)
    }

    /// Save registry to disk (creates parent dirs if needed).
    pub fn save(&self, registry_dir: Option<&Path>) -> Result<()> {
        let path = Self::path(registry_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating registry dir {}", parent.display()))?;
        }

        // Write atomically: write to tmp then rename
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, &json)
            .with_context(|| format!("writing registry tmp {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("renaming registry tmp → {}", path.display()))?;
        Ok(())
    }

    /// Register a new daemon.
    pub fn register(registry_dir: Option<&Path>, name: &str, entry: RegistryEntry) -> Result<()> {
        let mut reg = Self::load(registry_dir)?;
        reg.daemons.insert(name.to_string(), entry);
        reg.save(registry_dir)
    }

    /// Remove a daemon from the registry.
    pub fn deregister(registry_dir: Option<&Path>, name: &str) -> Result<()> {
        let mut reg = Self::load(registry_dir)?;
        reg.daemons.remove(name);
        reg.save(registry_dir)
    }

    /// Check if a PID is alive.
    pub fn is_pid_alive(pid: u32) -> bool {
        // kill(pid, 0) checks existence without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    /// Get info for all registered daemons (with liveness + TVL).
    pub fn status_all(registry_dir: Option<&Path>) -> Result<Vec<DaemonInfo>> {
        let reg = Self::load(registry_dir)?;
        let mut infos = Vec::new();

        for (name, entry) in &reg.daemons {
            let status = if Self::is_pid_alive(entry.pid) {
                DaemonStatus::Running
            } else {
                DaemonStatus::Crashed
            };

            // Try to read state file for TVL
            let (tvl, last_tick) = if entry.state_file.exists() {
                match RunState::load_or_new(&entry.state_file) {
                    Ok(state) => {
                        let total: f64 = state.balances.values().sum();
                        (Some(total), Some(state.last_tick))
                    }
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };

            infos.push(DaemonInfo {
                name: name.clone(),
                entry: entry.clone(),
                status,
                tvl,
                last_tick,
            });
        }

        // Sort by name for consistent output
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(infos)
    }
}
