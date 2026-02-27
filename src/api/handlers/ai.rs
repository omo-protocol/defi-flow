use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::api::error::ApiError;
use crate::api::middleware::AuthUser;
use crate::api::state::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<Value>,
    #[serde(default)]
    pub tools: Option<Vec<Value>>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_temperature() -> f32 {
    0.3
}

pub async fn chat_proxy(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Response, ApiError> {
    let inner = state.inner.read().await;

    // Rate limit check
    if let Err(retry_after) = inner.rate_limiter.check(&auth.user_id).await {
        return Err(ApiError::RateLimited(format!(
            "Rate limit exceeded. Try again in {retry_after}s"
        )));
    }

    let ai_api_key = &inner.ai_api_key;
    let ai_base_url = &inner.ai_base_url;
    let ai_model = &inner.ai_model;

    if ai_api_key.is_empty() {
        return Err(ApiError::Internal("AI provider not configured".into()));
    }

    // Build request body for the provider
    let mut body = serde_json::json!({
        "model": ai_model,
        "messages": req.messages,
        "stream": true,
        "temperature": req.temperature,
    });

    if let Some(tools) = &req.tools {
        body["tools"] = serde_json::json!(tools);
    }

    // Forward to AI provider
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", ai_base_url.trim_end_matches('/'));

    let upstream = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {ai_api_key}"))
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("AI provider request failed: {e}")))?;

    if !upstream.status().is_success() {
        let status = upstream.status().as_u16();
        let text = upstream
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".into());
        return Err(ApiError::Internal(format!(
            "AI provider returned {status}: {text}"
        )));
    }

    // Stream the response back as-is (SSE passthrough)
    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("text/event-stream"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));

    Ok((StatusCode::OK, headers, body).into_response())
}
