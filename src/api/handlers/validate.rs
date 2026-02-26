use axum::Json;
use axum::extract::State;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{ValidateRequest, ValidateResponse};
use crate::validate;

pub async fn validate_workflow(
    State(_state): State<AppState>,
    Json(req): Json<ValidateRequest>,
) -> Result<Json<ValidateResponse>, ApiError> {
    // Offline validation
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if let Err(errs) = validate::validate(&req.workflow) {
        for e in &errs {
            errors.push(e.to_string());
        }
    }

    // Optional on-chain validation
    if req.check_onchain && errors.is_empty() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let onchain_errors = validate::onchain::validate_onchain(&req.workflow).await;
        for e in onchain_errors {
            if matches!(
                e,
                validate::ValidationError::RpcUnreachable { .. }
                    | validate::ValidationError::MovementNoRoute { .. }
            ) {
                warnings.push(e.to_string());
            } else {
                errors.push(e.to_string());
            }
        }
    }

    Ok(Json(ValidateResponse {
        valid: errors.is_empty(),
        errors,
        warnings,
    }))
}
