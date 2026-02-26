use crate::model::Workflow;
use crate::validate;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn validate_workflow_json(json: &str) -> String {
    let workflow: Workflow = match serde_json::from_str(json) {
        Ok(w) => w,
        Err(e) => {
            return serde_json::json!({
                "valid": false,
                "errors": [format!("JSON parse error: {}", e)]
            })
            .to_string();
        }
    };
    match validate::validate(&workflow) {
        Ok(()) => serde_json::json!({ "valid": true }).to_string(),
        Err(errors) => {
            let error_strings: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            serde_json::json!({
                "valid": false,
                "errors": error_strings
            })
            .to_string()
        }
    }
}

#[wasm_bindgen]
pub fn parse_workflow_json(json: &str) -> String {
    let workflow: Workflow = match serde_json::from_str(json) {
        Ok(w) => w,
        Err(e) => {
            return serde_json::json!({
                "error": format!("JSON parse error: {}", e)
            })
            .to_string();
        }
    };
    serde_json::to_string_pretty(&workflow).unwrap_or_else(|e| {
        serde_json::json!({
            "error": format!("Serialization error: {}", e)
        })
        .to_string()
    })
}

#[wasm_bindgen]
pub fn get_schema() -> String {
    crate::schema::get_schema_json()
}
