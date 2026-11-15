//! Prometheus exposition endpoint for Grafana scraping.

use crate::core::engine::MintSnapshot;
use crate::web::state::AppState;
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub(crate) async fn get_metrics(State(state): State<Arc<AppState>>) -> Response {
    let snap = state.current_snapshot();
    let body = render(&snap);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

fn render(snap: &MintSnapshot) -> String {
    let mut out = String::with_capacity(1024);
    let version = env!("CARGO_PKG_VERSION");
    out.push_str("# HELP onyun_build_info Build information.\n");
    out.push_str("# TYPE onyun_build_info gauge\n");
    out.push_str(&format!("onyun_build_info{{version=\"{version}\"}} 1\n"));

    out.push_str("# HELP onyun_mint_active Whether a mint job is active (1) or idle (0).\n");
    out.push_str("# TYPE onyun_mint_active gauge\n");
    out.push_str(&format!("onyun_mint_active {}\n", bool_to_int(snap.active)));

    out.push_str("# HELP onyun_mint_completed Whether the current job completed (1) or not (0).\n");
    out.push_str("# TYPE onyun_mint_completed gauge\n");
    out.push_str(&format!(
        "onyun_mint_completed {}\n",
        bool_to_int(snap.completed)
    ));

    out.push_str("# HELP onyun_hashes_total Hashes attempted by the current job.\n");
    out.push_str("# TYPE onyun_hashes_total counter\n");
    out.push_str(&format!("onyun_hashes_total {}\n", snap.total_hashes));

    out.push_str("# HELP onyun_hashes_per_second Current hashing rate.\n");
    out.push_str("# TYPE onyun_hashes_per_second gauge\n");
    out.push_str(&format!(
        "onyun_hashes_per_second {}\n",
        snap.hashes_per_sec
    ));

    out.push_str("# HELP onyun_best_match_len Best address match length so far.\n");
    out.push_str("# TYPE onyun_best_match_len gauge\n");
    out.push_str(&format!("onyun_best_match_len {}\n", snap.best_len));

    out.push_str("# HELP onyun_target_len Requested address match length.\n");
    out.push_str("# TYPE onyun_target_len gauge\n");
    out.push_str(&format!("onyun_target_len {}\n", snap.target_len));

    out.push_str("# HELP onyun_hashes_since_best Hashes attempted since the best match.\n");
    out.push_str("# TYPE onyun_hashes_since_best gauge\n");
    out.push_str(&format!(
        "onyun_hashes_since_best {}\n",
        snap.hashes_since_best
    ));

    out.push_str("# HELP onyun_progress_ratio Intra-step progress in [0.0, 1.0].\n");
    out.push_str("# TYPE onyun_progress_ratio gauge\n");
    out.push_str(&format!("onyun_progress_ratio {}\n", snap.progress));

    out.push_str("# HELP onyun_pace_ratio Pace ratio relative to expected 32^(best_len + 1).\n");
    out.push_str("# TYPE onyun_pace_ratio gauge\n");
    out.push_str(&format!("onyun_pace_ratio {}\n", snap.pace_ratio));

    out.push_str("# HELP onyun_elapsed_seconds Elapsed time of the current job in seconds.\n");
    out.push_str("# TYPE onyun_elapsed_seconds gauge\n");
    out.push_str(&format!("onyun_elapsed_seconds {}\n", snap.elapsed_secs));

    if let Some(eta) = snap.eta_secs {
        out.push_str("# HELP onyun_eta_seconds Estimated seconds remaining.\n");
        out.push_str("# TYPE onyun_eta_seconds gauge\n");
        out.push_str(&format!("onyun_eta_seconds {eta}\n"));
    }

    if !snap.target.is_empty() {
        out.push_str("# HELP onyun_mint_info Current mint target.\n");
        out.push_str("# TYPE onyun_mint_info gauge\n");
        let target = snap.target.replace(['\\', '"', '\n'], "");
        out.push_str(&format!("onyun_mint_info{{target=\"{target}\"}} 1\n"));
    }

    out
}

fn bool_to_int(v: bool) -> u8 {
    u8::from(v)
}
