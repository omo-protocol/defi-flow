use schemars::schema_for;

use crate::model::Workflow;

/// Generate and print the JSON Schema for `Workflow`.
pub fn run() -> anyhow::Result<()> {
    let schema = schema_for!(Workflow);
    let json = serde_json::to_string_pretty(&schema)?;
    println!("{json}");
    Ok(())
}
