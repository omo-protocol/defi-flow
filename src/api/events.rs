use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    Deployed {
        nodes: Vec<String>,
        tvl: f64,
    },
    TickCompleted {
        timestamp: u64,
        tvl: f64,
    },
    NodeExecuted {
        node_id: String,
        action: String,
        amount: f64,
    },
    Rebalanced {
        group: String,
        drift: f64,
        adjustments: Vec<(String, f64)>,
    },
    MarginTopUp {
        perp_node: String,
        from_donor: String,
        amount: f64,
        new_ratio: f64,
    },
    ReserveAction {
        action: String,
        amount: f64,
    },
    HotReloaded {
        parameter_changes: Vec<String>,
    },
    Error {
        node_id: Option<String>,
        message: String,
    },
    Stopped {
        reason: String,
    },
}
