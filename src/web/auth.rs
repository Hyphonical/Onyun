//! Optional `ONYUN_USER` / `ONYUN_PASS` cookie authentication middleware.

use crate::web::state::AppState;
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;

pub(crate) const COOKIE_NAME: &str = "onyun_session";

#[derive(Serialize)]
struct AuthError {
    error: &'static str,
}

pub(crate) fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=')
            && k.trim() == name
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub(crate) fn is_authenticated(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(auth) = &state.auth else {
        return true;
    };
    extract_cookie(headers, COOKIE_NAME)
        .is_some_and(|token| auth.verify_token(&token))
}

pub(crate) async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if is_authenticated(&state, request.headers()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(AuthError {
                error: "authentication required",
            }),
        )
            .into_response()
    }
}
