//! Shared shell state and the platform seams a desktop host plugs into.
//!
//! This crate owns the **toolkit-level but platform-neutral** half of the old
//! `apps/app-linux/src/host/mod.rs` `Shared` struct: everything the IPC commands
//! need (CAD admission state, the input tracker, the bounded queue family, the
//! live window handle and the DOM layout tracker). Renderer telemetry (frame
//! timings, pull state, redraw pump, post-load driver) stays in the concrete app
//! (`app-linux::host::RenderTelemetry`) because it names `cad-render-wgpu`/
//! `cad-engine`, which are forbidden here.
//!
//! # Lock-order invariant (frozen)
//!
//! After the split a desktop host owns **two** mutexes:
//!
//! 1. [`Shared`] (`Arc<Mutex<Shared>>`, managed as [`ProbeShared`]), and
//! 2. the app-side render telemetry (`Arc<Mutex<RenderTelemetry>>`).
//!
//! The acquisition order is **always `Shared` -> render telemetry, never the
//! reverse**. A [`ShellTelemetry`] implementation must **never** touch
//! [`Shared`] (it only reads/writes the app telemetry), which makes the reverse
//! order impossible by construction. Code that needs both acquires `Shared`
//! first, drops it, then calls the seam — or holds it while calling a seam that
//! never re-enters `Shared`. The wire output is equivalent to the pre-split
//! host (`probe_input`/`cad_status`/`probe_status` goldens); the concurrency
//! claim is **not** "unchanged", it is "equivalent under this lock order".

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use cad_app::{StandardView, ToolParameter, ToolValue};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::layout::LayoutTracker;
use cad_ui_core::popup::PopupAck;
use cad_ui_core::protocol::InputKind;
use cad_ui_core::session::{CadLayerWrite, CadLayers, CadSettingKey, CadSettingValue, CadSettings};
use cad_ui_core::shell::ShellState;

/// The app-side render telemetry, exposed to the command layer through a narrow
/// object-safe seam.
///
/// Only plain values cross this boundary (a string token and an input kind);
/// the shell never names `cad-render-wgpu`/`cad-engine` types. Implementations
/// live in the concrete app and **must never lock [`Shared`]**: that is what
/// makes the `Shared -> telemetry` lock order the only possible one.
pub trait ShellTelemetry: Send + Sync {
    /// The `pulled=` evidence token appended to the env-gated `cad_status` log
    /// line (`"-"` when no pulled organisation was observed).
    fn cad_status_evidence(&self) -> String;

    /// Account for one input the tracker accepted (the bounded post-load pan
    /// evidence driver counts admitted moves/ups and flags a concurrent resize).
    fn note_input_accepted(&self, kind: InputKind);
}

/// The platform side of the popup commands (ADR-0021 D1/D2, WP3).
///
/// The popup **state machine** and the four-trigger gate lifecycle are
/// `cad_ui_core::popup` / [`crate::popup`]. Since popups are in-DOM overlays, the
/// only platform work left is deriving the `textEdit` payload from the live CAD
/// session (a main-thread-only read), which stays in the concrete host. This
/// seam keeps `cad_open_popup`/`cad_close_popup` in the shared command layer
/// without moving that session read (or any GTK/Tauri type) into it.
///
/// Implementations must preserve the host's threading contract (the Linux
/// implementation marshals the whole open onto the GTK main thread). They must
/// not lock [`Shared`] while holding their own lock; the command clones the
/// seam out of `Shared` and releases the lock before calling it.
pub trait ShellPopupHost: Send + Sync {
    /// Open (or focus) the allowlisted popup for `kind`.
    fn open(&self, shared: &Arc<Mutex<Shared>>, kind: &str) -> Result<PopupAck, String>;

    /// Close the open popup (idempotent) and clear the modal gate.
    fn close(&self, shared: &Arc<Mutex<Shared>>) -> Result<(), String>;
}

/// State shared with the IPC commands: deliberately **GPU-free and
/// renderer-free**.
///
/// The UI-framework-free part lives in [`ShellState`] (`cad-ui-core`) under the
/// explicit `shell` field; this struct adds the Tauri window handle, the DOM
/// layout tracker and the bounded typed queue family. The halves are accessed
/// explicitly (`state.shell.status`, `state.cad_tool_parameters`, …) — there is
/// deliberately no `Deref` bridge, so a future Windows host cannot inherit one
/// by accident.
///
/// Lock order: see the module docs. Never acquire this mutex while holding the
/// app-side render telemetry.
pub struct Shared {
    /// The UI-framework-free shared state (status, input, CAD admission/errors).
    pub shell: ShellState,
    /// The main window handle the app tick polls for size/state. Kept here
    /// because the public `Manager` trait does not expose plain windows by label.
    pub window: Option<tauri::Window>,
    /// React-reported CAD layout (logical rect -> viewport insets), seq/generation
    /// guarded.
    pub layout: LayoutTracker,
    /// Set when a new GPU generation starts: the cached CAD rect must be
    /// re-validated against the current top geometry (not blindly re-applied).
    pub layout_revalidate: bool,
    /// The margin-receiving container bounds (logical px) last used for a layout
    /// decision. A change re-validates the cached DOM rect even without a new
    /// sequence (the container resized under an unchanged rect).
    pub layout_bounds: Option<(f32, f32)>,
    /// Startup/CLI options (drawing, font/image dirs, size, probe mode).
    pub launch: LaunchOptions,
    /// Bounded main-thread queue of typed tool-parameter requests (ADR 0016
    /// decision A): admitted by the `cad_set_tool_parameter` IPC command and
    /// drained by the owner tick into `CadSession::set_tool_parameter`. Typed and
    /// separate from `shell.cad_commands` (name-only) so the rejected command-line
    /// text-payload path is never reintroduced.
    pub cad_tool_parameters: Vec<(ToolParameter, ToolValue)>,
    /// Bounded main-thread queue of terminal tool texts (ADR 0019 decision A.1):
    /// admitted by the dedicated `cad_set_tool_text` IPC and drained by the owner
    /// tick into `CadSession::set_tool_text`, which completes the command
    /// immediately.
    pub cad_tool_texts: Vec<String>,
    /// Bounded main-thread queue of replacement texts for the **current single
    /// selection** (ADR 0020 amendment B): admitted by `cad_set_selected_text`
    /// and drained by the owner tick into `CadSession::set_selected_text`.
    pub cad_selected_texts: Vec<String>,
    /// Bounded main-thread queue of property writes for the **current
    /// selection** (the docked panel's `SETENTITYPROPERTY` channel): admitted by
    /// `cad_set_selected_property` and drained into
    /// `CadSession::set_selected_property`.
    pub cad_selected_properties: Vec<(String, String)>,
    /// Bounded main-thread queue of typed standard-view requests: admitted by
    /// `cad_standard_view` and drained into `CadSession::standard_view`.
    pub cad_standard_views: VecDeque<StandardView>,
    /// Bounded main-thread queue of typed settings writes: admitted by
    /// `cad_set_setting` and drained into `CadSession::apply_setting`.
    pub cad_setting_writes: VecDeque<(CadSettingKey, CadSettingValue)>,
    /// Latest effective settings snapshot mirrored from the owner-thread session,
    /// so the worker-thread `cad_settings` IPC read never touches the `!Send`
    /// session.
    pub cad_settings: Option<CadSettings>,
    /// Bounded main-thread queue of typed layer writes (the layer panel's
    /// `cad_set_layer_visible` / `cad_restore_layers` / `cad_set_active_layer`
    /// channels and its `cad_create_layer` / `cad_rename_layer` /
    /// `cad_delete_layer` table writes): admitted by those commands and drained by
    /// the owner tick into the matching `CadSession` funnel.
    pub cad_layer_writes: VecDeque<CadLayerWrite>,
    /// Latest layer projection mirrored from the owner-thread session, so the
    /// worker-thread `cad_layers` read never touches the `!Send` session.
    ///
    /// `None` before a session exists (the read refuses with `cad_waiting`);
    /// `Some(Err(..))` carries the session's own projection refusal rather than
    /// serving stale rows.
    pub cad_layers: Option<Result<CadLayers, String>>,
    /// The app-side render telemetry seam (see [`ShellTelemetry`]). `None`
    /// before the host registers it and in unit tests; `cad_status` then reports
    /// the same `"-"` token a host with no pulled state would.
    pub telemetry: Option<Arc<dyn ShellTelemetry>>,
    /// The platform popup seam (see [`ShellPopupHost`]). `None` before the host
    /// registers it; the popup commands then refuse explicitly instead of
    /// pretending a window exists.
    pub popup_host: Option<Arc<dyn ShellPopupHost>>,
}

impl Shared {
    /// Create the shared shell state with the parsed launch options.
    pub fn new(launch: LaunchOptions) -> Self {
        let cad_mode = !launch.probe_only;
        Self {
            shell: ShellState::new(cad_mode),
            window: None,
            layout: LayoutTracker::new(),
            layout_revalidate: false,
            layout_bounds: None,
            launch,
            cad_tool_parameters: Vec::new(),
            cad_tool_texts: Vec::new(),
            cad_selected_texts: Vec::new(),
            cad_selected_properties: Vec::new(),
            cad_standard_views: VecDeque::new(),
            cad_setting_writes: VecDeque::new(),
            cad_settings: None,
            cad_layer_writes: VecDeque::new(),
            cad_layers: None,
            telemetry: None,
            popup_host: None,
        }
    }
}

/// Managed Tauri state: a shared handle to [`Shared`].
pub struct ProbeShared(pub Arc<Mutex<Shared>>);

/// Create the shared probe state with the parsed launch options.
pub fn new_shared(launch: LaunchOptions) -> Arc<Mutex<Shared>> {
    Arc::new(Mutex::new(Shared::new(launch)))
}

/// Lock [`Shared`], recovering the guard from a poisoned mutex.
///
/// Lock order: callers must not hold the app-side render telemetry here (see the
/// module docs); the telemetry seam is never called with this guard held.
pub fn lock(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
