//! Tauri desktop WebView host (`#[cfg(feature = "desktop")]`).

use crate::web::state::AppState;
use anyhow::{Context, Result};
use std::sync::Arc;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

pub(crate) fn launch(port: u16, state: Arc<AppState>) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/")
        .parse::<tauri::Url>()
        .context("invalid local UI URL")?;

    let app = tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Onyun")
                .inner_size(1180.0, 780.0)
                .min_inner_size(760.0, 560.0)
                .background_color(tauri::window::Color(0x14, 0x14, 0x14, 0xff))
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!("web/tauri.conf.json"))
        .context("failed to build Tauri desktop application")?;

    let state_on_exit = Arc::clone(&state);
    app.run(move |app_handle, event| {
        if let RunEvent::Exit = event {
            state_on_exit.stop_mint();
        } else if let RunEvent::WindowEvent {
            event: tauri::WindowEvent::CloseRequested { .. },
            ..
        } = event
        {
            state_on_exit.stop_mint();
            if let Some(win) = app_handle.get_webview_window("main") {
                let _ = win.close();
            }
        }
    });

    Ok(())
}
