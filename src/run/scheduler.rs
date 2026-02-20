use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::node::{CronInterval, Node, NodeId, Trigger};
use crate::model::workflow::Workflow;

/// Manages cron triggers for the live execution loop.
pub struct CronScheduler {
    /// (node_id, interval_duration) for each triggered node.
    triggers: Vec<(NodeId, Duration)>,
    /// When each node was last fired.
    last_fired: HashMap<NodeId, Instant>,
}

impl CronScheduler {
    pub fn new(workflow: &Workflow) -> Self {
        let triggers: Vec<(NodeId, Duration)> = workflow
            .nodes
            .iter()
            .filter_map(|node| {
                let trigger = get_trigger(node)?;
                if let Trigger::Cron { interval } = trigger {
                    let duration = cron_to_duration(interval);
                    Some((node.id().to_string(), duration))
                } else {
                    None
                }
            })
            .collect();

        CronScheduler {
            triggers,
            last_fired: HashMap::new(),
        }
    }

    /// Returns true if there are any triggered nodes.
    pub fn has_triggers(&self) -> bool {
        !self.triggers.is_empty()
    }

    /// Sleep until the next trigger fires, return the node IDs to execute.
    pub async fn wait_for_next(&mut self) -> Vec<NodeId> {
        if self.triggers.is_empty() {
            // No triggers — sleep forever (shouldn't happen in practice)
            tokio::time::sleep(Duration::from_secs(86400)).await;
            return Vec::new();
        }

        // Find the shortest time until any trigger fires
        let now = Instant::now();
        let mut min_wait = Duration::from_secs(86400);

        for (node_id, interval) in &self.triggers {
            let last = self.last_fired.get(node_id).copied().unwrap_or(now);
            let elapsed = now.duration_since(last);
            if elapsed >= *interval {
                // Already due — fire immediately
                min_wait = Duration::ZERO;
                break;
            }
            let remaining = *interval - elapsed;
            if remaining < min_wait {
                min_wait = remaining;
            }
        }

        if !min_wait.is_zero() {
            tokio::time::sleep(min_wait).await;
        }

        // Collect all nodes that are due to fire
        let now = Instant::now();
        let mut fired = Vec::new();

        for (node_id, interval) in &self.triggers {
            let last = self.last_fired.get(node_id).copied().unwrap_or_else(|| {
                // First run: pretend we fired at (now - interval) so it fires immediately
                now - *interval
            });
            let elapsed = now.duration_since(last);
            if elapsed >= *interval {
                fired.push(node_id.clone());
                self.last_fired.insert(node_id.clone(), now);
            }
        }

        fired
    }

    /// Get all currently due triggers without waiting (for --once mode).
    pub fn get_all_due(&mut self) -> Vec<NodeId> {
        let now = Instant::now();
        self.triggers
            .iter()
            .map(|(node_id, _)| {
                self.last_fired.insert(node_id.clone(), now);
                node_id.clone()
            })
            .collect()
    }
}

fn cron_to_duration(interval: &CronInterval) -> Duration {
    match interval {
        CronInterval::Hourly => Duration::from_secs(3600),
        CronInterval::Daily => Duration::from_secs(86400),
        CronInterval::Weekly => Duration::from_secs(604800),
        CronInterval::Monthly => Duration::from_secs(2592000),
    }
}

fn get_trigger(node: &Node) -> Option<&Trigger> {
    match node {
        Node::Perp { trigger, .. }
        | Node::Options { trigger, .. }
        | Node::Spot { trigger, .. }
        | Node::Lp { trigger, .. }
        | Node::Swap { trigger, .. }
        | Node::Bridge { trigger, .. }
        | Node::Lending { trigger, .. }
        | Node::Vault { trigger, .. }
        | Node::Pendle { trigger, .. }
        | Node::Optimizer { trigger, .. } => trigger.as_ref(),
        Node::Wallet { .. } => None,
    }
}
