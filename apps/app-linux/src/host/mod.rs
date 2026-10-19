//! Tauri host wiring: window + child webview + the GPU probe lifecycle.
//!
//! The probe is driven from the **Tauri event loop** (`RunEvent::MainEventsCleared`)
//! on the main thread; there is no second event loop.
//!
//! Ownership and threading (safety corrections):
//! - The GPU presenter is **main-thread-local** (a `thread_local` `RefCell`), not
//!   in the `Arc<Mutex<Shared>>`: IPC commands run on worker threads and hold
//!   clones of that Arc, so a GPU object stored there could be dropped off the
//!   main thread. It is created, used and destroyed only on the main thread.
//! - The shared state is **GPU-free**: only status and the input tracker cross
//!   threads, and the shared lock is never held across GPU work.
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
use cad_ui_core::layout::LayoutTracker;
use cad_ui_core::session::CadSession;
use cad_ui_core::shell::ShellState;

mod admission;
mod commands;
mod dom;
mod init;
mod options;
mod postload;
mod presenter;
mod telemetry;
mod tick;

#[cfg(test)]
mod tests;

// Public host API (unchanged external surface; `crate::host::…` still resolves).
pub use commands::{
    cad_command, cad_commands, cad_set_tool_parameter, cad_status, probe_input, probe_status,
    CadCommandAck,
};
pub use init::init;
pub use options::{
    LaunchOptions, ANIMATION_STEP, FRAME_INTERVAL, INITIAL_HEIGHT, INITIAL_WIDTH, WEBVIEW_LABEL,
    WINDOW_LABEL,
};
pub use postload::requested_exit_code;
pub(crate) use postload::resolve_process_exit;

// The `#[tauri::command]` macro emits hidden helper macros next to each command;
// re-export them alongside the command functions so `generate_handler!` (which
// resolves them at `crate::host::…`) keeps working after the split.
pub(crate) use commands::{
    __cmd__cad_command, __cmd__cad_commands, __cmd__cad_set_tool_parameter, __cmd__cad_status,
    __cmd__probe_input, __cmd__probe_status, __tauri_command_name_cad_command,
    __tauri_command_name_cad_commands, __tauri_command_name_cad_set_tool_parameter,
    __tauri_command_name_cad_status, __tauri_command_name_probe_input,
    __tauri_command_name_probe_status,
};
pub use presenter::{
    close_presenter, force_realize, force_unrealize, release_presenter, request_stop,
};
pub use tick::tick;

use postload::{ActualRenderSummary, PostloadPanState};

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

/// State shared with the IPC commands. Deliberately **GPU-free**: only status,
/// the input tracker and the CAD admission state cross threads.
///
/// The UI-framework-free half lives in [`ShellState`] (`cad-ui-core`) under the
/// explicit `shell` field; this struct adds the Tauri/GTK-specific probe fields
/// beside it. The halves are accessed explicitly (`state.shell.status`,
/// `state.shell.cad_commands`, …) — there is deliberately no `Deref` bridge, so
/// a future Win/macOS host cannot inherit one by accident.
pub struct Shared {
    /// The UI-framework-free shared state (status, input, CAD admission/errors).
    pub shell: ShellState,
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
    /// The probe window handle the tick polls for size/state. Kept here because
    /// the public `Manager` trait does not expose plain windows by label.
    window: Option<tauri::Window>,
    /// When the probe started; drives the bounded DOM-diagnostic schedule.
    diag_start: Instant,
    /// How many bounded DOM diagnostics have run (max 2).
    diag_runs: u32,
    /// `YACR_PROBE_NO_GPU`: skip the GPU presenter (WM/lifecycle baseline).
    no_gpu: bool,
    /// React-reported CAD layout (logical rect -> viewport insets), seq/generation guarded.
    layout: LayoutTracker,
    /// Set when a new GPU generation starts: the cached CAD rect must be
    /// re-validated against the current top geometry (not blindly re-applied).
    layout_revalidate: bool,
    /// The margin-receiving container bounds (logical px) last used for a layout
    /// decision. A change re-validates the cached DOM rect even without a new
    /// sequence (the container resized under an unchanged rect).
    layout_bounds: Option<(f32, f32)>,
    /// Last emitted engine render-gate diagnostic (dedup for the cad-log line).
    last_render_diag: Option<cad_engine::render::RenderDiagnostics>,
    /// Wall-clock time of the last emitted render-gate diagnostic (≤1 Hz).
    last_render_diag_log: Option<Instant>,
    /// The most recent **actual** render (render_seq/view/device), for the
    /// post-load pan driver's readiness/final-frame admission.
    last_actual_render: Option<ActualRenderSummary>,
    /// Observable state of the accelerated pulled organisation from the most
    /// recent **actual** render (`cad_render_wgpu::PulledState`). Surfaced in the
    /// `cad_status` evidence line so a skipped accelerated organisation is visible
    /// next to the status the frontend receives.
    last_pulled_state: Option<PulledState>,
    /// Bounded post-load pan evidence driver state (`YACR_PROBE_POSTLOAD_PAN`).
    postload_pan: Option<PostloadPanState>,
    /// App handle for the post-load driver's bounded natural exit.
    app_handle: Option<tauri::AppHandle>,
    /// The bounded post-load driver's requested process exit status, propagated
    /// to the OS by the entry point after the event loop returns (`AppHandle::exit`
    /// alone was observed to leave the OS status 0).
    postload_pan_exit: Option<i32>,
    /// Startup/CLI options (drawing, font/image dirs, size, probe mode).
    pub launch: LaunchOptions,
    /// Bounded main-thread queue of typed tool-parameter requests (ADR 0016
    /// decision A): admitted by the `cad_set_tool_parameter` IPC command and
    /// drained by the owner tick into `CadSession::set_tool_parameter`. Typed and
    /// separate from `shell.cad_commands` (name-only) so the rejected command-line
    /// text-payload path is never reintroduced.
    pub cad_tool_parameters: Vec<(cad_app::ToolParameter, cad_app::ToolValue)>,
}

/// Managed Tauri state: a shared handle to [`Shared`].
pub struct ProbeShared(pub Arc<Mutex<Shared>>);

/// Create the shared probe state with the parsed launch options.
pub fn new_shared(launch: LaunchOptions) -> Arc<Mutex<Shared>> {
    let cad_mode = !launch.probe_only;
    Arc::new(Mutex::new(Shared {
        shell: ShellState::new(cad_mode),
        last_timings: None,
        wake_pending: Arc::new(AtomicBool::new(false)),
        redraw_pump: None,
        last_frame: None,
        last_log: None,
        window: None,
        diag_start: Instant::now(),
        diag_runs: 0,
        no_gpu: std::env::var_os("YACR_PROBE_NO_GPU").is_some(),
        layout: LayoutTracker::new(),
        layout_revalidate: false,
        layout_bounds: None,
        last_render_diag: None,
        last_render_diag_log: None,
        last_actual_render: None,
        last_pulled_state: None,
        postload_pan: crate::evidence::postload_pan_enabled().then(PostloadPanState::new),
        app_handle: None,
        postload_pan_exit: None,
        launch,
        cad_tool_parameters: Vec::new(),
    }))
}

fn lock(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
