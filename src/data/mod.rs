pub mod csv_types;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::node::NodeId;

/// Entry in the data manifest mapping a node ID to its CSV data file.
#[derive(Debug, Deserialize, Clone)]
pub struct ManifestEntry {
    pub file: String,
    pub kind: String,
}

/// Load the data manifest from `manifest.json` in the data directory.
pub fn load_manifest(data_dir: &Path) -> Result<HashMap<NodeId, ManifestEntry>> {
    let manifest_path = data_dir.join("manifest.json");
    let contents = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading manifest at {}", manifest_path.display()))?;
    let manifest: HashMap<NodeId, ManifestEntry> = serde_json::from_str(&contents)
        .with_context(|| "parsing manifest.json")?;
    Ok(manifest)
}

/// Load CSV rows of type T from a file in the data directory.
pub fn load_csv<T: for<'de> Deserialize<'de>>(data_dir: &Path, filename: &str) -> Result<Vec<T>> {
    let path = data_dir.join(filename);
    let mut rdr = csv::Reader::from_path(&path)
        .with_context(|| format!("opening CSV file {}", path.display()))?;
    let rows: Vec<T> = rdr
        .deserialize()
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing CSV file {}", path.display()))?;
    Ok(rows)
}

/// Extract all timestamps from loaded CSV data to build the simulation clock.
pub fn collect_timestamps(data_dir: &Path, manifest: &HashMap<NodeId, ManifestEntry>) -> Result<Vec<u64>> {
    let mut timestamps = Vec::new();

    for entry in manifest.values() {
        match entry.kind.as_str() {
            "perp" => {
                let rows: Vec<csv_types::PerpCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "options" => {
                let rows: Vec<csv_types::OptionsCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "lp" => {
                let rows: Vec<csv_types::LpCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "lending" => {
                let rows: Vec<csv_types::LendingCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "vault" => {
                let rows: Vec<csv_types::VaultCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "pendle" => {
                let rows: Vec<csv_types::PendleCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            "spot" => {
                let rows: Vec<csv_types::PriceCsvRow> = load_csv(data_dir, &entry.file)?;
                timestamps.extend(rows.iter().map(|r| r.timestamp));
            }
            _ => {} // wallet, swap, bridge, optimizer don't have CSVs
        }
    }

    Ok(timestamps)
}
