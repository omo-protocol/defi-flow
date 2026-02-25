use std::collections::HashMap;

use anyhow::{bail, Result};

use crate::model::node::{Node, NodeId, VenueAllocation};
use crate::venues::RiskParams;

/// A single allocation group (may contain one or more co-hedged targets).
pub struct AllocationGroup {
    /// Target node IDs in this group (e.g. ["buy_eth", "short_eth"]).
    pub targets: Vec<NodeId>,
    /// Fraction of total portfolio for the entire group.
    pub fraction: f64,
}

/// Result of a Kelly allocation computation.
pub struct AllocationResult {
    /// Group-level allocations (preserves hedge structure).
    pub groups: Vec<AllocationGroup>,
}

/// Resolved stats for one allocation (static from JSON or derived from venue data).
struct ResolvedStats {
    expected_return: f64,
    volatility: f64,
}

/// Resolved risk for one allocation (combined from group members).
struct ResolvedRisk {
    p_loss: f64,
    loss_severity: f64,
    rebalance_cost: f64,
}

impl Default for ResolvedRisk {
    fn default() -> Self {
        Self {
            p_loss: 0.0,
            loss_severity: 0.0,
            rebalance_cost: 0.0,
        }
    }
}

/// Compute fractional Kelly allocations for an optimizer node.
///
/// Uses smooth Kelly: maximizes E[log(1 + f*R)] with integrated liquidation
/// probability and transaction costs. Falls back to classic Kelly when
/// risk params are unavailable (p_loss=0, cost=0).
///
/// `venue_stats` maps node_id → (annualized_alpha_return, annualized_alpha_vol)
/// `venue_risks` maps node_id → RiskParams from each venue's `risk_params()`
pub fn compute_kelly_allocations(
    node: &Node,
    _total_capital: f64,
    venue_stats: &HashMap<NodeId, (f64, f64)>,
    venue_risks: &HashMap<NodeId, RiskParams>,
) -> Result<AllocationResult> {
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
            groups: vec![],
        });
    }

    // Resolve stats and risk for each allocation
    let resolved: Vec<ResolvedStats> = allocations
        .iter()
        .map(|a| resolve_stats(a, venue_stats))
        .collect();

    let risks: Vec<ResolvedRisk> = allocations
        .iter()
        .map(|a| resolve_risk(a, venue_risks))
        .collect();

    // Log resolved stats for visibility
    for (i, (alloc, stats)) in allocations.iter().zip(resolved.iter()).enumerate() {
        let targets = alloc.targets();
        let label = targets.join("+");
        let source = if alloc.expected_return.is_some() && alloc.volatility.is_some() {
            "static"
        } else {
            "adaptive"
        };
        let risk = &risks[i];
        let risk_str = if risk.p_loss > 0.0 {
            format!(
                " p_loss={:.3}% sev={:.0}% cost={:.2}%",
                risk.p_loss * 100.0,
                risk.loss_severity * 100.0,
                risk.rebalance_cost * 100.0,
            )
        } else {
            String::new()
        };
        eprintln!(
            "  [kelly] {label}: return={:.2}%, vol={:.2}% ({source}){risk_str}",
            stats.expected_return * 100.0,
            stats.volatility * 100.0,
        );
    }

    let raw_kellys: Vec<f64> = resolved
        .iter()
        .zip(risks.iter())
        .map(|(s, r)| smooth_kelly(s.expected_return, s.volatility, kelly_fraction, r))
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

    // Log final allocation fractions
    for (alloc, &frac) in allocations.iter().zip(fractions.iter()) {
        let targets = alloc.targets();
        let label = targets.join("+");
        eprintln!("  [kelly] {label} → {:.1}%", frac * 100.0);
    }

    // Build group-level allocations (preserve hedge structure)
    let mut groups = Vec::new();
    for (alloc, &fraction) in allocations.iter().zip(fractions.iter()) {
        let targets = alloc.targets().iter().map(|s| s.to_string()).collect();
        groups.push(AllocationGroup { targets, fraction });
    }

    Ok(AllocationResult { groups })
}

/// Resolve expected_return and volatility for an allocation.
///
/// If static params are in the JSON, use them. Otherwise, derive from venue data:
/// - Single target: use that venue's alpha_stats directly.
/// - Group targets: sum alpha returns, sqrt-sum alpha vols.
///   For delta-neutral (spot+perp): spot alpha = (0,0), so group = perp funding stats.
fn resolve_stats(
    alloc: &VenueAllocation,
    venue_stats: &HashMap<NodeId, (f64, f64)>,
) -> ResolvedStats {
    // If both static params are specified, use them
    if let (Some(er), Some(vol)) = (alloc.expected_return, alloc.volatility) {
        return ResolvedStats {
            expected_return: er,
            volatility: vol,
        };
    }

    // Derive from venue alpha_stats
    let targets = alloc.targets();
    let mut total_return = 0.0;
    let mut total_var = 0.0;
    let mut found_any = false;

    for target in &targets {
        if let Some(&(ret, vol)) = venue_stats.get(*target) {
            total_return += ret;
            total_var += vol * vol; // sum of variances (uncorrelated yields)
            found_any = true;
        }
    }

    if !found_any {
        // No venue data available — use conservative defaults
        return ResolvedStats {
            expected_return: 0.0,
            volatility: 1.0, // high vol → Kelly allocates nothing
        };
    }

    // Allow partial override: if only one param is specified, use it
    let expected_return = alloc.expected_return.unwrap_or(total_return);
    let volatility = alloc.volatility.unwrap_or_else(|| total_var.sqrt().max(1e-6));

    ResolvedStats {
        expected_return,
        volatility,
    }
}

/// Resolve risk parameters for an allocation group.
///
/// For single targets: use venue risk directly.
/// For groups (delta-neutral): hedge legs offset the risky legs' losses.
/// When spot + short perp are grouped, the spot gain offsets the perp loss
/// on liquidation, leaving only unwind costs (~2%) as net severity.
fn resolve_risk(
    alloc: &VenueAllocation,
    venue_risks: &HashMap<NodeId, RiskParams>,
) -> ResolvedRisk {
    let targets = alloc.targets();
    let mut risk = ResolvedRisk::default();
    let mut n_risky = 0u32;

    for target in &targets {
        if let Some(r) = venue_risks.get(*target) {
            risk.p_loss = risk.p_loss.max(r.p_loss);
            risk.loss_severity = risk.loss_severity.max(r.loss_severity);
            risk.rebalance_cost += r.rebalance_cost;
            n_risky += 1;
        }
    }

    // For groups with hedge legs (e.g. spot in delta-neutral), the hedge
    // offsets the risky leg's catastrophic loss. Net severity = residual
    // unwind cost, not the gross single-leg loss.
    let n_total = targets.len() as u32;
    if n_total > 1 && n_risky > 0 {
        let n_hedge = n_total - n_risky;
        if n_hedge > 0 {
            // Hedged group: gains on hedge legs offset losses on risky legs.
            // Net severity ≈ risky_fraction - hedge_fraction (for equal weight).
            // For 50/50 delta-neutral (1 risky + 1 hedge): 0.5 - 0.5 = 0.
            // Floor at unwind costs (liquidation fee + slippage ≈ 2%).
            let risky_frac = n_risky as f64 / n_total as f64;
            let hedge_frac = n_hedge as f64 / n_total as f64;
            let net = risk.loss_severity * (risky_frac - hedge_frac);
            risk.loss_severity = net.max(0.02);
        }
        // If all legs are risky (no hedges), keep max severity unchanged.
    }

    risk
}

/// Smooth Kelly: maximize E[log(1 + f*R)] with integrated risk.
///
/// E[log(1+fR)] = (1-p_loss) * ln(1 + f*(return-cost)) + p_loss * ln(1 - f*severity)
///
/// When p_loss=0 and cost=0, this reduces to classic f* = return/vol^2.
/// Uses grid search (200 points) + golden-section refinement.
///
/// The search range is [0, 1/kelly_fraction] so that after applying
/// kelly_fraction, the result can reach up to 1.0 (full allocation).
/// This ensures half-Kelly of a strategy that wants 100%+ still gives 100%.
fn smooth_kelly(
    expected_return: f64,
    volatility: f64,
    kelly_fraction: f64,
    risk: &ResolvedRisk,
) -> f64 {
    if volatility <= 0.0 || expected_return <= 0.0 || kelly_fraction <= 0.0 {
        return 0.0;
    }

    // Fast path: if no risk, use classic Kelly
    if risk.p_loss <= 0.0 && risk.rebalance_cost <= 0.0 {
        let raw = expected_return / (volatility * volatility);
        return (raw * kelly_fraction).max(0.0);
    }

    let net_return = (expected_return - risk.rebalance_cost).max(0.0);
    if net_return <= 0.0 {
        return 0.0;
    }

    let p = risk.p_loss.min(1.0).max(0.0);
    let s = risk.loss_severity.min(0.999).max(0.0);

    // Search range: up to 1/kelly_fraction so result can reach 1.0 after scaling.
    // Also bounded by 1/severity to avoid log(0).
    let max_f = if s > 0.0 {
        (1.0 / kelly_fraction).min(0.99 / s)
    } else {
        1.0 / kelly_fraction
    };

    // Objective: E[log(1 + f*R)] with risk
    // Quadratic approximation for the "good" outcome:
    // ln(1 + f*net_return) ≈ f*net_return - 0.5*f^2*vol^2
    // Combined: (1-p)*(f*net_return - 0.5*f^2*vol^2) + p*ln(1 - f*s)
    let objective = |f: f64| -> f64 {
        if f <= 0.0 {
            return 0.0;
        }
        if f * s >= 1.0 {
            return f64::NEG_INFINITY;
        }

        let good = f * net_return - 0.5 * f * f * volatility * volatility;
        let bad = (1.0 - f * s).ln();
        (1.0 - p) * good + p * bad
    };

    // Grid search over f in (0, max_f] with 200 points
    let n_grid = 200;
    let mut best_f = 0.0;
    let mut best_val = 0.0_f64;

    for i in 1..=n_grid {
        let f = max_f * i as f64 / n_grid as f64;
        let val = objective(f);
        if val > best_val {
            best_val = val;
            best_f = f;
        }
    }

    // Golden-section refinement around best grid point
    let step = max_f / n_grid as f64;
    let mut lo = (best_f - step).max(0.001);
    let mut hi = (best_f + step).min(max_f);
    let golden = 0.381966011250105; // (3 - sqrt(5)) / 2

    for _ in 0..50 {
        let m1 = lo + golden * (hi - lo);
        let m2 = hi - golden * (hi - lo);
        if objective(m1) < objective(m2) {
            lo = m1;
        } else {
            hi = m2;
        }
        if (hi - lo).abs() < 1e-8 {
            break;
        }
    }

    let optimal_f = (lo + hi) / 2.0;

    // Scale by kelly_fraction (half-Kelly, quarter-Kelly, etc.)
    (optimal_f * kelly_fraction).max(0.0)
}

