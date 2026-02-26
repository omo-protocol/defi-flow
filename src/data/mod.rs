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
    let manifest: HashMap<NodeId, ManifestEntry> =
        serde_json::from_str(&contents).with_context(|| "parsing manifest.json")?;
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

/// Extract timestamps from all CSV data sources, align to the overlapping time
/// range, and resample to a uniform cadence based on the densest source.
///
/// This ensures:
/// - The simulation only covers the period where ALL venues have data
/// - dt_secs is uniform across all ticks (consistent interest/funding accrual)
/// - No venue starts with stale forward-filled data
pub fn collect_timestamps(
    data_dir: &Path,
    manifest: &HashMap<NodeId, ManifestEntry>,
) -> Result<Vec<u64>> {
    // Collect timestamps per data source (not mixed)
    let mut per_source: Vec<(String, Vec<u64>)> = Vec::new();

    for entry in manifest.values() {
        let ts: Vec<u64> = match entry.kind.as_str() {
            "perp" => {
                let rows: Vec<csv_types::PerpCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "options" => {
                let rows: Vec<csv_types::OptionsCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "lending" => {
                let rows: Vec<csv_types::LendingCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "vault" => {
                let rows: Vec<csv_types::VaultCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "pendle" => {
                let rows: Vec<csv_types::PendleCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "spot" => {
                let rows: Vec<csv_types::PriceCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            "lp" => {
                let rows: Vec<csv_types::LpCsvRow> = load_csv(data_dir, &entry.file)?;
                rows.iter().map(|r| r.timestamp).collect()
            }
            _ => continue,
        };
        if !ts.is_empty() {
            per_source.push((entry.file.clone(), ts));
        }
    }

    if per_source.is_empty() {
        return Ok(vec![]);
    }

    // Sort each source's timestamps
    for (_, ts) in &mut per_source {
        ts.sort();
    }

    // Single source — return its native timestamps (already aligned)
    if per_source.len() == 1 {
        let (_, ts) = per_source.into_iter().next().unwrap();
        return Ok(ts);
    }

    // Find the overlapping time range across all sources
    let global_start = per_source
        .iter()
        .map(|(_, ts)| *ts.first().unwrap())
        .max()
        .unwrap();
    let global_end = per_source
        .iter()
        .map(|(_, ts)| *ts.last().unwrap())
        .min()
        .unwrap();

    if global_start >= global_end {
        // No overlap — fall back to full union of all timestamps
        eprintln!("  warning: data sources do not overlap, using union of all timestamps");
        let mut all: Vec<u64> = per_source.into_iter().flat_map(|(_, ts)| ts).collect();
        all.sort();
        all.dedup();
        return Ok(all);
    }

    // Compute cadence from the densest source (median interval for robustness)
    let cadence = per_source
        .iter()
        .filter_map(|(_, ts)| {
            if ts.len() < 2 {
                return None;
            }
            let mut intervals: Vec<u64> = ts.windows(2).map(|w| w[1] - w[0]).collect();
            intervals.sort();
            Some(intervals[intervals.len() / 2]) // median
        })
        .min()
        .unwrap_or(86400);

    // Generate uniform ticks within the overlap
    let ticks: Vec<u64> = (0..)
        .map(|i: u64| global_start + i * cadence)
        .take_while(|&t| t <= global_end)
        .collect();

    let start_str = format_ts(global_start);
    let end_str = format_ts(global_end);
    eprintln!(
        "  Aligned {} data source(s): [{} → {}], cadence {}h, {} ticks",
        per_source.len(),
        start_str,
        end_str,
        cadence / 3600,
        ticks.len(),
    );

    Ok(ticks)
}

fn format_ts(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ts.to_string())
}
