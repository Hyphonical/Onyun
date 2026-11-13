//! Server-Sent Events (`/api/events`) streaming `MintSnapshot` updates.

use crate::web::state::AppState;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

pub(crate) async fn sse_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.events_tx.subscribe();
    let initial = state.current_snapshot();

    let stream = async_stream_channel(&initial, move |tx| async move {
        loop {
            match rx.recv().await {
                Ok(snap) => {
                    if let Ok(json) = serde_json::to_string(&snap)
                        && tx
                            .send(Ok(Event::default().event("snapshot").data(json)))
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

fn async_stream_channel<F, Fut>(
    initial: &crate::core::engine::MintSnapshot,
    spawn_fn: F,
) -> impl Stream<Item = Result<Event, Infallible>> + use<F, Fut>
where
    F: FnOnce(tokio::sync::mpsc::Sender<Result<Event, Infallible>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    if let Ok(json) = serde_json::to_string(initial) {
        let _ = tx.try_send(Ok(Event::default().event("snapshot").data(json)));
    }
    tokio::spawn(spawn_fn(tx));
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))
}
