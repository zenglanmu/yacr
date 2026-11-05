//! The Android host's Tauri entry point, its two shell-only reports, and the
//! main-thread CAD state the `native-below` tick drives.
//!
//! # Carrier
//!
//! `surfaceMode = "native-below"`, `documentOwner = "native"`: the native
//! [`cad_ui_core::session::CadSession`] owns `cad-db` and the renderer presents to
//! the `SurfaceView` the Kotlin lane layers **beneath** the WebView. The wasm
//! session does **not** run on this carrier, and this crate never instantiates a
//! second document authority (`AGENTS.md` iron rule 8).
//!
//! The shared React chrome is unchanged: on `native-below` it renders
//! `.native-input` (the single input funnel) and no `<canvas>`, and it routes every
//! CAD command to the native host through the shared
//! [`cad_desktop_shell::vendor::register_commands_with_vendor`] handler.
//!
//! # Commands
//!
//! The handler is the **shared** desktop command set plus the two shell-only
//! reports this carrier inherited from the `dom-canvas` phase
//! ([`canvas_session_status`], [`canvas_runtime_failed`]). Both remain valid: they
//! are the chrome's shell reports, and on `native-below` the chrome simply does not
//! send them (it has no wasm runtime failure to report).
//!
//! # Popups
//!
//! The shared `cad_open_popup`/`cad_close_popup` commands are served by a real
//! seam ([`crate::popup::AndroidPopupHost`]): popups are in-DOM overlays, so there
//! is no native window — the seam only marshals the shared lifecycle onto the loop
//! thread (bounded wait) and derives the `textEdit` payload from the live
//! `CadSession` there. Options, About and text-edit are therefore available on
//! this carrier, with the same admission gate as every desktop host.
//!
//! # Tick drive
//!
//! The tick runs on [`tauri::RunEvent::MainEventsCleared`], exactly like
//! `apps/app-linux`/`apps/app-macos`. On Android that event **is** delivered:
//! `tao 0.37.1`'s Android loop calls the handler with `Event::MainEventsCleared`
//! once per looper iteration (`platform_impl/android/mod.rs`), and the loop is kept
//! awake by [`pump::RedrawPump`], whose `run_on_main_thread` wake goes through
//! `EventLoopProxy::send_event` → `looper.wake()`. There is no second event loop and
//! no timer thread doing render work: the pump only wakes the loop, and every GPU
//! and `CadSession` call happens inside the single Tauri loop. (Verified by reading
//! the pinned dependency sources; **NOT RUN** on a device.)
//!
//! Which thread that is matters: `tao`'s `onFirstActivityCreate` spawns a thread
//! that prepares its own looper and calls the app entry point, so the Tauri loop
//! (and the tick) runs on that thread — **not** on the Android UI thread, where the
//! `SurfaceHolder` JNI callbacks arrive. [`crate::surface`] documents the one
//! `Mutex<PlatformSurface>` hand-off between them; the `CadSession`, the platform
//! pieces and the presenter never leave the tick thread.
//!
//! # Ownership
//!
//! * The `CadSession` (`!Send`) and the CAD platform pieces live in thread-locals
//!   of the tick thread and are created, used and dropped only there.
//! * [`Shared`] and [`RenderTelemetry`] are the two mutexes shared with IPC worker
//!   threads. Lock order is **always** `Shared` -> `RenderTelemetry`, never the
//!   reverse; the [`ShellTelemetry`] implementation locks only the telemetry.
//! * The GPU presenter is in [`crate::surface`]'s thread-local, never in `Shared`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::ThreadId;
use std::time::Instant;

use cad_host::HostOptions;
use cad_platform::{ConfigStore, PlatformFiles};
use cad_render_wgpu::{ProbeStageTimings, PulledState};
use cad_ui_core::protocol::InputKind;
use cad_ui_core::session::CadSession;

pub(crate) use cad_desktop_shell::state::{lock, Shared};

use crate::platform;
use crate::policy;
use crate::refusal;
use crate::report;

pub(crate) mod pump;
pub(crate) mod tick;

/// `logcat` tag of every line this host prints (`adb logcat -s yacr`).
pub const LOG_TAG: &str = "yacr";

/// Label of the single window Tauri creates from `tauri.conf.json`.
///
/// The Android carrier has exactly one window/webview created by the Tauri glue;
/// the host never builds a second one.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// The two shell-only report commands of this carrier.
///
/// These are the only names the local handler owns; every other name goes to the
/// shared registry handler unchanged.
pub const REPORT_COMMANDS: [&str; 2] = [
    refusal::CANVAS_SESSION_STATUS_COMMAND,
    refusal::CANVAS_RUNTIME_FAILED_COMMAND,
];

/// Install the logger that makes [`log`] records reach `logcat`.
///
/// Idempotent (`init_once`): a second call is a no-op, so a re-created Activity
/// cannot double-install it.
pub fn init_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag(LOG_TAG),
    );
}

/// The main-thread CAD platform pieces (built once in `setup`).
///
/// Holds `Rc` handles, so it lives in a thread-local and is never shared.
pub(crate) struct CadEnv {
    pub(crate) files: Rc<dyn PlatformFiles>,
    pub(crate) config: Rc<dyn ConfigStore>,
    pub(crate) surface: platform::AndroidSurface,
    pub(crate) options: HostOptions,
}

thread_local! {
    /// The CAD session (`!Send`): created, driven and dropped on the tick thread
    /// (the Tauri loop thread).
    pub(crate) static CAD: RefCell<Option<CadSession>> = const { RefCell::new(None) };
    /// The main-thread CAD platform pieces (see [`CadEnv`]).
    pub(crate) static CAD_ENV: RefCell<Option<CadEnv>> = const { RefCell::new(None) };
}

/// The Tauri/tao loop thread, recorded once by [`run`].
///
/// The native `CadSession`, `CAD_ENV` and the presenter are thread-locals of that
/// thread, so "is the caller the owner thread?" must be a real recorded
/// [`ThreadId`], never an assumption. On Android the loop thread is the one
/// `tao`'s entry point spawns for the event loop, **not** the Android UI thread
/// (where the `SurfaceHolder` callbacks arrive), so a toolkit probe like GTK's
/// does not exist here; comparing the id is the honest equivalent.
static LOOP_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// Record the calling thread as the Tauri loop thread (idempotent).
fn record_loop_thread() {
    let _ = LOOP_THREAD.set(std::thread::current().id());
}

/// Whether the caller is the Tauri loop thread that owns the `!Send` CAD state.
pub(crate) fn on_loop_thread() -> bool {
    LOOP_THREAD
        .get()
        .is_some_and(|loop_thread| *loop_thread == std::thread::current().id())
}

/// App-side render telemetry.
///
/// These fields name `cad-render-wgpu` types, so they cannot live in
/// `cad-desktop-shell` (that crate is renderer-free). The command layer sees only
/// the narrow [`cad_desktop_shell::state::ShellTelemetry`] seam.
///
/// # Lock order (frozen)
///
/// Never lock this mutex and then lock [`Shared`] in the same critical section: the
/// only legal order is `Shared` -> `RenderTelemetry`. The [`ShellTelemetry`]
/// implementation locks **only** this mutex, which makes the reverse impossible.
pub struct RenderTelemetry {
    /// The most recent presented frame's per-stage timings (evidence).
    last_timings: Option<ProbeStageTimings>,
    /// The redraw pump's single-pending flag: the tick clears it.
    wake_pending: Arc<AtomicBool>,
    /// The redraw pump that keeps the single Tauri loop awake.
    redraw_pump: Option<pump::RedrawPump>,
    /// Wall-clock time of the last rendered frame (frame-rate throttle).
    last_frame: Option<Instant>,
    /// Wall-clock time of the last periodic status log line.
    last_log: Option<Instant>,
    /// Observable state of the accelerated pulled organisation on the most recent
    /// **actual** render, surfaced through the `ShellTelemetry` seam.
    last_pulled_state: Option<PulledState>,
    /// The presenter generation the layout was last re-validated against.
    last_surface_generation: Option<u64>,
    /// The last logged DOM-rect / platform-surface geometry pair, so a steady
    /// disagreement does not flood `logcat`.
    last_geometry_notice: Option<((u32, u32), (u32, u32))>,
}

impl RenderTelemetry {
    fn new() -> Self {
        Self {
            last_timings: None,
            wake_pending: Arc::new(AtomicBool::new(false)),
            redraw_pump: None,
            last_frame: None,
            last_log: None,
            last_pulled_state: None,
            last_surface_generation: None,
            last_geometry_notice: None,
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
/// section.
pub(crate) fn lock_telemetry(
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) -> MutexGuard<'_, RenderTelemetry> {
    telemetry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The `ShellTelemetry` implementation: reads/writes **only** the app telemetry.
struct AppTelemetry(Arc<Mutex<RenderTelemetry>>);

impl cad_desktop_shell::state::ShellTelemetry for AppTelemetry {
    fn cad_status_evidence(&self) -> String {
        lock_telemetry(&self.0)
            .last_pulled_state
            .as_ref()
            .map(PulledState::to_string)
            .unwrap_or_else(|| "-".to_string())
    }

    fn note_input_accepted(&self, _kind: InputKind) {
        // There is no bounded post-load pan evidence driver on an Android host:
        // the desktop driver is an evidence fixture that needs a real fixture
        // drawing and an `--open` funnel, neither of which exists on this carrier.
        // Input acceptance itself is recorded by the shell tracker.
    }
}

/// Build the CAD platform composition (idempotent).
///
/// Options come from the announced asset directories
/// ([`crate::platform::asset_dirs`]). The handshake is posted to the Android UI
/// thread and may arrive after `setup`, so the tick re-reads the announcement every
/// tick **until the session exists** (see [`crate::host::tick`]); once the session is
/// built its `HostOptions` are fixed and a later announcement no longer applies.
/// `drawing` is deliberately always `None`: there is no `--open` argument. A
/// document arrives after startup through the SAF picker (`OPEN` or an external
/// VIEW intent), never as a fabricated startup path.
fn install_cad_env(shared: &Arc<Mutex<Shared>>) {
    let launch = lock(shared).launch.clone();
    let (fonts_dir, images_dir) = platform::option_dirs();
    let env = CadEnv {
        files: Rc::new(platform::AndroidFiles::new()),
        config: platform::boxed_config_store(),
        surface: platform::AndroidSurface::new(),
        options: HostOptions {
            fonts: Vec::new(),
            fonts_dir: fonts_dir.or_else(|| launch.fonts_dir.clone()),
            images_dir: images_dir.or_else(|| launch.images_dir.clone()),
            headless: false,
            drawing: None,
            size: launch.size,
        },
    };
    CAD_ENV.with(|cell| *cell.borrow_mut() = Some(env));
}

/// Register the app-side seams on the shared shell state.
///
/// Called once during Tauri `setup`, before the frontend can invoke a command.
/// The popup seam is a **real** host: popups are in-DOM overlays, so
/// [`crate::popup::AndroidPopupHost`] only marshals the shared lifecycle onto the
/// loop thread (where the `!Send` CAD selection lives) and derives the `textEdit`
/// payload there. Options/About/text-edit therefore work exactly like on the
/// desktop hosts; a failure is propagated verbatim, never turned into a pretend
/// acknowledgement.
fn register_seams(
    app: &tauri::AppHandle,
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) {
    let mut state = lock(shared);
    state.telemetry = Some(Arc::new(AppTelemetry(telemetry.clone())));
    state.popup_host = Some(Arc::new(crate::popup::AndroidPopupHost::new(app.clone())));
}

/// Ask the tick to stop and release the presenter (idempotent).
pub fn request_stop(shared: &Arc<Mutex<Shared>>, telemetry: &Arc<Mutex<RenderTelemetry>>) {
    lock(shared).shell.stop_requested = true;
    lock_telemetry(telemetry).redraw_pump = None;
}

/// Release the GPU presenter **on the tick thread** (surface before process exit).
pub fn release_presenter() {
    crate::surface::release();
}

/// Enter the terminal closing state and drop the surface (idempotent).
pub fn close_presenter() {
    crate::surface::release();
}

/// Report the session's drawing identity, one `logcat` line, no state.
///
/// `apps/desktop-ui/src/App.tsx` sends this whenever the session's status names a
/// drawing. On `native-below` the identity is the native session's own, so the host
/// **only logs** it — storing it would be a second document model (iron rule 8).
#[tauri::command]
pub fn canvas_session_status(
    label: String,
    batches: Option<u32>,
    draws: Option<u32>,
) -> Result<(), String> {
    log::info!("{}", report::session_status_line(&label, batches, draws));
    Ok(())
}

/// Report a `dom-canvas` transport failure: marker line **first**, then the exit.
///
/// Kept from the `dom-canvas` phase (this command is part of the chrome's shell
/// report set and remains valid). On `native-below` the chrome sends nothing here:
/// there is no wasm session whose transport could fail. The stable marker line
/// `yacr: unsupported-runtime: <step>` is emitted first on both channels, and the
/// `webgl2` step still ends the process with exit code `6`; every other step is
/// reported and left running (the WebView can show its own error state).
#[tauri::command]
pub fn canvas_runtime_failed(step: String, detail: String) -> Result<(), String> {
    // The step is untrusted IPC input and lands inside the marker line, so a raw
    // newline could forge a second `logcat` entry. Flatten before use.
    let step = report::flatten_step(&step);
    let marker = refusal::marker_line(&step);
    log::error!("{marker}");
    eprintln!("{marker}");

    let detail = if detail.trim().is_empty() {
        "(no detail reported)".to_string()
    } else {
        report::flatten_detail(&detail)
    };
    let diagnostic = format!("[probe] canvas runtime failed step={step} detail={detail}");
    log::error!("{diagnostic}");
    eprintln!("{diagnostic}");

    if refusal::is_webgl2_step(&step) {
        std::process::exit(refusal::WEBGL2_EXIT_CODE);
    }
    Ok(())
}

/// The one IPC handler: the shared desktop command set plus this carrier's reports.
///
/// `cad_desktop_shell::vendor::register_commands_with_vendor` composes the shared
/// registry (one list, one source of truth) with the vendor transport; the two
/// reports are local to this crate. Dispatch is by command name, so the shared handler receives every name it
/// owns unchanged and the local handler only ever sees its own two names.
pub fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    let shared = cad_desktop_shell::vendor::register_commands_with_vendor::<R>();
    let reports = report_commands::<R>();
    move |invoke| {
        if REPORT_COMMANDS.contains(&invoke.message.command()) {
            reports(invoke)
        } else {
            shared(invoke)
        }
    }
}

/// The two shell-only reports as one typed handler.
///
/// Generic over the runtime like [`cad_desktop_shell::vendor::register_commands_with_vendor`],
/// so the outer dispatch closure stays monomorphic and the shared handler's type is
/// never guessed.
fn report_commands<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![canvas_session_status, canvas_runtime_failed]
}

/// Start the Tauri application.
///
/// `#[tauri::mobile_entry_point]` generates the `start_app` symbol the generated
/// Kotlin glue calls (`gen/android`), and it needs the
/// `TAURI_ANDROID_PACKAGE_NAME_*` env vars `build.rs`/`tauri-build` exports.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The Tauri/tao loop thread owns every `!Send` CAD value. Record it before
    // `build`/`run` so the popup seam's inline-vs-marshal probe has a real answer
    // and an IPC handler can never mistake a worker thread for the owner.
    record_loop_thread();
    init_logging();
    log::info!(
        "app-android: native-below carrier starting (surfaceMode={} documentOwner={}; the native \
         CadSession owns the document, the SurfaceView below the WebView owns the pixels)",
        policy::SURFACE_MODE,
        policy::DOCUMENT_OWNER
    );

    // No `--open` on Android: the process is started by the Activity manager, so the
    // launch options are the defaults (no drawing, no explicit resource dirs) plus
    // the evidence-only probe switch. The session starts from the shared host core's
    // built-in document (exactly like a desktop host launched without `--open`); a
    // user document arrives later through the SAF picker (`OPEN` or an external VIEW
    // intent), so no opened-file identity or drawing bytes are invented at startup.
    let launch = cad_ui_core::launch::LaunchOptions {
        probe_only: std::env::var_os("YACR_PROBE_ONLY").is_some(),
        ..cad_ui_core::launch::LaunchOptions::default()
    };
    let shared = cad_desktop_shell::state::new_shared(launch);
    // This carrier is always `native-below`, so it serves the plugin manager and
    // registers the five commands through `register_commands_with_vendor`. The
    // lazy store resolves `filesDir` after the `nativeAssetsReady` handshake; the
    // vendor pass is enabled only when the persisted policy selects a decoder.
    cad_desktop_shell::plugins::configure_startup(
        &shared,
        Some(Arc::new(crate::plugin_store::AndroidPluginStore)),
        true,
    );
    let telemetry = new_render_telemetry();

    let setup_shared = shared.clone();
    let setup_telemetry = telemetry.clone();
    let run_shared = shared.clone();
    let run_telemetry = telemetry.clone();

    let app = tauri::Builder::default()
        .manage(cad_desktop_shell::state::ProbeShared(shared.clone()))
        .invoke_handler(invoke_handler())
        .setup(move |app| {
            // The single window/webview is created by Tauri from `tauri.conf.json`
            // before `setup` runs. Its handle is what the shared event sink emits
            // through and what the DOM layout's scale factor is read from.
            match tauri::Manager::get_window(app, MAIN_WINDOW_LABEL) {
                Some(window) => lock(&setup_shared).window = Some(window),
                None => log::error!(
                    "the '{}' window is missing: status events cannot be pushed and the layout \
                     scale factor falls back to 1.0",
                    MAIN_WINDOW_LABEL
                ),
            }
            register_seams(app.handle(), &setup_shared, &setup_telemetry);
            install_cad_env(&setup_shared);
            let wake_pending = lock_telemetry(&setup_telemetry).wake_pending.clone();
            let pump = pump::RedrawPump::spawn(app.handle(), wake_pending);
            lock_telemetry(&setup_telemetry).redraw_pump = Some(pump);
            log::info!("app-android: setup complete (native-below)");
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            log::error!("app-android: failed to build the Tauri app: {error}");
            std::process::exit(1);
        });

    // Single event loop: the host is ticked on `MainEventsCleared` (see the module
    // docs for why that is delivered on Android). The surface is explicitly released
    // before teardown (`Destroyed` arrives too late, with no tick afterwards).
    app.run(move |_app_handle, event| match event {
        tauri::RunEvent::MainEventsCleared => crate::host::tick::tick(&run_shared, &run_telemetry),
        tauri::RunEvent::ExitRequested { .. } => {
            log::info!("app-android: run event ExitRequested");
            cad_desktop_shell::popup::on_process_exit(&run_shared);
            crate::surface::release();
            request_stop(&run_shared, &run_telemetry);
        }
        tauri::RunEvent::Exit => {
            log::info!("app-android: run event Exit");
            cad_desktop_shell::popup::on_process_exit(&run_shared);
            crate::surface::release();
            request_stop(&run_shared, &run_telemetry);
        }
        _ => {}
    });
}
