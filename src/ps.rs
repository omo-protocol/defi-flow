use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::run::registry::{DaemonStatus, Registry};

pub fn run(registry_dir: Option<&Path>) -> Result<()> {
    let infos = Registry::status_all(registry_dir)?;

    if infos.is_empty() {
        println!("No strategies registered.");
        return Ok(());
    }

    // Header
    println!(
        "{:<24} {:<10} {:<10} {:<8} {:<12} {:<10} {}",
        "NAME", "MODE", "NETWORK", "PID", "TVL", "UPTIME", "STATUS"
    );
    println!("{}", "-".repeat(90));

    let now = Utc::now();

    for info in &infos {
        let status_str = match info.status {
            DaemonStatus::Running => "running",
            DaemonStatus::Crashed => "crashed",
        };

        let pid_str = match info.status {
            DaemonStatus::Running => format!("{}", info.entry.pid),
            DaemonStatus::Crashed => "—".to_string(),
        };

        let tvl_str = match info.tvl {
            Some(v) => format!("${:.0}", v),
            None => "—".to_string(),
        };

        let uptime_str = match info.entry.started_at.parse::<DateTime<Utc>>() {
            Ok(started) => {
                if matches!(info.status, DaemonStatus::Crashed) {
                    "—".to_string()
                } else {
                    format_duration(now.signed_duration_since(started))
                }
            }
            Err(_) => "—".to_string(),
        };

        println!(
            "{:<24} {:<10} {:<10} {:<8} {:<12} {:<10} {}",
            truncate(&info.name, 23),
            info.entry.mode,
            info.entry.network,
            pid_str,
            tvl_str,
            uptime_str,
            status_str,
        );
    }

    // Summary
    let running = infos
        .iter()
        .filter(|i| matches!(i.status, DaemonStatus::Running))
        .count();
    let crashed = infos
        .iter()
        .filter(|i| matches!(i.status, DaemonStatus::Crashed))
        .count();
    println!("\n{} running, {} crashed", running, crashed);

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max - 1])
    } else {
        s.to_string()
    }
}

fn format_duration(dur: chrono::TimeDelta) -> String {
    let secs = dur.num_seconds();
    if secs < 0 {
        return "—".to_string();
    }
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}
