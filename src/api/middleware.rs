use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::error::ApiError;
use super::state::AppState;

pub struct AuthUser {
    pub user_id: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::Unauthorized("missing authorization header".into()))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| ApiError::Unauthorized("invalid authorization format".into()))?;

        let inner = state.inner.read().await;
        let claims = super::auth::verify_jwt(token, &inner.auth_secret)
            .map_err(|_| ApiError::Unauthorized("invalid or expired token".into()))?;

        Ok(AuthUser {
            user_id: claims.sub,
        })
    }
}
