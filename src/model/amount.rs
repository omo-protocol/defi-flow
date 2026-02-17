use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Amount of tokens to transfer along an edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Amount {
    /// A fixed token amount, expressed as a decimal string (e.g. "1000.50").
    Fixed {
        /// Decimal string representation of the amount.
        value: String,
    },
    /// A percentage of the available balance (0.0 - 100.0).
    Percentage {
        /// Percentage value between 0 and 100.
        value: f64,
    },
    /// Transfer all available tokens.
    All,
}
