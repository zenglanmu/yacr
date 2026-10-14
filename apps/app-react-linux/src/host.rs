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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cad_engine::render::{BackendPreference, FrameCompleteness, RenderSessionHandle};
use cad_host::{HostOptions, HostPlatform};
use cad_platform::{ConfigStore, HostCapabilities, PlatformFiles, SurfaceMetrics};
use cad_render_wgpu::ProbeStageTimings;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::cad::CadStatus;
use crate::protocol::{InputTracker, ProbeInput, ProbeInputAck, ProbeStatus};

/// Label of the top-level probe window.
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Initial window size in physical pixels.
pub const INITIAL_WIDTH: u32 = 960;
/// Initial window size in physical pixels.
pub const INITIAL_HEIGHT: u32 = 600;
/// Minimum interval between presented frames (bounds CPU/GPU work to ~60 fps).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;
/// Bounded main-thread CAD command queue capacity (admission only, not completion).
pub const CAD_COMMAND_CAP: usize = 256;
/// Bounded ordered main-thread CAD input queue capacity.
pub const CAD_INPUT_CAP: usize = 512;

/// CLI/startup options for the React host (owned by the main thread).
///
/// `--open <path>` (or `YACR_TEST_DWG`) is the real fixture funnel: the path is
/// read through the same `PlatformFiles` open path, never a fake headless open.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// A drawing to open at startup (real file, read through the open funnel).
    pub drawing: Option<PathBuf>,
    /// Explicit CAD font directory (`--fonts-dir`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`).
    pub images_dir: Option<PathBuf>,
    /// Initial logical window size.
    pub size: [f64; 2],
    /// `--probe` / `YACR_PROBE_ONLY`: keep the old probe marker path and report
    /// CAD as unsupported (no CAD session).
    pub probe_only: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            drawing: None,
            fonts_dir: None,
            images_dir: None,
            size: [f64::from(INITIAL_WIDTH), f64::from(INITIAL_HEIGHT)],
            probe_only: false,
        }
    }
}

impl LaunchOptions {
    /// Parse `--open <path>`, `--fonts-dir <path>`, `--images-dir <path>` and
    /// `--probe` from CLI arguments (the real fixture funnel; never a fake open).
    pub fn from_args<I: Iterator<Item = String>>(args: I) -> Self {
        let mut options = Self::default();
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--open" => options.drawing = args.next().map(PathBuf::from),
                "--fonts-dir" => options.fonts_dir = args.next().map(PathBuf::from),
                "--images-dir" => options.images_dir = args.next().map(PathBuf::from),
                "--probe" => options.probe_only = true,
                _ => {}
            }
        }
        options
    }
}

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
    static CAD: RefCell<Option<crate::cad::CadSession>> = const { RefCell::new(None) };
    /// The main-thread CAD platform pieces (see [`CadEnv`]).
    static CAD_ENV: RefCell<Option<CadEnv>> = const { RefCell::new(None) };
}

/// Initialization script: capture uncaught JS errors into `window.__yacr_diag`
/// so the diagnostic eval can report them (bounded diagnostics only).
const ERROR_HOOK_JS: &str = r#"
(function () {
  try {
    window.__yacr_diag = window.__yacr_diag || { errors: [] };
    window.addEventListener('error', function (e) {
      try { window.__yacr_diag.errors.push('error: ' + (e.message || e.error || String(e))); } catch (_) {}
    });
    window.addEventListener('unhandledrejection', function (e) {
      try { window.__yacr_diag.errors.push('rejection: ' + String(e.reason)); } catch (_) {}
    });
  } catch (_) {}
})();
"#;

/// Diagnostic eval: report DOM mount state, bounding rects, computed styles and
/// counts. Returns an object; the callback receives it serialized as JSON.
const DOM_DIAG_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_diag || { errors: [] };
    function info(el) {
      if (!el) return null;
      var r = el.getBoundingClientRect();
      var cs = getComputedStyle(el);
      return {
        rect: [r.x, r.y, r.width, r.height],
        display: cs.display, visibility: cs.visibility, opacity: cs.opacity,
        background: cs.backgroundColor, color: cs.color,
        children: el.children.length, htmlLen: el.innerHTML.length
      };
    }
    return {
      readyState: document.readyState,
      href: location.href,
      title: document.title,
      bodyTextLen: (document.body && document.body.innerText || '').length,
      buttons: document.querySelectorAll('button').length,
      inputs: document.querySelectorAll('input').length,
      selects: document.querySelectorAll('select').length,
      root: info(document.getElementById('root')),
      main: info(document.querySelector('main.shell')),
      ribbon: info(document.querySelector('.ribbon')),
      commandBar: info(document.querySelector('.command-bar')),
      errors: d.errors
    };
  } catch (e) {
    return { evalError: String(e) };
  }
})()
"#;

fn run_dom_diag(run: u32) {
    WEBVIEW.with(|cell| {
        if let Some(webview) = cell.borrow().as_ref() {
            let result = webview.eval_with_callback(DOM_DIAG_JS, move |json| {
                eprintln!("[probe] DOM diag #{run}: {json}");
            });
            if let Err(error) = result {
                eprintln!("[probe] DOM diag #{run} dispatch failed: {error}");
            }
        } else {
            eprintln!("[probe] DOM diag #{run}: no webview handle");
        }
    });
}

/// The kind of a host-side (transport) CAD error.
///
/// A good event clears only its **own** kind, so a successful present never
/// hides an open failure and a successful bind never hides a present error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostErrorKind {
    /// `set_render_session` (device attach) failed.
    Bind,
    /// A CAD frame present/render failed.
    Present,
    /// The owner-thread CAD tick failed.
    Tick,
}

/// A host-side CAD error (technical caption + kind). Never UI prose.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostError {
    kind: HostErrorKind,
    text: String,
}

/// Record a host-side error of `kind` (replacing any previous host error).
fn note_host_error(state: &mut Shared, kind: HostErrorKind, text: String) {
    state.cad_host_error = Some(HostError { kind, text });
}

/// Clear the host error **only** if it is of `kind` (never a different kind, and
/// never a CadSession business error, which lives in the snapshot).
fn clear_host_error(state: &mut Shared, kind: HostErrorKind) {
    if state
        .cad_host_error
        .as_ref()
        .is_some_and(|error| error.kind == kind)
    {
        state.cad_host_error = None;
    }
}

/// Project the CadSession snapshot plus the host-side error into the status the
/// frontend reads.
///
/// No false clear: when the host error is absent the snapshot's own (business /
/// open) error is preserved; when both exist they are both shown.
fn projected_status(mut snapshot: CadStatus, host_error: Option<&str>) -> CadStatus {
    snapshot.error = match (host_error, snapshot.error.take()) {
        (Some(host), Some(business)) => Some(format!("{host}; {business}")),
        (Some(host), None) => Some(host.to_string()),
        (None, business) => business,
    };
    snapshot
}

/// Outcome of one bind step for the CAD render session.
enum BindStep {
    /// No bridge is installed yet (deferred, not a failure).
    NoBridge,
    /// Skipped: the bridge is closing/failed, or this generation already failed.
    Skipped,
    /// A bind was attempted for `generation` with `result`.
    Attempted(u64, Result<(), String>),
}

/// Whether a bind may be attempted: never once ready, and never again for the
/// generation that already failed (fail-stop until the generation changes).
fn bind_allowed(ready: bool, failed_generation: Option<u64>, owner_generation: u64) -> bool {
    !ready && failed_generation != Some(owner_generation)
}

/// The telemetry state of one CAD frame attempt (never UI text).
///
/// `NoSession`/`Waiting`/`Error` are explicitly **not** `Presented`, so a log
/// consumer can never mistake a non-present for a new frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CadFrameLogState {
    Presented,
    Waiting,
    Error,
    NoSession,
}

impl CadFrameLogState {
    fn label(self) -> &'static str {
        match self {
            Self::Presented => "presented",
            Self::Waiting => "waiting",
            Self::Error => "error",
            Self::NoSession => "no-session",
        }
    }
}

/// Classify a CAD frame result for the env-gated telemetry log.
fn cad_frame_log_state(result: &Option<Result<Option<u64>, String>>) -> CadFrameLogState {
    match result {
        Some(Ok(Some(_))) => CadFrameLogState::Presented,
        Some(Ok(None)) => CadFrameLogState::Waiting,
        Some(Err(_)) => CadFrameLogState::Error,
        None => CadFrameLogState::NoSession,
    }
}

/// A rendered-frame completeness label for the telemetry log (technical, not UI).
fn frame_completeness_label(completeness: FrameCompleteness) -> &'static str {
    match completeness {
        FrameCompleteness::CurrentScene => "current_scene",
        FrameCompleteness::BaseOnlyPendingOverlays => "base_only_pending_overlays",
    }
}

/// Render an optional telemetry number, using `-` for an absent value.
fn opt_telemetry_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Render an optional telemetry duration (µs), using `-` for an absent value.
fn opt_telemetry_u128(value: Option<u128>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// State shared with the IPC commands. Deliberately **GPU-free**: only status
/// and the input tracker cross threads.
pub struct Shared {
    /// The probe status returned to the frontend.
    pub status: ProbeStatus,
    /// Input/sequence tracker (marker offset and interaction state).
    pub input: InputTracker,
    /// Set by destroy/exit handling; the tick performs the stop and release.
    stop_requested: bool,
    /// Whether the presenter has been stopped and released.
    stopped: bool,
    /// Whether the last tick paused (minimized / zero-size) the probe.
    paused: bool,
    /// Count of ticks that passed the frame-rate throttle (diagnostic).
    ticks: u64,
    /// Chosen present mode label (evidence).
    present_mode: Option<String>,
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
    /// React-reported CAD layout (CSS rect -> native margins), seq/generation guarded.
    layout: crate::layout::LayoutTracker,
    /// Set when a new GPU generation starts: the cached CAD rect must be
    /// re-validated against the current top geometry (not blindly re-applied).
    layout_revalidate: bool,
    /// Whether the CAD session path is enabled (false in `--probe` mode).
    pub cad_mode: bool,
    /// Whether the main-thread CAD session has been created.
    pub cad_ready: bool,
    /// Bounded main-thread CAD command queue (admission only; not completion).
    pub cad_commands: Vec<String>,
    /// Bounded ordered main-thread CAD input queue.
    pub cad_inputs: Vec<ProbeInput>,
    /// Bounded per-input IPC receive time (`seq -> Instant`), for the env-gated
    /// drag-latency telemetry. Kept separate from the input queue (no queue
    /// change) and capped at [`CAD_INPUT_CAP`].
    cad_input_recv: Vec<(u64, Instant)>,
    /// Latest `cad_status` snapshot (main-thread derived; cloned for IPC).
    pub cad_status: Option<CadStatus>,
    /// Last real **host-side** (transport) CAD error (bind/present/tick). CAD
    /// business/open failures live in the CadSession snapshot instead.
    cad_host_error: Option<HostError>,
    /// The presenter generation whose bind attempt failed. A bind is attempted
    /// **once** per generation: until the generation changes (a new device) no
    /// further attempt is made, so there is no per-tick initialization loop.
    cad_bind_failed_generation: Option<u64>,
    /// Startup/CLI options (drawing, font/image dirs, size, probe mode).
    pub launch: LaunchOptions,
}

/// Managed Tauri state: a shared handle to [`Shared`].
pub struct ProbeShared(pub Arc<Mutex<Shared>>);

/// Create the shared probe state with the parsed launch options.
pub fn new_shared(launch: LaunchOptions) -> Arc<Mutex<Shared>> {
    let cad_mode = !launch.probe_only;
    Arc::new(Mutex::new(Shared {
        status: ProbeStatus::default(),
        input: InputTracker::new(),
        stop_requested: false,
        stopped: false,
        paused: false,
        ticks: 0,
        present_mode: None,
        last_timings: None,
        wake_pending: Arc::new(AtomicBool::new(false)),
        redraw_pump: None,
        last_frame: None,
        last_log: None,
        window: None,
        diag_start: Instant::now(),
        diag_runs: 0,
        no_gpu: std::env::var_os("YACR_PROBE_NO_GPU").is_some(),
        layout: crate::layout::LayoutTracker::new(),
        layout_revalidate: false,
        cad_mode,
        cad_ready: false,
        cad_commands: Vec::new(),
        cad_inputs: Vec::new(),
        cad_input_recv: Vec::new(),
        cad_status: if cad_mode {
            None
        } else {
            Some(unsupported_status(Some(
                "CAD is disabled in probe mode".into(),
            )))
        },
        cad_host_error: None,
        cad_bind_failed_generation: None,
        launch,
    }))
}

/// A `cad_status` snapshot that advertises no CAD capability (probe mode or a
/// failed session): an explicit error, never a fabricated document/count.
fn unsupported_status(error: Option<String>) -> CadStatus {
    CadStatus {
        document_label: None,
        entity_count: None,
        loading: false,
        error,
        can_open: false,
        can_fit: false,
        backend: None,
        adapter: None,
        camera: None,
        import_count: 0,
        frame_count: 0,
    }
}

fn lock(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Resolve the durable config/preference paths (`$XDG_CONFIG_HOME` or `~/.config`).
///
/// `None` when no home is resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    match dir {
        Some(dir) => {
            let app = dir.join("yacr-react");
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
fn start_maximized(no_maximize_env_present: bool) -> bool {
    !no_maximize_env_present
}

/// Build the probe window, its child webview and the GPU presenter.
///
/// Runs on the main thread during Tauri `setup`.
pub fn init(app: &mut tauri::App, shared: &Arc<Mutex<Shared>>) -> tauri::Result<()> {
    // `YACR_PROBE_OPAQUE` is a diagnostic-only escape hatch (window visual test).
    let transparent = std::env::var_os("YACR_PROBE_OPAQUE").is_none();
    // Default: open maximized (the CAD partition should use the whole screen).
    // `YACR_PROBE_NO_MAXIMIZE` (acceptance/evidence only) opts out; it is never
    // the default.
    let maximize = start_maximized(std::env::var_os("YACR_PROBE_NO_MAXIMIZE").is_some());
    let window = tauri::window::WindowBuilder::new(app, WINDOW_LABEL)
        .title("yacr react probe")
        .inner_size(f64::from(INITIAL_WIDTH), f64::from(INITIAL_HEIGHT))
        .min_inner_size(320.0, 240.0)
        .resizable(true)
        .transparent(transparent)
        .maximized(maximize)
        .build()?;

    // Top-level XID check: the probe surface is created on this exact window.
    log_top_level_xid(&window);

    // Use the window's real size for the child webview, not the initial
    // constants (the WM may already have adjusted the window).
    let initial_size = window
        .inner_size()
        .unwrap_or(tauri::PhysicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT));

    // Transparent, full-window child webview overlaying the probe surface.
    let child = {
        let webview = tauri::webview::WebviewBuilder::new(
            WEBVIEW_LABEL,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .transparent(transparent)
        .auto_resize()
        .initialization_script(ERROR_HOOK_JS)
        .on_page_load(|_webview, payload| {
            eprintln!(
                "[probe] page_load url={} event={:?}",
                payload.url(),
                payload.event()
            );
        });
        let child = window.add_child(
            webview,
            tauri::PhysicalPosition::new(0, 0),
            tauri::PhysicalSize::new(initial_size.width.max(1), initial_size.height.max(1)),
        )?;
        // Keep a handle for the bounded DOM diagnostics (main thread only).
        WEBVIEW.with(|cell| *cell.borrow_mut() = Some(child.clone()));
        Some(child)
    };

    // Window events: log geometry (WM-native evidence) and release the surface
    // before the window goes away.
    let events_shared = shared.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            eprintln!(
                "[probe] WM event: window resized to {}x{}",
                size.width, size.height
            );
        }
        tauri::WindowEvent::Moved(position) => {
            eprintln!(
                "[probe] WM event: window moved to {},{}",
                position.x, position.y
            );
        }
        tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            eprintln!("[probe] WM event: scale factor {scale_factor}");
        }
        tauri::WindowEvent::Focused(focused) => {
            eprintln!("[probe] WM event: focused={focused}");
        }
        tauri::WindowEvent::CloseRequested { .. } => {
            eprintln!("[probe] WM event: close requested; closing surface first");
            close_presenter();
        }
        tauri::WindowEvent::Destroyed => {
            eprintln!("[probe] WM event: destroyed; requesting stop");
            request_stop(&events_shared);
        }
        _ => {}
    });

    let evidence_window = window.clone();
    let no_gpu = lock(shared).no_gpu;
    lock(shared).window = Some(window.clone());

    // ---- CAD platform composition (main thread; `Rc`, kept in a TLS) ----
    let launch = lock(shared).launch.clone();
    if !launch.probe_only {
        let locale = std::env::var("YACR_LOCALE").unwrap_or_else(|_| "zh-CN".to_string());
        match crate::platform::ReactFiles::new(&locale) {
            Ok(files) => {
                let (config, preference) = config_paths();
                let config_store: std::rc::Rc<dyn ConfigStore> = match (config, preference) {
                    (Some(config), Some(preference)) => {
                        std::rc::Rc::new(crate::platform::ReactConfigStore::new(config, preference))
                    }
                    _ => std::rc::Rc::new(crate::platform::ReactConfigStore::non_persistent()),
                };
                let env = CadEnv {
                    files: std::rc::Rc::new(files),
                    config: config_store,
                    surface: crate::platform::ReactSurface::new(),
                    options: HostOptions {
                        fonts: Vec::new(),
                        fonts_dir: launch.fonts_dir.clone(),
                        images_dir: launch.images_dir.clone(),
                        headless: false,
                        drawing: launch.drawing.clone(),
                        size: launch.size,
                    },
                };
                CAD_ENV.with(|cell| *cell.borrow_mut() = Some(env));
            }
            Err(error) => {
                let text = format!("{error:?}");
                eprintln!("[probe] CAD platform init FAILED: {text}");
                // A platform-composition failure is a CadSession-level failure:
                // it lives in the snapshot, not the host transient error.
                lock(shared).cad_status = Some(unsupported_status(Some(text)));
            }
        }
    }

    if no_gpu {
        eprintln!("[probe] YACR_PROBE_NO_GPU: GPU bridge skipped (React-only baseline)");
    } else {
        // GTK overlay bridge: an independent native child surface (windowed
        // GtkDrawingArea) with the React webview reparented as a transparent
        // sibling. **No top-level fallback** — failure is explicit.
        match crate::gtk_bridge::install(&window, WINDOW_LABEL) {
            Ok(owner) => {
                owner.log_tree();
                let release = std::rc::Rc::new(release_presenter);
                crate::gtk_bridge::install_unrealize_hooks(&owner, release);
                BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
                if let Some(webview) = child.as_ref() {
                    if let Err(error) = crate::gtk_bridge::reparent_webview(webview, WINDOW_LABEL) {
                        eprintln!("[probe] bridge reparent FAILED: {error}");
                        lock(shared).status.error =
                            Some(format!("gtk bridge reparent failed: {error}"));
                    }
                }
            }
            Err(error) => {
                eprintln!("[probe] bridge install FAILED: {error} (no top-level fallback)");
                lock(shared).status.error = Some(format!("gtk bridge install failed: {error}"));
            }
        }
    }
    // Keep the single event loop awake so the probe renders continuously.
    let wake_pending = lock(shared).wake_pending.clone();
    let pump = crate::redraw::RedrawPump::spawn(app.handle(), wake_pending);
    lock(shared).redraw_pump = Some(pump);

    // Env-gated, evidence-only lifecycle driver (no effect on normal runs).
    crate::evidence::maybe_spawn(&evidence_window, child.as_ref());
    crate::evidence::maybe_spawn_lifecycle(&evidence_window, app.handle());
    Ok(())
}

/// Ask the tick to stop and release the presenter (idempotent).
pub fn request_stop(shared: &Arc<Mutex<Shared>>) {
    let mut state = lock(shared);
    state.stop_requested = true;
    state.redraw_pump = None;
}

/// Enter the terminal closing state and drop the surface **on the main thread**
/// (B1): sets `Closing` so a later `try_init_gpu` can never run, then releases.
pub fn close_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.close();
        }
    });
}

/// Stop and release the GPU presenter **on the main thread**, before the window
/// is torn down. Safe to call repeatedly.
pub fn release_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.release_gpu();
        }
    });
}

/// Evidence-only: force the overlay to unrealize on the main thread, running the
/// registered unrealize hook (GPU released before native teardown). The owner is
/// temporarily moved out of the thread-local cell so GTK's **synchronous**
/// unrealize signal cannot re-enter an already-borrowed `RefCell`; the GPU is
/// released here, before the native unrealize, then the owner is put back.
/// Returns whether a bridge existed.
pub fn force_unrealize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.release_gpu();
            owner.force_unrealize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}

/// Evidence-only: re-realize/re-map the overlay after a forced unrealize so the
/// next tick can create a new GPU generation. Uses the same move-out pattern so
/// any synchronous GTK signal cannot re-enter a borrowed `RefCell`.
pub fn force_realize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.force_realize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}

/// One probe tick. Runs on the main thread. The shared lock is held only for
/// bookkeeping, never across GPU work.
pub fn tick(shared: &Arc<Mutex<Shared>>) {
    // ---- short critical section: bookkeeping only ----
    let (size, marker) = {
        let mut state = lock(shared);
        state.wake_pending.store(false, Ordering::SeqCst);

        if state.stopped {
            return;
        }
        if state.stop_requested {
            drop(state);
            release_presenter();
            let mut state = lock(shared);
            state.stopped = true;
            state.status.stopped = true;
            return;
        }

        let now = Instant::now();
        if let Some(last) = state.last_frame {
            if now.duration_since(last) < FRAME_INTERVAL {
                return;
            }
        }
        state.last_frame = Some(now);
        state.ticks += 1;

        // Periodic (~1 Hz) status line for evidence; includes the last frame's
        // per-stage timings and the frame index.
        if state
            .last_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            state.last_log = Some(now);
            let stages = state
                .last_timings
                .map(|t| {
                    format!(
                        "write={} acquire={} configure={} submit={} scope={} present={} total={}",
                        t.write_buffer_us,
                        t.acquire_us,
                        t.configure_us,
                        t.submit_us,
                        t.scope_pop_us,
                        t.present_us,
                        t.total_us
                    )
                })
                .unwrap_or_else(|| "none".to_string());
            eprintln!(
                "[probe] status backend={:?} adapter={:?} size={}x{} frames={} ticks={} present_mode={:?} stopped={} marker={:.3} stages_us[{}] error={:?}",
                state.status.backend,
                state.status.adapter,
                state.status.width,
                state.status.height,
                state.status.frames,
                state.ticks,
                state.present_mode,
                state.status.stopped,
                state.input.marker_offset(),
                stages,
                state.status.error,
            );
        }

        // Bounded DOM diagnostics at ~6s and ~14s (2 runs max); they only
        // dispatch an eval, so they never block the loop.
        if state.diag_runs < 2 {
            let due = Duration::from_secs(6 + 8 * u64::from(state.diag_runs));
            if state.diag_start.elapsed() >= due {
                state.diag_runs += 1;
                let run = state.diag_runs;
                drop(state);
                run_dom_diag(run);
                return;
            }
        }

        let Some(window) = state.window.clone() else {
            return;
        };
        let minimized = window.is_minimized().unwrap_or(false);
        let size = window
            .inner_size()
            .unwrap_or(tauri::PhysicalSize::new(0, 0));

        if minimized || size.width == 0 || size.height == 0 {
            // Zero/occluded surface: pause instead of reconfiguring to zero.
            if !state.paused {
                eprintln!(
                    "[probe] paused (minimized={minimized}) size={}x{}",
                    size.width, size.height
                );
                state.paused = true;
            }
            state.status.width = size.width;
            state.status.height = size.height;
            return;
        }
        if state.paused {
            eprintln!("[probe] resumed size={}x{}", size.width, size.height);
            state.paused = false;
        }
        // Report the live size even when no GPU presenter is active.
        state.status.width = size.width;
        state.status.height = size.height;

        // Input wins over the fixed animation on the frame it arrives.
        let marker = if state.input.take_dirty() {
            state.input.marker_offset()
        } else {
            state.input.advance_animation(ANIMATION_STEP);
            state.input.marker_offset()
        };
        (size, marker)
    };

    // ---- deferred bridge GPU init (main thread, no shared lock held) ----
    // `LayoutTracker::confirmed` is the single source of truth for the CAD-rect
    // gate; the bridge keeps no second copy.
    let layout_confirmed = lock(shared).layout.confirmed();
    let init_result = BRIDGE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|owner| owner.try_init_gpu(layout_confirmed))
    });
    if let Some(result) = init_result {
        let mut state = lock(shared);
        match result {
            Ok(()) => {
                let mut surface = None;
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow().as_ref() {
                        if let Some(adapter) = owner.adapter() {
                            state.status.backend = Some(adapter.backend.clone());
                            state.status.adapter = Some(adapter.name.clone());
                        }
                        if let Some(mode) = owner.present_mode() {
                            state.present_mode = Some(format!("{mode:?}"));
                        }
                        surface = owner.surface_size();
                    }
                });
                // A new GPU generation: keep the DOM input sequence monotonic and
                // re-validate the cached CAD rect against the current top geometry
                // (do not blindly re-apply it; there is no cross-generation reset).
                state.layout.note_generation();
                state.layout_revalidate = true;
                // Report the actual child-surface size, not the window size.
                if let Some((w, h)) = surface {
                    state.status.width = w;
                    state.status.height = h;
                }
            }
            Err(error) => {
                state.status.error = Some(error);
            }
        }
        return;
    }

    // ---- React CAD layout -> native margins (main thread, no shared lock) ----
    let rect = lock(shared).input.last_rect();
    if let Some(rect) = rect {
        let (css_w, css_h) = lock(shared)
            .window
            .clone()
            .map(|window| {
                let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
                let size = window
                    .inner_size()
                    .unwrap_or(tauri::PhysicalSize::new(0, 0));
                (
                    size.width as f32 / scale as f32,
                    size.height as f32 / scale as f32,
                )
            })
            .unwrap_or((0.0, 0.0));
        let (revalidate, decision) = {
            let mut state = lock(shared);
            let revalidate = state.layout_revalidate;
            state.layout_revalidate = false;
            let decision = if revalidate {
                state.layout.revalidate(&rect, css_w, css_h)
            } else {
                state.layout.accept(&rect, css_w, css_h)
            };
            (revalidate, decision)
        };
        match decision {
            Ok(margins) => {
                // Keep the CAD surface source in sync with the accepted DOM
                // rect. The host `tick` re-reads this surface every cycle
                // (`HostCore::tick` -> `metrics`), so `viewport.logical_size`
                // always matches the rect the renderer draws into. Leaving it
                // at the creation-time size made `Camera::zoom_at` anchor off
                // the cursor after any DOM-rect change (window resize, panel
                // toggle, DPR change).
                let scale = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.scale_factor().ok())
                    .unwrap_or(1.0);
                CAD_ENV.with(|cell| {
                    if let Some(env) = cell.borrow().as_ref() {
                        env.surface.set_metrics(cad_surface_metrics(&rect, scale));
                    }
                });
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow_mut().as_mut() {
                        owner.apply_layout(margins);
                    }
                });
                let surface =
                    BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
                let top = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.inner_size().ok());
                // Distinguish the three geometries explicitly (evidence only).
                eprintln!(
                    "[probe] geometry top={}x{} dom={}x{} native_surface={}x{} margins top={} bottom={} start={} end={} revalidated={}",
                    top.map(|s| s.width).unwrap_or(0),
                    top.map(|s| s.height).unwrap_or(0),
                    rect.width,
                    rect.height,
                    surface.map(|s| s.0).unwrap_or(0),
                    surface.map(|s| s.1).unwrap_or(0),
                    margins.top,
                    margins.bottom,
                    margins.start,
                    margins.end,
                    revalidate,
                );
            }
            Err(crate::layout::LayoutError::StaleSequence) => {}
            Err(error) => {
                if revalidate {
                    // The cached rect no longer fits the current top geometry:
                    // wait for fresh metrics instead of applying stale margins.
                    eprintln!(
                        "[probe] layout revalidation deferred (waiting for fresh metrics): {error:?}"
                    );
                } else {
                    lock(shared).status.error = Some(format!("layout rejected: {error:?}"));
                }
            }
        }
    }

    // ---- CAD session: lazy creation once the DOM rect is confirmed ----
    let cad_mode = lock(shared).cad_mode;
    if cad_mode && CAD.with(|cell| cell.borrow().is_none()) {
        let rect = lock(shared).input.last_rect();
        if let Some(rect) = rect {
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
            let created = CAD_ENV.with(|cell| {
                let env = cell.borrow();
                let env = env.as_ref()?;
                // The surface must report before `CadSession::new` (the open
                // funnel validates canvas metrics against it).
                env.surface.set_metrics(cad_surface_metrics(&rect, scale));
                let platform = HostPlatform {
                    files: env.files.clone(),
                    config_store: env.config.clone(),
                    surface: std::rc::Rc::new(env.surface.clone()),
                    resources: std::rc::Rc::new(cad_host::DesktopResources),
                    capabilities: HostCapabilities {
                        open: true,
                        new: false,
                        save: false,
                        plot: false,
                        import_worker: true,
                    },
                };
                Some(crate::cad::CadSession::new(
                    env.options.clone(),
                    platform,
                    RenderSessionHandle::new(BackendPreference::WebGpu),
                ))
            });
            match created {
                Some(Ok(session)) => {
                    CAD.with(|cell| *cell.borrow_mut() = Some(session));
                    eprintln!("[probe] cad session created");
                }
                Some(Err(error)) => {
                    let text = format!("{error:?}");
                    eprintln!("[probe] cad session creation FAILED: {text}");
                    // A CadSession-level (core/open) failure lives in the
                    // snapshot, never in the host transient error.
                    lock(shared).cad_status = Some(unsupported_status(Some(text)));
                }
                None => {}
            }
        }
    }

    // ---- CAD session bind (main thread): only a successful bind marks ready ----
    //
    // The device is attached through the presenter's `device_queue`. With no
    // presenter yet this stores the session and returns `Ok` (device deferred,
    // not a failure). A bind is attempted **at most once per presenter
    // generation**: a failure is recorded against the generation, kept visible,
    // and the session stays not-ready (waiting) until the generation changes (a
    // new device) — no per-tick initialization loop. A bridge that is not
    // installed yet, or is closing/failed, defers without a failure.
    if cad_mode && !lock(shared).cad_ready {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).cad_bind_failed_generation;
            let step = BRIDGE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return BindStep::NoBridge;
                };
                if owner.failure().is_some()
                    || matches!(owner.state(), crate::gtk_bridge::BridgeState::Closing)
                {
                    return BindStep::Skipped;
                }
                let generation = owner.generation();
                if !bind_allowed(false, failed_generation, generation) {
                    return BindStep::Skipped;
                }
                BindStep::Attempted(generation, owner.set_render_session(Some(render)))
            });
            match step {
                BindStep::Attempted(generation, Ok(())) => {
                    let mut state = lock(shared);
                    state.cad_ready = true;
                    state.cad_bind_failed_generation = None;
                    clear_host_error(&mut state, HostErrorKind::Bind);
                    eprintln!("[probe] cad render session bound (generation {generation})");
                }
                BindStep::Attempted(generation, Err(error)) => {
                    let mut state = lock(shared);
                    state.cad_bind_failed_generation = Some(generation);
                    note_host_error(
                        &mut state,
                        HostErrorKind::Bind,
                        format!("cad bind failed: {error}"),
                    );
                    eprintln!(
                        "[probe] cad bind FAILED (generation {generation}); fail-stop until a new generation: {error}"
                    );
                }
                BindStep::NoBridge | BindStep::Skipped => {}
            }
        }
    }

    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    let cad_log = std::env::var_os("YACR_PROBE_CAD_LOG").is_some();
    // Telemetry: the latest input sequence in this batch and its IPC-to-drain
    // age (measured from the Rust `Instant` recorded at admission).
    let mut last_input_seq: Option<u64> = None;
    let mut input_age_us: Option<u128> = None;
    if cad_mode {
        let (commands, inputs) = {
            let mut state = lock(shared);
            (
                std::mem::take(&mut state.cad_commands),
                std::mem::take(&mut state.cad_inputs),
            )
        };
        let queued = commands.len() + inputs.len();
        last_input_seq = inputs.last().map(|input| input.sequence);
        input_age_us = last_input_seq.and_then(|sequence| {
            let state = lock(shared);
            state
                .cad_input_recv
                .iter()
                .rev()
                .find(|(seq, _)| *seq == sequence)
                .map(|(_, received)| received.elapsed().as_micros())
        });
        let inputs_started = Instant::now();
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    for command in &commands {
                        // Admission already returned `{accepted:true}`. The real
                        // business outcome is the CadSession snapshot error; it
                        // is not duplicated into the host transient error.
                        if let Err(error) = session.command(command) {
                            eprintln!("[probe] cad command rejected: {error}");
                        }
                    }
                    for input in &inputs {
                        if let Err(error) = session.handle_input(input) {
                            eprintln!("[probe] cad input rejected: {error}");
                        }
                    }
                }
            });
        }
        let inputs_us = inputs_started.elapsed().as_micros();
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        // A tick failure is a host transient error, cleared only by a later
        // successful tick.
        let tick_started = Instant::now();
        let tick = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.tick()));
        let tick_us = tick_started.elapsed().as_micros();
        match tick {
            Some(Ok(())) => clear_host_error(&mut lock(shared), HostErrorKind::Tick),
            Some(Err(error)) => note_host_error(
                &mut lock(shared),
                HostErrorKind::Tick,
                format!("cad tick failed: {error}"),
            ),
            None => {}
        }
        if cad_log && queued > 0 {
            eprintln!("[probe] cad-stage queued={queued} input_us={inputs_us} tick_us={tick_us}");
        }
    }

    // ---- GPU work, without holding the shared lock ----
    let surface = BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
    if cad_mode {
        let (logical, scale) = {
            let rect = lock(shared).input.last_rect();
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
            (
                rect.map(|rect| [f64::from(rect.width), f64::from(rect.height)])
                    .unwrap_or([0.0, 0.0]),
                scale,
            )
        };
        // Draw one CAD frame through the bound render session (main thread, no
        // shared lock held across the engine/GPU work).
        let frame_started = Instant::now();
        let frame_result: Option<Result<Option<u64>, String>> = CAD.with(|cell| {
            let borrowed = cell.borrow();
            let session = borrowed.as_ref()?;
            Some(BRIDGE.with(|bridge| {
                let mut slot = bridge.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return Ok(None);
                };
                if owner.failure().is_some() {
                    return Ok(None);
                }
                let (w, h) = owner.desired_size();
                if w == 0 || h == 0 {
                    return Ok(None);
                }
                owner.resize_to(w, h)?;
                match owner.frame_cad(logical, scale) {
                    Ok(Some(stats)) => {
                        session.note_frame(stats.frame_index);
                        Ok(Some(stats.frame_index))
                    }
                    // No new frame and no reusable retained frame: explicit
                    // waiting; no blank/fake grid is presented.
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                }
            }))
        });
        let (backend, adapter) = BRIDGE.with(|cell| {
            let bridge = cell.borrow();
            bridge
                .as_ref()
                .and_then(|owner| owner.adapter())
                .map(|info| (Some(info.backend.clone()), Some(info.name.clone())))
                .unwrap_or((None, None))
        });
        match &frame_result {
            Some(Ok(Some(frames))) => {
                let mut state = lock(shared);
                state.status.frames = *frames;
                state.status.error = None;
                // A good present clears only a previous present error (never a
                // bind error, and never the CadSession business error).
                clear_host_error(&mut state, HostErrorKind::Present);
            }
            // Explicit waiting: no present happened, so nothing is cleared.
            Some(Ok(None)) => {}
            Some(Err(error)) => {
                let mut state = lock(shared);
                note_host_error(
                    &mut state,
                    HostErrorKind::Present,
                    format!("cad present failed: {error}"),
                );
            }
            None => {}
        }
        let frame_us = frame_started.elapsed().as_micros();
        let snapshot = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|session| session.snapshot(backend, adapter, 0))
        });
        if let Some(snapshot) = &snapshot {
            lock(shared).cad_status = Some(snapshot.clone());
        }
        // Consume the presented-frame observation exactly once, and only for a
        // real present (never for a waiting/error frame), so the telemetry can
        // never replay a stale frame.
        let observation = if matches!(&frame_result, Some(Ok(Some(_)))) {
            BRIDGE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|owner| owner.take_last_cad_frame_observation())
            })
        } else {
            None
        };
        if let Some(observation) = observation {
            // The native presenter's real CAD stage timings (this is what makes
            // the periodic `status … stages_us[…]` meaningful in CAD mode).
            lock(shared).last_timings = Some(observation.timings);
        }
        if cad_log {
            let view_current = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|session| session.render_session().view_revision())
            });
            let state_tag = cad_frame_log_state(&frame_result);
            if let (Some(obs), CadFrameLogState::Presented) = (observation, state_tag) {
                let camera = snapshot.as_ref().and_then(|status| status.camera);
                let (cx, cy, cscale) = camera
                    .map(|c| (c.center[0], c.center[1], c.scale))
                    .unwrap_or((f64::NAN, f64::NAN, f64::NAN));
                let t = obs.timings;
                eprintln!(
                    "[probe] cad-frame presented view_current={} view_rendered={} complete={} device_epoch={} texture_rev={} size={}x{} frame_id={} frame_us={} input_seq={} input_age_us={} camera_center={cx:.3},{cy:.3} camera_scale={cscale:.6} stages_us[write={} acquire={} configure={} submit={} scope={} present={} total={}]",
                    view_current.unwrap_or(0),
                    obs.rendered_view_revision,
                    frame_completeness_label(obs.completeness),
                    obs.device_epoch,
                    obs.texture_revision,
                    obs.size.0,
                    obs.size.1,
                    obs.frame_id,
                    frame_us,
                    opt_telemetry_u64(last_input_seq),
                    opt_telemetry_u128(input_age_us),
                    t.write_buffer_us,
                    t.acquire_us,
                    t.configure_us,
                    t.submit_us,
                    t.scope_pop_us,
                    t.present_us,
                    t.total_us,
                );
            } else {
                eprintln!(
                    "[probe] cad-frame {} frame_us={} view_current={}",
                    state_tag.label(),
                    frame_us,
                    view_current.unwrap_or(0),
                );
            }
        }
    } else {
        // ---- probe frame (marker path; `--probe` mode only) ----
        let outcome: Option<Result<(u64, ProbeStageTimings), String>> = BRIDGE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let owner = slot.as_mut()?;
            if owner.failure().is_some() {
                return None;
            }
            let (w, h) = owner.desired_size();
            if w == 0 || h == 0 {
                return None;
            }
            if let Err(error) = owner.resize_to(w, h) {
                return Some(Err(error));
            }
            owner.frame(marker)
        });
        if let Some(outcome) = outcome {
            let mut state = lock(shared);
            match outcome {
                Ok((frames, timings)) => {
                    state.status.frames = frames;
                    // Report the actual child-surface size when the surface
                    // exists, otherwise the window size (top-level).
                    if let Some((w, h)) = surface {
                        state.status.width = w;
                        state.status.height = h;
                    } else {
                        state.status.width = size.width;
                        state.status.height = size.height;
                    }
                    state.last_timings = Some(timings);
                    state.status.error = None;
                }
                Err(error) => {
                    state.status.error = Some(error);
                }
            }
        }
    }
}

/// Log the X11 top-level XID the probe surface is created on.
fn log_top_level_xid(window: &tauri::Window) {
    match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Xlib(xlib) => eprintln!(
                "[probe] top-level X11 XID (Xlib) = 0x{:x} visual=0x{:x}",
                xlib.window, xlib.visual_id
            ),
            RawWindowHandle::Xcb(xcb) => {
                eprintln!("[probe] top-level X11 XID (Xcb) = 0x{:x}", xcb.window.get())
            }
            RawWindowHandle::Wayland(_) => {
                eprintln!("[probe] ERROR: top-level handle is Wayland, not X11")
            }
            other => eprintln!("[probe] unexpected top-level raw handle: {other:?}"),
        },
        Err(error) => eprintln!("[probe] could not read the top-level window handle: {error}"),
    }
}

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    lock(&shared.0).status.clone()
}

/// Admission result of `cad_command`: queued, **not** completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

/// The canonical command string a frontend command names (`OPEN`/`FIT` only).
fn normalize_cad_command(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_uppercase().as_str() {
        "OPEN" => Some("OPEN"),
        "FIT" => Some("FIT"),
        _ => None,
    }
}

/// Admit a command into the bounded main-thread queue (unknown refused first).
fn admit_command(state: &mut Shared, command: &str) -> Result<(), String> {
    let Some(normalized) = normalize_cad_command(command) else {
        return Err(format!("unsupported CAD command: {}", command.trim()));
    };
    if !state.cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if state.cad_commands.len() >= CAD_COMMAND_CAP {
        return Err("CAD command queue is full".into());
    }
    state.cad_commands.push(normalized.to_string());
    Ok(())
}

/// Queue one ordered CAD input, or refuse explicitly (not ready / full).
fn queue_cad_input(state: &mut Shared, input: &ProbeInput) -> Result<(), String> {
    if !state.cad_mode {
        // Probe marker path keeps the tracker semantics.
        return Ok(());
    }
    if !state.cad_ready {
        return Err("cad_waiting".into());
    }
    if state.cad_inputs.len() >= CAD_INPUT_CAP {
        return Err("input_queue_full".into());
    }
    state.cad_inputs.push(input.clone());
    // Record the IPC receive time for the env-gated drag-latency telemetry.
    state.cad_input_recv.push((input.sequence, Instant::now()));
    if state.cad_input_recv.len() > CAD_INPUT_CAP {
        state.cad_input_recv.remove(0);
    }
    Ok(())
}

/// `cad_command` command: admit `OPEN`/`FIT` into the bounded main-thread queue.
///
/// An unknown command is rejected immediately (never queued). `{accepted:true}`
/// means the command was **queued**, not that it completed; the real outcome
/// surfaces through `cad_status.error`/`loading`.
#[tauri::command]
pub fn cad_command(
    shared: tauri::State<'_, ProbeShared>,
    command: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    admit_command(&mut state, &command)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> CadStatus {
    let status = {
        let state = lock(&shared.0);
        let host_error = state
            .cad_host_error
            .as_ref()
            .map(|error| error.text.as_str());
        match state.cad_status.clone() {
            Some(snapshot) => projected_status(snapshot, host_error),
            None => unsupported_status(None),
        }
    };
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        match serde_json::to_string(&status) {
            Ok(json) => eprintln!("[probe] cad_status {json}"),
            Err(error) => eprintln!("[probe] cad_status serialize failed: {error}"),
        }
    }
    status
}

/// Map an accepted frontend CAD rect (CSS pixels) plus the window scale factor
/// to the surface metrics the host reads each tick.
///
/// The scale is clamped positive so a degenerate window scale factor can never
/// produce invalid canvas metrics (the host then refuses to advance the
/// viewport instead of fabricating one).
fn cad_surface_metrics(rect: &crate::layout::CadMetrics, scale: f64) -> SurfaceMetrics {
    SurfaceMetrics {
        logical: [f64::from(rect.width), f64::from(rect.height)],
        scale: scale.max(0.1),
        safe_insets: [0.0; 4],
    }
}

/// `probe_input` command: sequence-gate one frontend input and route it.
///
/// In CAD mode a pointer/wheel input is queued for the main thread only when the
/// CAD session is ready; before that it is refused with an explicit `cad_waiting`
/// reason (never a pretend success), and a full queue is refused explicitly.
/// `resize` is applied by the tick through the CAD rect path.
#[tauri::command]
pub fn probe_input(shared: tauri::State<'_, ProbeShared>, input: ProbeInput) -> ProbeInputAck {
    let kind = input.kind;
    let sequence = input.sequence;
    let ack = lock(&shared.0).input.apply(&input);
    // Evidence line: shows the real IPC path (invoke -> command -> tracker).
    // x/y/dx/dy/width are logged so the accepted input can be tied to real OS
    // coordinates (diagnosis only; identifiers stay English).
    eprintln!(
        "[probe] input kind={} seq={} accepted={} marker={:.3} terminal={} reason={:?} x={:?} y={:?} dx={:?} dy={:?} width={:?}",
        kind.as_str(),
        sequence,
        ack.accepted,
        ack.marker_offset,
        ack.terminal,
        ack.reason,
        input.x,
        input.y,
        input.dx,
        input.dy,
        input.width,
    );
    if !ack.accepted {
        return ack;
    }
    if matches!(kind, crate::protocol::InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(&shared.0);
        queue_cad_input(&mut state, &input)
    };
    match queued {
        Ok(()) => ack,
        Err(reason) => ProbeInputAck {
            accepted: false,
            sequence,
            marker_offset: ack.marker_offset,
            terminal: ack.terminal,
            reason: Some(reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InputKind;

    fn input(kind: InputKind) -> ProbeInput {
        ProbeInput {
            sequence: 1,
            kind,
            x: Some(1.0),
            y: Some(2.0),
            dx: None,
            dy: None,
            width: None,
            height: None,
            dpr: None,
        }
    }

    #[test]
    fn launch_options_parse_cli() {
        let opts = LaunchOptions::from_args(
            [
                "--open",
                "/tmp/a.dwg",
                "--fonts-dir",
                "/tmp/fonts",
                "--images-dir",
                "/tmp/img",
                "--probe",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/a.dwg"))
        );
        assert_eq!(
            opts.fonts_dir.as_deref(),
            Some(std::path::Path::new("/tmp/fonts"))
        );
        assert_eq!(
            opts.images_dir.as_deref(),
            Some(std::path::Path::new("/tmp/img"))
        );
        assert!(opts.probe_only);
        assert_eq!(opts.size, [960.0, 600.0]);
        // Unknown args are ignored; missing values leave the field unset.
        let empty = LaunchOptions::from_args(["--nope"].into_iter().map(String::from));
        assert!(empty.drawing.is_none() && !empty.probe_only);
    }

    #[test]
    fn accepted_rect_maps_to_canvas_surface_metrics() {
        let rect = crate::layout::CadMetrics {
            x: 12.0,
            y: 40.0,
            width: 800.0,
            height: 600.0,
            dpr: 2.0,
            sequence: 7,
        };
        let metrics = cad_surface_metrics(&rect, 2.0);
        // The surface reports the CAD rect (CSS/logical px), not the offset.
        assert_eq!(metrics.logical, [800.0, 600.0]);
        assert_eq!(metrics.scale, 2.0);
        assert_eq!(metrics.safe_insets, [0.0; 4]);
        // A degenerate scale factor is clamped, never accepted as-is.
        assert_eq!(cad_surface_metrics(&rect, 0.0).scale, 0.1);
    }

    #[test]
    fn unsupported_status_is_explicit_and_not_fake() {
        let status = unsupported_status(None);
        assert_eq!(status.document_label, None);
        assert_eq!(status.entity_count, None);
        assert_eq!(status.camera, None);
        assert!(!status.can_open && !status.can_fit && !status.loading);
        assert_eq!(status.import_count, 0);
        assert_eq!(status.frame_count, 0);
        let with_error = unsupported_status(Some("boom".into()));
        assert_eq!(with_error.error.as_deref(), Some("boom"));
    }

    #[test]
    fn command_admission_normalizes_and_is_bounded() {
        assert_eq!(normalize_cad_command(" open "), Some("OPEN"));
        assert_eq!(normalize_cad_command("FIT"), Some("FIT"));
        for rejected in ["", "LINE", "SAVE", "OPEN FIT", "OPENX"] {
            assert_eq!(normalize_cad_command(rejected), None, "{rejected:?}");
        }

        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        assert!(state.cad_mode && !state.cad_ready);
        assert!(admit_command(&mut state, "LINE").is_err());
        assert!(admit_command(&mut state, "OPEN").is_ok());
        assert_eq!(state.cad_commands, vec!["OPEN".to_string()]);
        for _ in 1..CAD_COMMAND_CAP {
            admit_command(&mut state, "FIT").unwrap();
        }
        assert_eq!(state.cad_commands.len(), CAD_COMMAND_CAP);
        assert!(
            admit_command(&mut state, "FIT").is_err(),
            "the command queue is bounded at {CAD_COMMAND_CAP}"
        );
    }

    #[test]
    fn input_queue_waits_for_cad_then_bounds() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        // Before the CAD session exists an input is refused explicitly.
        assert_eq!(
            queue_cad_input(&mut state, &input(InputKind::PointerDown)).unwrap_err(),
            "cad_waiting"
        );
        state.cad_ready = true;
        for _ in 0..CAD_INPUT_CAP {
            queue_cad_input(&mut state, &input(InputKind::PointerMove)).unwrap();
        }
        assert_eq!(state.cad_inputs.len(), CAD_INPUT_CAP);
        assert_eq!(
            queue_cad_input(&mut state, &input(InputKind::PointerMove)).unwrap_err(),
            "input_queue_full"
        );
    }

    #[test]
    fn probe_mode_reports_cad_unsupported() {
        let launch = LaunchOptions {
            probe_only: true,
            ..LaunchOptions::default()
        };
        let shared = new_shared(launch);
        let state = lock(&shared);
        assert!(!state.cad_mode);
        let status = state.cad_status.clone().expect("probe status present");
        assert!(status.error.is_some());
        assert!(!status.can_open && !status.can_fit);
        assert_eq!(status.import_count, 0);
    }

    fn sample_status(error: Option<&str>) -> CadStatus {
        CadStatus {
            document_label: Some("demo".into()),
            entity_count: Some(1),
            loading: false,
            error: error.map(str::to_string),
            can_open: true,
            can_fit: true,
            backend: Some("vulkan".into()),
            adapter: Some("P600".into()),
            camera: None,
            import_count: 0,
            frame_count: 0,
        }
    }

    #[test]
    fn bind_error_is_visible_and_never_marks_ready() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        assert!(!state.cad_ready);
        note_host_error(
            &mut state,
            HostErrorKind::Bind,
            "cad bind failed: attach".into(),
        );
        // A bind failure never marks the session ready.
        assert!(!state.cad_ready);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad bind failed: attach"));
        assert!(
            projected.can_fit,
            "the session document capability is preserved"
        );
        assert!(projected.can_open);
    }

    #[test]
    fn present_failure_is_visible_without_business_error_then_clears() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        note_host_error(
            &mut state,
            HostErrorKind::Present,
            "cad present failed: gpu".into(),
        );
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad present failed: gpu"));
        clear_host_error(&mut state, HostErrorKind::Present);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error, None);
    }

    #[test]
    fn open_failure_persists_despite_present_success() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        // While both exist they are both shown.
        note_host_error(
            &mut state,
            HostErrorKind::Present,
            "cad present failed: gpu".into(),
        );
        let projected = projected_status(
            sample_status(Some("read failed")),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(
            projected.error.as_deref(),
            Some("cad present failed: gpu; read failed")
        );
        // A successful present clears only the present error; the open failure stays.
        clear_host_error(&mut state, HostErrorKind::Present);
        let projected = projected_status(
            sample_status(Some("read failed")),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("read failed"));
    }

    #[test]
    fn host_error_clear_is_kind_specific() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        note_host_error(&mut state, HostErrorKind::Bind, "cad bind failed: x".into());
        clear_host_error(&mut state, HostErrorKind::Present);
        assert!(
            state.cad_host_error.is_some(),
            "a present clear never clears a bind error"
        );
        clear_host_error(&mut state, HostErrorKind::Bind);
        assert!(state.cad_host_error.is_none());
    }

    #[test]
    fn previous_host_error_does_not_stick_once_corrected() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        note_host_error(&mut state, HostErrorKind::Tick, "cad tick failed: y".into());
        clear_host_error(&mut state, HostErrorKind::Tick);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error, None);
    }

    #[test]
    fn bind_attempts_are_bounded_per_generation() {
        // Once a generation has failed, 100 further ticks never re-attempt it.
        let failed = Some(3u64);
        for _ in 0..100 {
            assert!(
                !bind_allowed(false, failed, 3),
                "no repeated bind in the same generation"
            );
        }
        // A new presenter generation allows exactly one fresh attempt.
        assert!(bind_allowed(false, failed, 4));
        // A generation 0 failure is also fail-stopped until the generation changes.
        assert!(!bind_allowed(false, Some(0), 0));
        assert!(bind_allowed(false, Some(0), 1));
        // Ready never attempts; no failure yet is allowed.
        assert!(!bind_allowed(true, None, 4));
        assert!(bind_allowed(false, None, 0));
    }

    #[test]
    fn bind_error_survives_a_waiting_present() {
        let shared = new_shared(LaunchOptions::default());
        let mut state = lock(&shared);
        note_host_error(&mut state, HostErrorKind::Bind, "cad bind failed: x".into());
        // An explicit-waiting present (`Ok(None)`) clears nothing, so the bind
        // error stays visible.
        clear_host_error(&mut state, HostErrorKind::Present);
        assert!(state.cad_host_error.is_some());
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad bind failed: x"));
    }

    #[test]
    fn start_maximized_defaults_true_and_opts_out_explicitly() {
        assert!(start_maximized(false), "the default is maximized");
        assert!(
            !start_maximized(true),
            "the acceptance-only env opts out of maximize"
        );
    }

    #[test]
    fn cad_frame_log_state_never_claims_a_present_for_unknown_or_waiting() {
        assert_eq!(cad_frame_log_state(&None), CadFrameLogState::NoSession);
        assert_eq!(
            cad_frame_log_state(&Some(Ok(None))),
            CadFrameLogState::Waiting
        );
        assert_eq!(
            cad_frame_log_state(&Some(Err("x".into()))),
            CadFrameLogState::Error
        );
        assert_eq!(
            cad_frame_log_state(&Some(Ok(Some(7)))),
            CadFrameLogState::Presented
        );
        // Unknown/waiting/error are never a present.
        assert_ne!(CadFrameLogState::NoSession, CadFrameLogState::Presented);
        assert_ne!(CadFrameLogState::Waiting, CadFrameLogState::Presented);
        assert_ne!(CadFrameLogState::Error, CadFrameLogState::Presented);
        assert_eq!(
            frame_completeness_label(FrameCompleteness::CurrentScene),
            "current_scene"
        );
        assert_eq!(
            frame_completeness_label(FrameCompleteness::BaseOnlyPendingOverlays),
            "base_only_pending_overlays"
        );
        assert_eq!(opt_telemetry_u64(None), "-");
        assert_eq!(opt_telemetry_u64(Some(3)), "3");
        assert_eq!(opt_telemetry_u128(None), "-");
        assert_eq!(opt_telemetry_u128(Some(42)), "42");
    }
}
