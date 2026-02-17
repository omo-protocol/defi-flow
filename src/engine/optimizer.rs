use anyhow::{bail, Result};

use crate::model::node::{Node, NodeId, VenueAllocation};

/// Result of a Kelly allocation computation.
pub struct AllocationResult {
    /// (target_node_id, fraction_of_capital)
    pub allocations: Vec<(NodeId, f64)>,
}

/// Compute fractional Kelly allocations for an optimizer node.
///
/// For each venue: raw_kelly_i = expected_return_i / volatility_i^2
/// Then scaled by kelly_fraction, clamped to max_allocation, and normalized.
pub fn compute_kelly_allocations(node: &Node, _total_capital: f64) -> Result<AllocationResult> {
    let (kelly_fraction, max_allocation, allocations) = match node {
        Node::Optimizer {
            kelly_fraction,
            max_allocation,
            allocations,
            ..
        } => (*kelly_fraction, *max_allocation, allocations),
        _ => bail!("compute_kelly_allocations called on non-optimizer node"),
    };

    if allocations.is_empty() {
        return Ok(AllocationResult {
            allocations: vec![],
        });
    }

    let raw_kellys: Vec<f64> = allocations
        .iter()
        .map(|a| kelly_raw(a, kelly_fraction))
        .collect();

    // Clamp to max_allocation
    let max_alloc = max_allocation.unwrap_or(1.0);
    let clamped: Vec<f64> = raw_kellys.iter().map(|&k| k.min(max_alloc).max(0.0)).collect();

    // Normalize so they sum to at most 1.0
    let total: f64 = clamped.iter().sum();
    let fractions: Vec<f64> = if total > 1.0 {
        clamped.iter().map(|&c| c / total).collect()
    } else {
        clamped
    };

    let result = allocations
        .iter()
        .zip(fractions.iter())
        .map(|(a, &f)| (a.target_node.clone(), f))
        .collect();

    Ok(AllocationResult {
        allocations: result,
    })
}

/// Raw Kelly fraction for a single venue:
/// f* = expected_return / volatility^2, scaled by kelly_fraction.
fn kelly_raw(alloc: &VenueAllocation, kelly_fraction: f64) -> f64 {
    if alloc.volatility <= 0.0 {
        return 0.0;
    }
    let raw = alloc.expected_return / (alloc.volatility * alloc.volatility);
    (raw * kelly_fraction).max(0.0)
}

/// Check if current allocations have drifted past the threshold.
/// Returns true if any venue's actual fraction differs from target by more than `drift_threshold`.
pub fn should_rebalance(
    current_values: &[(NodeId, f64)],
    target_fractions: &[(NodeId, f64)],
    drift_threshold: f64,
) -> bool {
    let total: f64 = current_values.iter().map(|(_, v)| v).sum();
    if total <= 0.0 {
        return false;
    }

    for (target_id, target_frac) in target_fractions {
        let actual_value = current_values
            .iter()
            .find(|(id, _)| id == target_id)
            .map(|(_, v)| *v)
            .unwrap_or(0.0);
        let actual_frac = actual_value / total;
        if (actual_frac - target_frac).abs() > drift_threshold {
            return true;
        }
    }
    false
}
