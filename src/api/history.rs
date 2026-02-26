use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::backtest::result::BacktestResult;
use crate::model::workflow::Workflow;

use super::events::EngineEvent;
use super::types::{BacktestRecord, BacktestSummary};

pub struct HistoryStore {
    dir: PathBuf,
}

impl HistoryStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn backtests_dir(&self) -> PathBuf {
        self.dir.join("backtests")
    }

    fn runs_dir(&self) -> PathBuf {
        self.dir.join("runs")
    }

    pub fn save_backtest(
        &self,
        id: &str,
        result: &BacktestResult,
        workflow: &Workflow,
    ) -> Result<()> {
        let dir = self.backtests_dir();
        std::fs::create_dir_all(&dir).context("creating backtests dir")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let record = serde_json::json!({
            "id": id,
            "workflow": workflow,
            "result": result,
            "created_at": now,
        });

        let path = dir.join(format!("{id}.json"));
        let file =
            std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        serde_json::to_writer_pretty(file, &record).context("writing backtest record")?;
        Ok(())
    }

    pub fn list_backtests(&self) -> Result<Vec<BacktestSummary>> {
        let dir = self.backtests_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&contents) {
                        results.push(BacktestSummary {
                            id: val["id"].as_str().unwrap_or("").to_string(),
                            label: val["result"]["label"].as_str().unwrap_or("").to_string(),
                            twrr_pct: val["result"]["twrr_pct"].as_f64().unwrap_or(0.0),
                            sharpe: val["result"]["sharpe"].as_f64().unwrap_or(0.0),
                            max_drawdown_pct: val["result"]["max_drawdown_pct"]
                                .as_f64()
                                .unwrap_or(0.0),
                            created_at: val["created_at"].as_u64().unwrap_or(0),
                        });
                    }
                }
            }
        }

        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    pub fn get_backtest(&self, id: &str) -> Result<BacktestRecord> {
        let path = self.backtests_dir().join(format!("{id}.json"));
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let val: serde_json::Value = serde_json::from_str(&contents)?;

        Ok(BacktestRecord {
            id: val["id"].as_str().unwrap_or("").to_string(),
            workflow: serde_json::from_value(val["workflow"].clone())?,
            result: serde_json::from_value(val["result"].clone())?,
            created_at: val["created_at"].as_u64().unwrap_or(0),
        })
    }

    pub fn save_run_log(
        &self,
        session_id: &str,
        events: &[EngineEvent],
        workflow: &Workflow,
    ) -> Result<()> {
        let dir = self.runs_dir();
        std::fs::create_dir_all(&dir).context("creating runs dir")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let record = serde_json::json!({
            "session_id": session_id,
            "workflow": workflow,
            "events": events,
            "stopped_at": now,
        });

        let path = dir.join(format!("{session_id}.json"));
        let file = std::fs::File::create(&path)?;
        serde_json::to_writer_pretty(file, &record)?;
        Ok(())
    }
}
