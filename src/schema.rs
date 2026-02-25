use schemars::schema_for;

use crate::model::Workflow;

/// Generate and print the JSON Schema for `Workflow`.
pub fn get_schema_json() -> String {
    let schema = schema_for!(Workflow);
    serde_json::to_string_pretty(&schema).expect("fail")
}

#[cfg(feature = "full")]
pub fn run() -> anyhow::Result<()> {
    println!("{}", get_schema_json());
    Ok(())
}
