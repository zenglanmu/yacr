//! Tauri host wiring for Windows: window + child WebView2 + the GPU surface
//! lifecycle.
//!
//! The host is driven from the **Tauri event loop** (`RunEvent::MainEventsCleared`)
//! on the main thread; there is no second event loop.
//!
//! Ownership and threading:
//! - The CAD child HWND and its wgpu presenter are **main-thread-local** (a
//!   `thread_local` `RefCell`), never in the shared state. IPC commands run on
//!   worker threads and hold clones of that `Arc`, so a GPU object stored there
//!   could be dropped off the main thread. It is created, used and destroyed
//!   only on the main thread.
//! - Two mutexes are shared with worker threads: the shell state ([`Shared`], in
//!   `cad-desktop-shell`) and [`RenderTelemetry`]. **Acquisition order is always
//!   `Shared` -> `RenderTelemetry`, never the reverse**; the [`ShellTelemetry`]
//!   implementation below only locks the telemetry mutex and therefore can never
//!   re-enter `Shared`.
//! - The shared state is **GPU-free**: only status and the input tracker cross
//!   threads, and the shell lock is never held across GPU work.
//! - The surface is explicitly stopped and released on `CloseRequested` and on
//!   `ExitRequested`/`Exit` (surface before window), not only after `Destroyed`
//!   (which arrives after the window is already gone, with no tick afterwards).
//!
//! Composition: the WebView2/React layer is above in both pixels and input; the
//! CAD child HWND is display-only (`WS_DISABLED`) and is kept directly below the
//! wry `WRY_WEBVIEW` container by [`crate::win32_surface`].

use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use cad_host::HostOptions;
use cad_platform::{ConfigStore, PlatformFiles};
use cad_render_wgpu::{ProbeStageTimings, PulledState};
use cad_ui_core::popup::PopupAck;
use cad_ui_core::protocol::InputKind;
use cad_ui_core::session::CadSession;

mod init;
mod popup;
mod tick;

pub(crate) use cad_desktop_shell::state::lock;
pub use cad_desktop_shell::state::new_shared;
pub use cad_desktop_shell::state::{ProbeShared, Shared, ShellPopupHost, ShellTelemetry};

pub use init::init;
pub use tick::tick;

/// Label of the top-level window (same value as the Linux host).
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Minimum interval between presented frames (bounds CPU/GPU work to ~60 fps).
pub const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;

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
    /// The CAD child HWND + presenter, main-thread only.
    static SURFACE: RefCell<Option<crate::win32_surface::Win32CadSurface>> =
        const { RefCell::new(None) };
    /// The CAD session (`!Send`): created, driven and dropped on the main thread.
    static CAD: RefCell<Option<CadSession>> = const { RefCell::new(None) };
    /// The main-thread CAD platform pieces (see [`CadEnv`]).
    static CAD_ENV: RefCell<Option<CadEnv>> = const { RefCell::new(None) };
}

/// Read the durable `wgpu_backend` preference for the next presenter build.
///
/// The presenter is (re)built on the host main thread, so this reads once per
/// construction through the same `ConfigStore` every other user preference uses
/// — the renderer never does I/O and nothing is read per frame. `None` (platform
/// composition not built yet in probe-only runs, or an absent/unreadable
/// preference) maps to the renderer's `auto` default.
pub(crate) fn wgpu_backend_preference() -> Option<String> {
    CAD_ENV.with(|cell| {
        let env = cell.borrow();
        let env = env.as_ref()?;
        crate::platform::wgpu_backend_preference(env.config.as_ref())
    })
}

/// App-side render telemetry.
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
/// this mutex, which makes the reverse order impossible.
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
    /// `YACR_PROBE_NO_GPU`: skip the GPU surface (WM/lifecycle baseline).
    no_gpu: bool,
    /// Observable state of the accelerated pulled organisation from the most
    /// recent **actual** render, surfaced through the `ShellTelemetry` seam.
    last_pulled_state: Option<PulledState>,
    /// What the most recent **actual** render really drew, for the 1 Hz
    /// `[probe] actual-render` evidence line (never replayed).
    last_actual_render: Option<ActualRenderSnapshot>,
    /// The last `cad-present-error` marker text emitted. A persistent failure
    /// repeats every tick; the marker is evidence, not a progress log, so an
    /// identical consecutive text is printed once (it is cleared by the next
    /// successful present).
    last_present_error_text: Option<String>,
    /// Per-tick session counters (cheap bookkeeping reads, no render). A flat
    /// `frames_rendered` while present attempts keep succeeding is the
    /// "re-presenting a retained texture" signature; the two revisions separate
    /// "the retained texture contains commit N" from "the scene reached N".
    last_session_counters: Option<SessionCounters>,
}

/// The per-tick bookkeeping the 1 Hz `[probe] actual-render` line reports.
pub(crate) struct SessionCounters {
    /// Engine render counter (one per **actual** renderer invocation).
    pub(crate) frames_rendered: u64,
    /// Whether a CAD frame is pending (`cad_frame_dirty`).
    pub(crate) dirty: bool,
    /// Scene revision baked into the retained/presented texture.
    pub(crate) rendered_revision: Option<u64>,
    /// Scene revision the GPU has committed (the latest commit).
    pub(crate) committed_revision: Option<u64>,
}

/// The actual-render evidence kept for the 1 Hz line.
pub(crate) struct ActualRenderSnapshot {
    /// Instanced batches the renderer drew (`instanced_batches`).
    pub(crate) instanced_batches: usize,
    /// Per-pulled-segment draws on the accelerated tier.
    pub(crate) instanced_draws: u64,
    /// Observable state of the pulled organisation on this frame.
    pub(crate) pulled: PulledState,
}

impl RenderTelemetry {
    fn new() -> Self {
        Self {
            last_timings: None,
            wake_pending: Arc::new(AtomicBool::new(false)),
            redraw_pump: None,
            last_frame: None,
            last_log: None,
            no_gpu: std::env::var_os("YACR_PROBE_NO_GPU").is_some(),
            last_pulled_state: None,
            last_actual_render: None,
            last_present_error_text: None,
            last_session_counters: None,
        }
    }
}

/// Create the app-side render telemetry.
pub fn new_render_telemetry() -> Arc<Mutex<RenderTelemetry>> {
    Arc::new(Mutex::new(RenderTelemetry::new()))
}

/// Lock [`RenderTelemetry`], recovering the guard from a poisoned mutex.
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

    fn note_input_accepted(&self, _kind: InputKind) {
        // The Windows host ships no evidence-only post-load pan driver, so there
        // is no per-input accounting to do here.
    }
}

/// The platform popup seam for the shared `cad_open_popup`/`cad_close_popup`
/// commands.
///
/// Popups are in-DOM overlays, so there is no window to create, focus or
/// destroy. The main-thread `textEdit` payload derivation stays in [`popup`];
/// this adapter only marshals the command onto the main thread (through
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
fn register_seams(
    app: &tauri::AppHandle,
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) {
    let mut state = lock(shared);
    state.telemetry = Some(Arc::new(AppTelemetry(telemetry.clone())));
    state.popup_host = Some(Arc::new(AppPopupHost { app: app.clone() }));
}

/// Ask the tick to stop and release the presenter (idempotent).
///
/// Lock order: the shell mutex is acquired and released before the telemetry
/// mutex (`Shared` -> `RenderTelemetry`).
pub fn request_stop(shared: &Arc<Mutex<Shared>>, telemetry: &Arc<Mutex<RenderTelemetry>>) {
    lock(shared).shell.stop_requested = true;
    lock_telemetry(telemetry).redraw_pump = None;
}

/// Enter the terminal closing state and drop the surface **on the main thread**:
/// sets `Closing` so a later GPU init can never run, then releases.
pub fn close_presenter() {
    SURFACE.with(|cell| {
        if let Some(surface) = cell.borrow_mut().as_mut() {
            surface.close();
        }
    });
}

/// Stop and release the GPU presenter **on the main thread**, before the window
/// is torn down. Safe to call repeatedly.
pub fn release_presenter() {
    SURFACE.with(|cell| {
        if let Some(surface) = cell.borrow_mut().as_mut() {
            surface.release_gpu();
        }
    });
}
