mod manifest;
mod plan;
pub mod source;

use std::path::Path;

use anyhow::{Context, Result};

use plan::DataSource;
use source::FetchResult;

/// Run the fetch-data command: read workflow, fetch data from APIs, write CSVs + manifest.
pub fn run(workflow_path: &Path, output_dir: &Path, days: u32, interval: &str) -> Result<()> {
    // 1. Load and validate workflow
    let workflow = crate::validate::load_and_validate(workflow_path).map_err(|errors| {
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        anyhow::anyhow!("Workflow validation failed:\n  {}", msgs.join("\n  "))
    })?;

    // 2. Build fetch plan
    let jobs = plan::build_plan(&workflow);
    if jobs.is_empty() {
        println!("No nodes require external data. Nothing to fetch.");
        return Ok(());
    }

    let total_nodes: usize = jobs.iter().map(|j| j.node_ids.len()).sum();
    println!(
        "Fetch plan: {} data sources covering {} nodes",
        jobs.len(),
        total_nodes
    );
    for job in &jobs {
        println!(
            "  {} {} → {} (nodes: {})",
            job.source.name(),
            job.key,
            job.filename,
            job.node_ids.join(", ")
        );
    }
    println!();

    // 3. Compute time range
    let end_ms = chrono::Utc::now().timestamp_millis() as u64;
    let start_ms = end_ms - (days as u64) * 86_400_000;
    let config = source::FetchConfig {
        start_time_ms: start_ms,
        end_time_ms: end_ms,
        interval: interval.to_string(),
    };

    // 4. Create output directory
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("creating output directory {}", output_dir.display()))?;

    // 5. Build tokio runtime and execute fetches
    let rt = tokio::runtime::Runtime::new().context("creating async runtime")?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("defi-flow/0.1")
            .build()
            .context("creating HTTP client")?;

        let mut manifest_entries: Vec<(String, String, String)> = Vec::new();
        let mut success_count = 0u32;
        let mut fail_count = 0u32;

        for (i, job) in jobs.iter().enumerate() {
            println!(
                "[{}/{}] Fetching {} {} ...",
                i + 1,
                jobs.len(),
                job.source.name(),
                job.key
            );

            let result = fetch_job(&client, job, &config).await;

            match result {
                Ok(data) => {
                    let row_count = data.row_count();
                    manifest::write_fetch_result(output_dir, &job.filename, &data)?;
                    for node_id in &job.node_ids {
                        manifest_entries.push((
                            node_id.clone(),
                            job.filename.clone(),
                            job.kind.clone(),
                        ));
                    }
                    println!(
                        "  OK  {} → {} ({} rows)",
                        job.key, job.filename, row_count
                    );
                    success_count += 1;
                }
                Err(e) => {
                    println!(
                        "  WARN  {} failed: {:#}. Skipping nodes: {}",
                        job.key,
                        e,
                        job.node_ids.join(", ")
                    );
                    fail_count += 1;
                }
            }
        }

        // 6. Write manifest
        if !manifest_entries.is_empty() {
            manifest::write_manifest(output_dir, &manifest_entries)?;
        }

        println!(
            "\nDone: {} succeeded, {} failed. Wrote manifest.json with {} entries to {}",
            success_count,
            fail_count,
            manifest_entries.len(),
            output_dir.display()
        );

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

/// Dispatch a single fetch job to the appropriate data source.
async fn fetch_job(
    client: &reqwest::Client,
    job: &plan::FetchJob,
    config: &source::FetchConfig,
) -> Result<FetchResult> {
    match &job.source {
        DataSource::HyperliquidPerp => source::hyperliquid::fetch_perp(client, &job.key, config)
            .await
            .map(FetchResult::Perp),

        DataSource::HyperliquidSpot => source::hyperliquid::fetch_spot(client, &job.key, config)
            .await
            .map(FetchResult::Price),

        DataSource::Rysk => {
            // Fetch Hyperliquid spot prices as oracle for Rysk's synthetic history
            let oracle_prices = source::hyperliquid::fetch_spot(client, &job.key, config)
                .await
                .ok();
            source::rysk::fetch_options(
                client,
                &job.key,
                config,
                oracle_prices.as_deref(),
            )
            .await
            .map(FetchResult::Options)
        }

        DataSource::AerodromeSubgraph => source::aerodrome::fetch_lp(client, &job.key, config)
            .await
            .map(FetchResult::Lp),

        DataSource::DefiLlamaYield => {
            if job.kind == "pendle" {
                // Strip "pendle:" prefix from key
                let market = job.key.strip_prefix("pendle:").unwrap_or(&job.key);
                source::defillama::fetch_pendle(client, market, config)
                    .await
                    .map(FetchResult::Pendle)
            } else {
                // Parse "venue_slug:asset" from key
                let (venue, asset) = job
                    .key
                    .split_once(':')
                    .unwrap_or(("unknown", &job.key));
                source::defillama::fetch_lending(client, venue, asset, config)
                    .await
                    .map(FetchResult::Lending)
            }
        }
    }
}
