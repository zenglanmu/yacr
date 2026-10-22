//! Tauri host wiring: window + child webview + the GPU probe lifecycle.
//!
//! The probe is driven from the **Tauri event loop** (`RunEvent::MainEventsCleared`)
//! on the main thread; there is no second event loop.
//!
//! Ownership and threading (safety corrections):
//! - The GPU presenter is **main-thread-local** (a `thread_local` `RefCell`), not
//!   in the shared state: IPC commands run on worker threads and hold clones of
//!   that `Arc`, so a GPU object stored there could be dropped off the main
//!   thread. It is created, used and destroyed only on the main thread.
//! - Two mutexes are shared with worker threads after the WP1b split: the shell
//!   state ([`Shared`], in `cad-desktop-shell`) and [`RenderTelemetry`] (below).
//!   **Acquisition order is always `Shared` -> `RenderTelemetry`, never the
//!   reverse**; the [`ShellTelemetry`] implementation below only locks the
//!   telemetry mutex and therefore can never re-enter `Shared`.
//! - The shared state is **GPU-free**: only status and the input tracker cross
//!   threads, and the shell lock is never held across GPU work.
//! - The surface is explicitly stopped and released on `CloseRequested` and on
//!   `ExitRequested`/`Exit` (surface before window), not only after `Destroyed`
//!   (which arrives after the window is already gone, with no tick afterwards).
//!
//! Lifecycle:
//! - resize: the surface follows the window's physical size; a zero-size window
//!   (minimized/unmapped) **pauses** instead of reconfiguring to zero;
//! - minimize/restore: detected from the window state each tick and resumed when
//!   a non-zero size returns;
//! - GPU failure: recorded in [`ProbeStatus::error`], never a fabricated frame.

use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use cad_host::HostOptions;
use cad_platform::{ConfigStore, PlatformFiles};
use cad_render_wgpu::{ProbeStageTimings, PulledState};
use cad_ui_core::protocol::InputKind;
use cad_ui_core::session::CadSession;

mod admission;
mod dom;
mod init;
mod locale;
mod options;
mod popup;
mod postload;
mod presenter;
mod telemetry;
mod tick;

#[cfg(test)]
mod tests;

// The shared Tauri command/state layer (the `#[tauri::command]` functions and
// the shell half of the former `Shared`) now lives in `cad-desktop-shell`; the
// `__cmd__*`/`__tauri_command_name_*` hidden macros moved with it, so no
// re-export shim remains here.
pub use cad_desktop_shell::commands::{
    cad_close_popup, cad_command, cad_commands, cad_open_popup, cad_set_selected_property,
    cad_set_selected_text, cad_set_setting, cad_set_tool_parameter, cad_set_tool_text,
    cad_settings, cad_standard_view, cad_status, probe_input, probe_status, CadCommandAck,
};
pub(crate) use cad_desktop_shell::state::lock;
pub use cad_desktop_shell::state::new_shared;
pub use cad_desktop_shell::state::{ProbeShared, Shared, ShellPopupHost, ShellTelemetry};

pub use init::init;
pub use options::{
    LaunchOptions, ANIMATION_STEP, FRAME_INTERVAL, INITIAL_HEIGHT, INITIAL_WIDTH, WEBVIEW_LABEL,
    WINDOW_LABEL,
};
pub use popup::PopupAck;
// Evidence-only accessors (`YACR_PROBE_POPUP_EVIDENCE`): the host-side
// open/close functions and the `textEdit` payload derivation. Never used by the
// command path.
pub(crate) use popup::{close_popup_for_probe, open_popup_for_probe, text_edit_payload_for_probe};
// Evidence-only: the **same** selected-text admission `cad_set_selected_text`
// runs, so the text-edit popup phase queues through the real path instead of
// calling `CadSession::set_selected_text` directly.
pub(crate) use cad_ui_core::session::admit_selected_text;
pub use postload::requested_exit_code;
pub(crate) use postload::resolve_process_exit;
pub use presenter::{
    close_presenter, force_realize, force_unrealize, release_presenter, request_stop,
};
pub use tick::tick;

use postload::{ActualRenderSummary, PostloadPanPhase, PostloadPanState};

/// Main-thread-only CAD platform pieces (built once in `init`).
///
/// Holds `Rc` handles, so it lives in a thread-local and is never shared.
struct CadEnv {
    files: std::rc::Rc<dyn PlatformFiles>,
    config: std::rc::Rc<dyn ConfigStore>,
    surface: crate::platform::ReactSurface,
    options: HostOptions,
}

thread_local! {
    /// The GTK overlay bridge (GTK widgets + GPU presenter), main-thread only.
    static BRIDGE: RefCell<Option<crate::gtk_bridge::GtkOverlayOwner>> =
        const { RefCell::new(None) };
    /// The child webview, kept for bounded DOM diagnostics (main thread only).
    static WEBVIEW: RefCell<Option<tauri::Webview>> = const { RefCell::new(None) };
    /// The CAD session (`!Send`): created, driven and dropped on the main thread.
    static CAD: RefCell<Option<CadSession>> = const { RefCell::new(None) };
    /// The main-thread CAD platform pieces (see [`CadEnv`]).
    static CAD_ENV: RefCell<Option<CadEnv>> = const { RefCell::new(None) };
}

/// App-side render telemetry: the former renderer half of `Shared`.
///
/// These fields name `cad-render-wgpu`/`cad-engine` types, so they cannot live in
/// `cad-desktop-shell` (both crates are forbidden there). The command layer sees
/// only the narrow [`ShellTelemetry`] seam.
///
/// # Lock order (frozen)
///
/// Never lock this mutex while holding nothing and then lock [`Shared`] inside
/// the same critical section: the only legal order is `Shared` ->
/// `RenderTelemetry`. The [`ShellTelemetry`] implementation below locks **only**
/// this mutex, which makes the reverse order impossible. The tick acquires the
/// shell lock first whenever it needs both.
pub struct RenderTelemetry {
    /// The most recent frame's per-stage timings (evidence).
    last_timings: Option<ProbeStageTimings>,
    /// The redraw pump's single-pending flag: the tick clears it, so the pump
    /// never queues more than one outstanding wake.
    wake_pending: Arc<AtomicBool>,
    /// The redraw pump that keeps the single Tauri loop awake.
    redraw_pump: Option<crate::redraw::RedrawPump>,
    /// Wall-clock time of the last rendered frame (frame-rate throttle).
    last_frame: Option<Instant>,
    /// Wall-clock time of the last periodic status log line.
    last_log: Option<Instant>,
    /// When the probe started; drives the bounded DOM-diagnostic schedule.
    diag_start: Instant,
    /// How many bounded DOM diagnostics have run (max 2).
    diag_runs: u32,
    /// `YACR_PROBE_NO_GPU`: skip the GPU presenter (WM/lifecycle baseline).
    no_gpu: bool,
    /// Last emitted engine render-gate diagnostic (dedup for the cad-log line).
    last_render_diag: Option<cad_engine::render::RenderDiagnostics>,
    /// Wall-clock time of the last emitted render-gate diagnostic (≤1 Hz).
    last_render_diag_log: Option<Instant>,
    /// The most recent **actual** render (render_seq/view/device), for the
    /// post-load pan driver's readiness/final-frame admission.
    last_actual_render: Option<ActualRenderSummary>,
    /// Observable state of the accelerated pulled organisation from the most
    /// recent **actual** render (`cad_render_wgpu::PulledState`). Surfaced through
    /// the `ShellTelemetry` seam in the `cad_status` evidence line.
    last_pulled_state: Option<PulledState>,
    /// Bounded post-load pan evidence driver state (`YACR_PROBE_POSTLOAD_PAN`).
    postload_pan: Option<PostloadPanState>,
    /// App handle for the post-load driver's bounded natural exit.
    app_handle: Option<tauri::AppHandle>,
    /// The bounded post-load driver's requested process exit status, propagated
    /// to the OS by the entry point after the event loop returns (`AppHandle::exit`
    /// alone was observed to leave the OS status 0).
    postload_pan_exit: Option<i32>,
}

impl RenderTelemetry {
    fn new() -> Self {
        Self {
            last_timings: None,
            wake_pending: Arc::new(AtomicBool::new(false)),
            redraw_pump: None,
            last_frame: None,
            last_log: None,
            diag_start: Instant::now(),
            diag_runs: 0,
            no_gpu: std::env::var_os("YACR_PROBE_NO_GPU").is_some(),
            last_render_diag: None,
            last_render_diag_log: None,
            last_actual_render: None,
            last_pulled_state: None,
            postload_pan: crate::evidence::postload_pan_enabled().then(PostloadPanState::new),
            app_handle: None,
            postload_pan_exit: None,
        }
    }
}

/// Create the app-side render telemetry.
pub fn new_render_telemetry() -> Arc<Mutex<RenderTelemetry>> {
    Arc::new(Mutex::new(RenderTelemetry::new()))
}

/// Lock [`RenderTelemetry`], recovering the guard from a poisoned mutex.
///
/// Lock order: never call this before locking [`Shared`] in the same critical
/// section (see [`RenderTelemetry`]).
pub(crate) fn lock_telemetry(
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) -> MutexGuard<'_, RenderTelemetry> {
    telemetry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The `ShellTelemetry` implementation: reads/writes **only** the app-side
/// telemetry. It never locks [`Shared`], so the `Shared -> telemetry` order
/// cannot invert.
struct AppTelemetry(Arc<Mutex<RenderTelemetry>>);

impl ShellTelemetry for AppTelemetry {
    fn cad_status_evidence(&self) -> String {
        lock_telemetry(&self.0)
            .last_pulled_state
            .as_ref()
            .map(PulledState::to_string)
            .unwrap_or_else(|| "-".to_string())
    }

    fn note_input_accepted(&self, kind: InputKind) {
        // Post-load pan driver accounting: count host-admitted gesture inputs,
        // and flag a concurrent resize (which would make any view change
        // unattributable to the pan).
        let mut telemetry = lock_telemetry(&self.0);
        if let Some(pan) = telemetry.postload_pan.as_mut() {
            match kind {
                InputKind::PointerMove => pan.admitted_moves += 1,
                InputKind::PointerUp => pan.admitted_ups += 1,
                InputKind::Resize => {
                    if matches!(
                        pan.phase,
                        PostloadPanPhase::Gesture | PostloadPanPhase::AwaitingFinal
                    ) {
                        pan.concurrent_resize = true;
                    }
                }
                _ => {}
            }
        }
    }
}

/// The platform popup seam for the shared `cad_open_popup`/`cad_close_popup`
/// commands.
///
/// Popups are in-DOM overlays, so there is no window to create, focus or
/// destroy. The main-thread `textEdit` payload derivation stays in [`popup`];
/// this adapter only marshals the command onto the GTK main thread (through
/// [`crate::main_thread`]) and calls the shared lifecycle. It holds no `Shared`
/// handle (no reference cycle) and never locks the shell state itself.
struct AppPopupHost {
    app: tauri::AppHandle,
}

impl ShellPopupHost for AppPopupHost {
    fn open(&self, shared: &Arc<Mutex<Shared>>, kind: &str) -> Result<PopupAck, String> {
        let app = self.app.clone();
        let shared = shared.clone();
        let kind = kind.to_string();
        crate::main_thread::on_main_thread(&app, move || {
            popup::open_popup(&shared, &kind, &popup::HostPopupOps)
        })
    }

    fn close(&self, shared: &Arc<Mutex<Shared>>) -> Result<(), String> {
        let app = self.app.clone();
        let shared = shared.clone();
        crate::main_thread::on_main_thread(&app, move || popup::close_popup(&shared))
    }
}

/// Register the app-side seams on the shared shell state.
///
/// Called once during Tauri `setup`, before the frontend can invoke a command.
pub(crate) fn register_seams(
    app: &tauri::AppHandle,
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) {
    let mut state = lock(shared);
    state.telemetry = Some(Arc::new(AppTelemetry(telemetry.clone())));
    state.popup_host = Some(Arc::new(AppPopupHost { app: app.clone() }));
}
