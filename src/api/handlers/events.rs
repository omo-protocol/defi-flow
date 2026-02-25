use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use tokio::sync::broadcast;
use tokio_stream::Stream;

use crate::api::error::ApiError;
use crate::api::state::AppState;

pub async fn event_stream(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let state_inner = state.inner.read().await;
    let session = state_inner
        .sessions
        .get(&session_id)
        .ok_or_else(|| ApiError::NotFound(format!("session '{session_id}' not found")))?;

    let mut rx = session.event_tx.subscribe();
    drop(state_inner);

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().data(json));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let msg = format!("{{\"type\":\"Lagged\",\"missed\":{n}}}");
                    yield Ok(Event::default().data(msg));
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream))
}
