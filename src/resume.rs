use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::run::registry::Registry;

/// Resume all previously-running strategy daemons.
///
/// Reads the registry, clears stale entries, then spawns a detached
/// `defi-flow run` process for each entry. The spawned processes
/// self-register with their new PIDs.
pub fn run(registry_dir: Option<&Path>) -> Result<()> {
    let reg = Registry::load(registry_dir)?;

    if reg.daemons.is_empty() {
        println!("No strategies to resume.");
        return Ok(());
    }

    println!("Found {} strategies to resume:", reg.daemons.len());

    // Collect entries before clearing (clone the data we need)
    let entries: Vec<_> = reg.daemons.into_iter().collect();

    // Clear stale registry — spawned processes will re-register with new PIDs
    let empty = Registry::default();
    empty.save(registry_dir)?;

    let mut resumed = 0;

    for (name, entry) in &entries {
        if !entry.strategy_file.exists() {
            eprintln!(
                "  SKIP '{}': strategy file missing ({})",
                name,
                entry.strategy_file.display()
            );
            continue;
        }

        // Ensure log file parent directory exists
        if let Some(parent) = entry.log_file.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Ensure state file parent directory exists
        if let Some(parent) = entry.state_file.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let mut cmd = Command::new("defi-flow");
        cmd.arg("run")
            .arg(&entry.strategy_file)
            .arg("--state-file")
            .arg(&entry.state_file)
            .arg("--log-file")
            .arg(&entry.log_file)
            .arg("--network")
            .arg(&entry.network);

        if entry.mode == "dry-run" {
            cmd.arg("--dry-run");
        }

        if let Some(dir) = registry_dir {
            cmd.arg("--registry-dir").arg(dir);
        }

        // Detach: redirect stdout/stderr to log file, don't inherit stdin
        let log_file = File::create(&entry.log_file)
            .with_context(|| format!("creating log file {}", entry.log_file.display()))?;
        let log_err = log_file
            .try_clone()
            .with_context(|| "cloning log file handle")?;

        cmd.stdout(log_file).stderr(log_err).stdin(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                println!("  Resumed '{}' (PID {})", name, child.id());
                resumed += 1;
            }
            Err(e) => {
                eprintln!("  FAILED '{}': {}", name, e);
            }
        }
    }

    println!("\nResumed {}/{} strategies.", resumed, entries.len());

    Ok(())
}
