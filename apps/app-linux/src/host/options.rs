//! Host startup surface: the window labels/constants and the platform-specific
//! startup policy.
//!
//! The **pure** launch-option parsing and the input-hot frame gate are shared
//! and live in [`cad_ui_core::launch`] / [`cad_ui_core::shell`] (re-exported
//! here so the host's import surface stays `super::options::…`); everything in
//! this file is Linux/Tauri-specific policy: the window/webview labels, the
//! marker animation step and the platform config paths.

use std::path::PathBuf;

pub use cad_ui_core::launch::{LaunchOptions, INITIAL_HEIGHT, INITIAL_WIDTH};
pub use cad_ui_core::shell::{frame_interval, FRAME_INTERVAL};
/// Compatibility re-exports: the Linux tick now gates through
/// [`cad_ui_core::shell::ShellState::input_active`], but `super::options::{…}`
/// stays the stable import surface for the shared gate.
#[allow(unused_imports)]
pub use cad_ui_core::shell::{input_active, FRAME_INTERVAL_ACTIVE, INPUT_ACTIVE_WINDOW};

/// Label of the top-level probe window.
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;

/// Resolve the durable config/preference paths (`$XDG_CONFIG_HOME` or `~/.config`).
///
/// `None` when no home is resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
pub(super) fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    match dir {
        Some(dir) => {
            let app = dir.join("yacr");
            (
                Some(app.join("config.json")),
                Some(app.join("preference.json")),
            )
        }
        None => (None, None),
    }
}

/// Whether the window starts maximized.
///
/// Default is maximized; the acceptance-only `YACR_PROBE_NO_MAXIMIZE` env opts
/// out (its presence, not its value, is passed here).
pub(super) fn start_maximized(no_maximize_env_present: bool) -> bool {
    !no_maximize_env_present
}
