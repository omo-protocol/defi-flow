use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use super::types::FetchResult;

#[derive(Serialize)]
struct ManifestEntry {
    file: String,
    kind: String,
}

/// Write manifest.json mapping node IDs to CSV filenames.
pub fn write_manifest(
    output_dir: &Path,
    entries: &[(String, String, String)], // (node_id, filename, kind)
) -> Result<()> {
    let map: HashMap<&str, ManifestEntry> = entries
        .iter()
        .map(|(id, file, kind)| {
            (
                id.as_str(),
                ManifestEntry {
                    file: file.clone(),
                    kind: kind.clone(),
                },
            )
        })
        .collect();

    let path = output_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(&map)?;
    std::fs::write(&path, json).context("writing manifest.json")?;
    Ok(())
}

/// Write a FetchResult to a CSV file.
pub fn write_fetch_result(output_dir: &Path, filename: &str, result: &FetchResult) -> Result<()> {
    let path = output_dir.join(filename);
    match result {
        FetchResult::Perp(rows) => write_csv(&path, rows),
        FetchResult::Options(rows) => write_csv(&path, rows),
        FetchResult::Lending(rows) => write_csv(&path, rows),
        FetchResult::Vault(rows) => write_csv(&path, rows),
        FetchResult::Pendle(rows) => write_csv(&path, rows),
        FetchResult::Price(rows) => write_csv(&path, rows),
        FetchResult::Lp(rows) => write_csv(&path, rows),
    }
}

fn write_csv<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)
        .with_context(|| format!("creating CSV file {}", path.display()))?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    Ok(())
}
