use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Unauthorized(String),
    Conflict(String),
    Internal(String),
    RateLimited(String),
    Validation(Vec<String>),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, json!({ "error": msg })),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, json!({ "error": msg })),
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, json!({ "error": msg })),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, json!({ "error": msg })),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": msg })),
            ApiError::RateLimited(msg) => (StatusCode::TOO_MANY_REQUESTS, json!({ "error": msg })),
            ApiError::Validation(errors) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "valid": false, "errors": errors }),
            ),
        };

        (status, axum::Json(body)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(format!("{:#}", err))
    }
}
