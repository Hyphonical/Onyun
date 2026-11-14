//! REST API handlers for `/api/status`, `/api/mint/*`, `/api/mints`, and `/api/auth/*`.

use crate::web::auth::{COOKIE_NAME, is_authenticated};
use crate::web::state::{AppState, SavedMintEntry, StatusPayload};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
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
    engine: Option<String>,
    devices: Option<Vec<String>>,
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
    let selected_engine = crate::core::engine::select_engine(state.default_engine);
    Json(StatusPayload {
        version: env!("CARGO_PKG_VERSION"),
        engine: selected_engine
            .as_ref()
            .map(|e| e.label())
            .unwrap_or(crate::core::engine::cpu::LABEL),
        engine_kind: state.default_engine.as_str(),
        device_name: selected_engine
            .as_ref()
            .map(|e| e.device_name())
            .unwrap_or_else(|_| crate::core::engine::cpu::LABEL.to_string()),
        devices: state.devices.clone(),
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
    match state.start_mint(
        &body.target,
        body.threads,
        body.engine.as_deref(),
        body.devices.as_deref(),
    ) {
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

pub(crate) async fn export_mint(
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
    let dir = crate::core::storage::target_dir(&state.out_dir, &normalized);
    if !dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "mint directory not found".into(),
            }),
        )
            .into_response();
    }

    let mut archive = Vec::new();
    let mut any_file = false;
    for name in ["hostname", "hs_ed25519_public_key", "hs_ed25519_secret_key"] {
        let path = dir.join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        any_file = true;
        let entry_name = format!("{normalized}/{name}");
        let mode = if name == "hostname" { 0o644 } else { 0o600 };
        append_tar_entry(&mut archive, &entry_name, mode, &bytes);
    }

    if !any_file {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "no keypair files found in mint directory".into(),
            }),
        )
            .into_response();
    }

    // Standard ustar EOF: two 512-byte zero blocks.
    archive.resize(archive.len() + 1024, 0);

    let disposition = format!("attachment; filename=\"{normalized}.tar\"");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/x-tar".to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        archive,
    )
        .into_response()
}

fn append_tar_entry(out: &mut Vec<u8>, name: &str, mode: u32, data: &[u8]) {
    let mut header = [0u8; 512];
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(100);
    header[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    write_octal(&mut header[100..108], mode as u64);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], data.len() as u64);
    write_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");

    let checksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    let chk = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(chk.as_bytes());

    out.extend_from_slice(&header);
    out.extend_from_slice(data);
    let rem = data.len() % 512;
    if rem != 0 {
        out.resize(out.len() + (512 - rem), 0);
    }
}

fn write_octal(buf: &mut [u8], val: u64) {
    let width = buf.len() - 1;
    let formatted = format!("{val:0width$o}", width = width);
    let bytes = formatted.as_bytes();
    let start = bytes.len().saturating_sub(width);
    buf[..width].copy_from_slice(&bytes[start..]);
    buf[width] = 0;
}
