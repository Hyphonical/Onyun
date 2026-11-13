//! REST API handlers for `/api/status`, `/api/mint/*`, `/api/mints`, and `/api/auth/*`.

use crate::web::auth::{is_authenticated, COOKIE_NAME};
use crate::web::state::{AppState, SavedMintEntry, StatusPayload};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Serialize)]
pub(crate) struct OkBody {
    ok: bool,
}

#[derive(Deserialize)]
pub(crate) struct StartMintRequest {
    target: String,
    threads: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    username: String,
    password: String,
}

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<StatusPayload> {
    let authenticated = is_authenticated(&state, &headers);
    let snapshot = if authenticated {
        state.current_snapshot()
    } else {
        crate::core::engine::MintSnapshot {
            active: false,
            completed: false,
            target: String::new(),
            best_address: None,
            best_len: 0,
            target_len: 0,
            total_hashes: 0,
            hashes_since_best: 0,
            hashes_per_sec: 0.0,
            elapsed_secs: 0.0,
            progress: 0.0,
            pace_ratio: 0.0,
            eta_secs: None,
        }
    };
    Json(StatusPayload {
        version: env!("CARGO_PKG_VERSION"),
        engine: crate::core::engine::MintEngine::label(&crate::core::engine::cpu::CpuMkEngine),
        cpu_threads: state.cpu_threads,
        out_dir: state.out_dir.display().to_string(),
        auth_required: state.auth.is_some(),
        authenticated,
        snapshot,
    })
}

pub(crate) async fn post_login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Response {
    let Some(auth) = &state.auth else {
        return Json(OkBody { ok: true }).into_response();
    };
    if !auth.verify_credentials(body.username.trim(), &body.password) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "invalid username or password".into(),
            }),
        )
            .into_response();
    }
    let Some(token) = auth.issue_token() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "failed to issue session token".into(),
            }),
        )
            .into_response();
    };
    let cookie = format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(OkBody { ok: true }),
    )
        .into_response()
}

pub(crate) async fn post_logout() -> Response {
    let cookie = format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(OkBody { ok: true }),
    )
        .into_response()
}

pub(crate) async fn post_mint_start(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartMintRequest>,
) -> Response {
    match state.start_mint(&body.target, body.threads) {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: format!("{err:#}"),
            }),
        )
            .into_response(),
    }
}

pub(crate) async fn post_mint_stop(State(state): State<Arc<AppState>>) -> Json<OkBody> {
    state.stop_mint();
    Json(OkBody { ok: true })
}

pub(crate) async fn get_mints(State(state): State<Arc<AppState>>) -> Json<Vec<SavedMintEntry>> {
    Json(state.list_mints())
}

pub(crate) async fn delete_mint(
    State(state): State<Arc<AppState>>,
    Path(target): Path<String>,
) -> Response {
    let Ok(normalized) = crate::core::base32::normalize_target(&target) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "invalid target name".into(),
            }),
        )
            .into_response();
    };
    let active = state.current_snapshot();
    if active.active && active.target == normalized {
        state.stop_mint();
    }
    let dir = crate::core::storage::target_dir(&state.out_dir, &normalized);
    if !dir.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "mint directory not found".into(),
            }),
        )
            .into_response();
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Json(OkBody { ok: true }).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("failed to remove mint: {err}"),
            }),
        )
            .into_response(),
    }
}
