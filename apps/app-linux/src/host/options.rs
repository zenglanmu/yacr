//! Host startup surface: the window labels/constants and the platform-specific
//! startup policy.
//!
//! The **pure** launch-option parsing is shared and lives in
//! [`cad_ui_core::launch`] (re-exported here so the host's import surface stays
//! `super::options::…`); everything in this file is Linux/Tauri-specific policy:
//! the window/webview labels, the frame-rate constants and the platform config
//! paths.

use std::path::PathBuf;
use std::time::Duration;

pub use cad_ui_core::launch::{LaunchOptions, INITIAL_HEIGHT, INITIAL_WIDTH};

/// Label of the top-level probe window.
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Minimum interval between presented frames while the host is **idle**
/// (bounds CPU/GPU work to ~60 fps).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Minimum interval while pointer/command input is active.
///
/// A 16 ms gate is 62.5 Hz: against a 60 Hz display every ~10th frame lands in
/// the same refresh, and the input queue can wait a full gate (0–16 ms) before
/// the crosshair moves. Matching the redraw pump's 8 ms tick while input is live
/// gives the presenter a fresh cursor position for every refresh (the presenter
/// still drops surplus frames; see `redraw.rs`), and the queue wait drops to
/// ≤8 ms. Idle frames keep the 16 ms cap, so a still window is unchanged.
pub const FRAME_INTERVAL_ACTIVE: Duration = Duration::from_millis(8);
/// How long after the last admitted input the host stays on the active cadence.
///
/// Two idle frame periods: a 60 Hz pointer stream arrives every ~16.7 ms, so
/// this keeps the gate open for the whole gesture without staying hot forever.
pub const INPUT_ACTIVE_WINDOW: Duration = Duration::from_millis(32);
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;

/// Whether input currently owns the frame cadence: queued work or an input
/// admitted within [`INPUT_ACTIVE_WINDOW`].
///
/// Pure so the tick's gate is contract-testable without a GTK loop.
pub fn input_active(pending: bool, last_input_age: Option<Duration>) -> bool {
    pending || last_input_age.is_some_and(|age| age <= INPUT_ACTIVE_WINDOW)
}

/// The frame gate for this tick (active vs idle).
pub fn frame_interval(active: bool) -> Duration {
    if active {
        FRAME_INTERVAL_ACTIVE
    } else {
        FRAME_INTERVAL
    }
}

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
