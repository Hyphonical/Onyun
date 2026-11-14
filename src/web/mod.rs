//! Axum HTTP/SSE server and optional Tauri desktop launcher.

mod assets;
mod auth;
#[cfg(feature = "desktop")]
mod desktop;
mod events;
mod routes;
mod state;

use crate::cli::args::UiArgs;
use crate::cli::render;
use crate::cli::style::Color;
use crate::web::auth::require_auth;
use crate::web::state::AppState;
use anyhow::{Context, Result};
use axum::{
    Router, middleware,
    routing::{delete, get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

pub(crate) fn run_ui(args: &UiArgs) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start async runtime")?;

    let state = AppState::new(args.out_dir.clone(), args.engine, args.flags);
    let bind_addr: SocketAddr = format!("{}:{}", args.bind, args.port)
        .parse()
        .with_context(|| format!("invalid bind address '{}:{}'", args.bind, args.port))?;

    let (listener, actual_port) = rt.block_on(async {
        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind UI server on {bind_addr}"))?;
        let port = listener
            .local_addr()
            .context("failed to read local bind address")?
            .port();
        Ok::<_, anyhow::Error>((listener, port))
    })?;

    let router = build_router(Arc::clone(&state));
    let use_desktop = should_launch_desktop(args);

    let flags = args.flags;
    render::blank(flags);
    render::rule(flags, &format!("Onyun UI (v{})", env!("CARGO_PKG_VERSION")));
    render::row(
        flags,
        "ui",
        "Mode",
        if use_desktop {
            "Desktop (Tauri)"
        } else {
            "Web (Axum)"
        },
        Color::Heading,
    );
    let ui_engine = crate::core::engine::select_engine(args.engine);
    render::row(
        flags,
        "ui",
        "Engine",
        ui_engine
            .as_ref()
            .map(|e| e.label())
            .unwrap_or(crate::core::engine::cpu::LABEL),
        Color::Text,
    );
    if let Ok(engine) = &ui_engine {
        render::row(flags, "ui", "Device", &engine.device_name(), Color::Text);
    }
    render::row(
        flags,
        "ui",
        "URL",
        &format!("http://{}:{actual_port}", display_host(&args.bind)),
        Color::Accent,
    );
    render::row(
        flags,
        "ui",
        "Output",
        &args.out_dir.display().to_string(),
        Color::Dim,
    );
    render::row(
        flags,
        "ui",
        "Auth",
        if state.auth.is_some() {
            "enabled (ONYUN_USER/ONYUN_PASS)"
        } else {
            "disabled"
        },
        Color::Dim,
    );
    render::blank(flags);

    if use_desktop {
        #[cfg(feature = "desktop")]
        {
            let server_state = Arc::clone(&state);
            let _server = rt.spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            let res = desktop::launch(actual_port, &server_state);
            state.stop_mint();
            return res;
        }
    }

    let shutdown_state = Arc::clone(&state);
    rt.block_on(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown_state.stop_mint();
            })
            .await
            .context("HTTP server error")
    })
}

fn build_router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/events", get(events::sse_events))
        .route("/api/mint/start", post(routes::post_mint_start))
        .route("/api/mint/stop", post(routes::post_mint_stop))
        .route("/api/mints", get(routes::get_mints))
        .route("/api/mints/{target}", delete(routes::delete_mint))
        .route("/api/mints/{target}/export", get(routes::export_mint))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_auth,
        ));

    Router::new()
        .route("/api/status", get(routes::get_status))
        .route("/api/auth/login", post(routes::post_login))
        .route("/api/auth/logout", post(routes::post_logout))
        .merge(protected)
        .fallback(assets::spa_or_asset)
        .with_state(state)
}

fn should_launch_desktop(args: &UiArgs) -> bool {
    if !cfg!(feature = "desktop") || args.web {
        return false;
    }
    matches!(args.bind.as_str(), "127.0.0.1" | "localhost" | "::1")
}

fn display_host(bind: &str) -> &str {
    if bind == "0.0.0.0" || bind == "::" {
        "127.0.0.1"
    } else {
        bind
    }
}
