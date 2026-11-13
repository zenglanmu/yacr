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
use cad_ui_core::session::events::EventBus;
use cad_ui_core::session::{
    CadFonts, CadLayerWrite, CadLayers, CadRecentFiles, CadSettingKey, CadSettingValue,
    CadSettings, CadStatus, OrbitDelta, QueueFamily, QueueReads, QueueSink,
};
use cad_ui_core::shell::{modal_gate_open, ShellState};

/// Re-exported so a host's tick can name the sample it drains from
/// [`Shared::cad_pinches`] without depending on the wire module's internals.
pub use cad_ui_core::session::NativePinch;

/// The bound on [`Shared::cad_pinches`].
///
/// A trackpad pinch sample is small and the owner tick drains the queue every
/// frame, so overflow means the tick is not running (GPU init, a paused window,
/// a carrier with no native session). The producer then keeps the **newest**
/// samples and reports the drop once, instead of growing without bound or
/// silently merging samples (their incremental `scale` cannot be summed).
pub const NATIVE_PINCH_CAP: usize = 64;

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

/// The platform side of the recent-drawings commands.
///
/// Recent drawings are a **host file capability** (the durable list and the
/// reopen gesture), not CAD document data: the `CadSession` is `!Send` and owned
/// by the owner thread, so the shared command layer cannot read it directly. A
/// host installs one seam that knows how to marshal the two calls onto its
/// session thread; a host that installs none refuses by name
/// (`unsupported-runtime: cad_recent_files`), never with an empty list that
/// would look like "no recent drawings".
///
/// Implementations must not hold their own lock while waiting on the session
/// thread and must not lock [`Shared`] while holding a lock of their own; the
/// commands clone the seam out of `Shared` and release the lock before calling.
pub trait ShellRecentHost: Send + Sync {
    /// The real durable list, newest first (possibly empty).
    fn list(&self) -> Result<CadRecentFiles, String>;

    /// Start reopening one recorded entry by its opaque handle. `display_name`
    /// is the caller's row label; the host revalidates the handle against its
    /// own list and uses the stored name.
    fn open_recent(&self, opaque: &str, display_name: &str) -> Result<(), String>;
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
    /// The revisioned cad-event bus (semantic events + the per-frame
    /// `status-changed`). Transport sinks are registered lazily from
    /// [`crate::events`] once [`Self::window`] exists.
    pub events: EventBus,
    /// The previous flush's projected status, the baseline for
    /// [`crate::events`]'s semantic diff.
    pub events_prev: Option<CadStatus>,
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
    /// Bounded main-thread queue of typed orbit deltas (the ViewCube's
    /// `cad_orbit { yaw, pitch }`): admitted by `cad_orbit` and drained into
    /// `CadSession::orbit`.
    pub cad_orbits: VecDeque<OrbitDelta>,
    /// Bounded main-thread queue of **native** trackpad pinch samples
    /// ([`NativePinch`]): pushed by the carrier's own toolkit gesture (Linux:
    /// `gtk_bridge::pinch`) and drained by the owner tick, which converts them
    /// with that tick's own CSS↔toolkit mapping and appends them to the same
    /// ordered input batch as the DOM inputs.
    ///
    /// Bounded at [`NATIVE_PINCH_CAP`]; the producer drops the oldest sample and
    /// logs once when the queue overflows (a sample is never merged into
    /// another — the incremental `scale` semantics do not survive summing).
    pub cad_pinches: VecDeque<NativePinch>,
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
    /// Latest read-only font-registry report mirrored from the owner-thread
    /// session, so the worker-thread `cad_fonts` read never touches the `!Send`
    /// session. `None` before the first tick (the read refuses with
    /// `cad_waiting`); `Some(Err(..))` carries the session's own projection
    /// refusal rather than an empty registry.
    pub cad_fonts: Option<Result<CadFonts, String>>,
    /// Latest reflected plugin-API snapshot mirrored from the owner-thread
    /// session, so `cad_api_call` never touches the `!Send` session from the IPC
    /// thread.
    ///
    /// `None` means **no snapshot yet** (before the first owner tick): the read
    /// answers `cad_waiting`. `Some(CadApiSnapshot { document: None })` is the
    /// real "a session exists but no document is open" state, answered
    /// `{ "document": null }`. It is a **derived** snapshot of the authoritative
    /// `cad-db` document (never written back) and never a fabricated document.
    pub cad_api_snapshot: Option<cad_ui_core::session::CadApiSnapshot>,
    /// Bounded main-thread queue of plugin-API **mutation** requests
    /// (`cad_api_submit`). Admitted by the IPC command and drained by the owner
    /// tick into `CadSession::api_submit` — the session is `!Send`, so the IPC
    /// thread never applies a mutation.
    ///
    /// Empty is the state, not `None`: nothing queued. The bound is
    /// `cad_ui_core::session::CAD_API_SUBMIT_CAP`, enforced at admission.
    pub cad_api_calls: std::collections::VecDeque<cad_ui_core::session::CadApiSubmission>,
    /// Completed plugin-API mutation outcomes waiting for `cad_api_poll`, oldest
    /// first. Bounded by `CAD_API_RESULT_CAP` (the oldest is dropped), never a
    /// second authority: every entry is the answer the session produced.
    pub cad_api_results: Vec<cad_ui_core::session::CadApiOutcome>,
    /// The app-side render telemetry seam (see [`ShellTelemetry`]). `None`
    /// before the host registers it and in unit tests; `cad_status` then reports
    /// the same `"-"` token a host with no pulled state would.
    pub telemetry: Option<Arc<dyn ShellTelemetry>>,
    /// The platform popup seam (see [`ShellPopupHost`]). `None` before the host
    /// registers it; the popup commands then refuse explicitly instead of
    /// pretending a window exists.
    pub popup_host: Option<Arc<dyn ShellPopupHost>>,
    /// The platform release-page seam (see
    /// [`crate::release::ShellReleaseOpener`]). `None` before the host registers
    /// one, and permanently on a carrier with no browser (Android): the command
    /// then refuses with its stable `unsupported-runtime` marker instead of a
    /// fabricated acknowledgement.
    pub release_opener: Option<Arc<dyn crate::release::ShellReleaseOpener>>,
    /// The platform recent-drawings seam (see [`ShellRecentHost`]). `None`
    /// before the host registers one; the two commands then refuse by name
    /// (`unsupported-runtime: cad_recent_files`) instead of answering an empty
    /// list that would look like "nothing was ever opened".
    pub recent_host: Option<Arc<dyn ShellRecentHost>>,
    /// The WebView vendor-decoding pass (pending → records → apply). Host
    /// transport state, not document state: it carries a token and bounded
    /// diagnostics, never geometry or a second `cad-db`.
    pub vendor: crate::vendor::VendorStore,
    /// A settled derived-representation snapshot waiting for the owner tick to
    /// install it on the render session. Display-only; never written to `cad-db`.
    pub vendor_pending_representations: Option<crate::vendor::PendingVendorRepresentation>,
    /// Set by the next `set_vendor_representations(None)` the tick must apply
    /// (a new document has no derived geometry yet).
    pub vendor_representations_clear: bool,
    /// Durable plugin registry (policy + installed bundles). `None` before the
    /// host wires it, when the host has no config directory, or on a carrier
    /// that does not serve the plugin manager.
    pub plugin_store: Option<Arc<dyn cad_platform::plugins::PluginStore>>,
    /// Whether this **carrier** serves the plugin manager (`native-below` only).
    pub plugins_supported: bool,
    /// The decoder enabled when this process launched. Mutations persist the
    /// policy but never change this; the status compares the two.
    pub plugins_session_active: Option<String>,
    /// The owner-tick performance sampler behind `cad_status.performance`.
    ///
    /// Sampled at most once per second on the owner tick; the concrete host
    /// fills the GPU half through [`crate::performance::PerformanceState::record_gpu`].
    /// A carrier that never calls it leaves those fields `None`.
    pub performance: crate::performance::PerformanceState,
}

impl Shared {
    /// Create the shared shell state with the parsed launch options.
    pub fn new(launch: LaunchOptions) -> Self {
        let cad_mode = !launch.probe_only;
        Self {
            shell: ShellState::new(cad_mode),
            window: None,
            events: EventBus::new(),
            events_prev: None,
            layout: LayoutTracker::new(),
            layout_revalidate: false,
            layout_bounds: None,
            launch,
            cad_tool_parameters: Vec::new(),
            cad_tool_texts: Vec::new(),
            cad_selected_texts: Vec::new(),
            cad_selected_properties: Vec::new(),
            cad_standard_views: VecDeque::new(),
            cad_orbits: VecDeque::new(),
            cad_pinches: VecDeque::new(),
            cad_setting_writes: VecDeque::new(),
            cad_settings: None,
            cad_layer_writes: VecDeque::new(),
            cad_layers: None,
            cad_fonts: None,
            cad_api_snapshot: None,
            cad_api_calls: VecDeque::new(),
            cad_api_results: Vec::new(),
            telemetry: None,
            popup_host: None,
            release_opener: None,
            recent_host: None,
            vendor: crate::vendor::VendorStore::default(),
            vendor_pending_representations: None,
            vendor_representations_clear: false,
            plugin_store: None,
            plugins_supported: false,
            plugins_session_active: None,
            performance: Default::default(),
        }
    }

    /// Queue one native (toolkit) pinch sample and report whether the oldest was
    /// dropped to make room.
    ///
    /// The caller logs the drop once per distinct state; this method stays a pure
    /// queue operation so it can be unit-tested without a toolkit.
    pub fn push_native_pinch(&mut self, pinch: NativePinch) -> bool {
        let dropped = self.cad_pinches.len() >= NATIVE_PINCH_CAP;
        if dropped {
            self.cad_pinches.pop_front();
        }
        self.cad_pinches.push_back(pinch);
        dropped
    }

    /// The **queue sink** over this host's own bounded queues.
    ///
    /// The IPC commands hand this to [`cad_ui_core::session::wire`], which applies
    /// the shared gate and then pushes here. The gate inputs (`cad_mode`, the
    /// modal popup gate) are read **once**, before the queues are borrowed — this
    /// is the only place the desktop reads them, exactly like the per-command
    /// bodies did before the shared wire layer existed.
    pub fn queue_sink(&mut self) -> QueueSink<'_> {
        let cad_mode = self.shell.cad_mode;
        let modal_open = modal_gate_open(&self.shell);
        QueueSink::new(
            cad_mode,
            modal_open,
            QueueFamily {
                commands: &mut self.shell.cad_commands,
                tool_parameters: &mut self.cad_tool_parameters,
                tool_texts: &mut self.cad_tool_texts,
                selected_texts: &mut self.cad_selected_texts,
                selected_properties: &mut self.cad_selected_properties,
                standard_views: &mut self.cad_standard_views,
                orbits: &mut self.cad_orbits,
                setting_writes: &mut self.cad_setting_writes,
                layer_writes: &mut self.cad_layer_writes,
                api_calls: &mut self.cad_api_calls,
                api_results: &mut self.cad_api_results,
            },
        )
        .with_runtime_aliases(&self.shell.command_aliases)
    }

    /// The [`cad_ui_core::session::ControlReads`] the desktop serves.
    ///
    /// Every answer is the **owner-tick mirror**: the session is `!Send`, so the
    /// IPC thread never reads it. `None` before the first tick (or in probe mode)
    /// is the explicit `cad_waiting` answer, never a fabricated default.
    pub fn queue_reads(&self) -> QueueReads<'_> {
        QueueReads::new(
            self.cad_settings.as_ref(),
            self.cad_layers.as_ref(),
            self.cad_fonts.as_ref(),
            self.cad_api_snapshot.as_ref(),
        )
    }
}

/// The owner tick's plugin-API drain: answer queued submissions, publish every
/// completed prompt, then append both to the shared result queue.
///
/// The submission queue is taken under the lock (exactly like the other
/// owner-tick drains) and applied **outside** it on `session` — the `!Send`
/// owner-thread session. A submission that starts an `Editor` prompt has no
/// answer yet (the prompt's answer arrives on a later tick through
/// `CadSession::api_prompt_results`). A host with no session answers
/// `cad_waiting` for every submission instead of dropping it.
pub fn drain_api_calls(
    shared: &Arc<Mutex<Shared>>,
    session: Option<&cad_ui_core::session::CadSession>,
) {
    let calls: Vec<cad_ui_core::session::CadApiSubmission> = {
        let mut state = lock(shared);
        std::mem::take(&mut state.cad_api_calls)
            .into_iter()
            .collect()
    };
    let mut outcomes: Vec<cad_ui_core::session::CadApiOutcome> = Vec::new();
    for call in calls {
        if let Some(outcome) = cad_ui_core::session::api_submit_or_wait(session, call) {
            outcomes.push(outcome);
        }
    }
    if let Some(session) = session {
        outcomes.extend(session.api_prompt_results());
    }
    if outcomes.is_empty() {
        return;
    }
    let mut state = lock(shared);
    for outcome in outcomes {
        cad_ui_core::session::push_api_result(&mut state.cad_api_results, outcome);
    }
}

/// Managed Tauri state: a shared handle to [`Shared`].
pub struct ProbeShared(pub Arc<Mutex<Shared>>);

/// Create the shared probe state with the parsed launch options.
pub fn new_shared(launch: LaunchOptions) -> Arc<Mutex<Shared>> {
    Arc::new(Mutex::new(Shared::new(launch)))
}

/// Install the release-page platform seam (see
/// [`crate::release::ShellReleaseOpener`]).
///
/// Mirrors how the hosts install `telemetry`/`popup_host`: a host with a URL
/// opener calls this once during startup; a carrier without one simply never
/// calls it and the command keeps its explicit `unsupported-runtime` refusal.
pub fn configure_release_opener(
    shared: &Arc<Mutex<Shared>>,
    opener: Arc<dyn crate::release::ShellReleaseOpener>,
) {
    lock(shared).release_opener = Some(opener);
}

/// Install the recent-drawings platform seam (see [`ShellRecentHost`]).
///
/// Mirrors `configure_release_opener`: a host with a native session that owns a
/// durable recent list calls this once during startup; a host that never does
/// keeps the named refusal instead of an empty supported list.
pub fn configure_recent_host(shared: &Arc<Mutex<Shared>>, host: Arc<dyn ShellRecentHost>) {
    lock(shared).recent_host = Some(host);
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

#[cfg(test)]
mod pinch_queue_tests {
    use super::*;
    use cad_ui_core::launch::LaunchOptions;

    fn shared() -> Shared {
        Shared::new(LaunchOptions::default())
    }

    #[test]
    fn the_pinch_queue_starts_empty_and_keeps_every_sample_in_order() {
        let mut shared = shared();
        assert!(shared.cad_pinches.is_empty());
        for scale in [1.05, 1.10, 0.95] {
            assert!(
                !shared.push_native_pinch(NativePinch {
                    x: 10.0,
                    y: 20.0,
                    scale,
                }),
                "no drop while the queue is under its bound"
            );
        }
        let scales: Vec<f64> = shared.cad_pinches.iter().map(|pinch| pinch.scale).collect();
        assert_eq!(scales, vec![1.05, 1.10, 0.95], "FIFO, never reordered");
    }

    #[test]
    fn the_pinch_queue_drops_the_oldest_sample_and_reports_it() {
        let mut shared = shared();
        for index in 0..NATIVE_PINCH_CAP {
            assert!(!shared.push_native_pinch(NativePinch {
                x: index as f64,
                y: 0.0,
                scale: 1.0,
            }));
        }
        assert_eq!(shared.cad_pinches.len(), NATIVE_PINCH_CAP);
        assert!(
            shared.push_native_pinch(NativePinch {
                x: -1.0,
                y: 0.0,
                scale: 1.0,
            }),
            "the producer is told the oldest sample was dropped"
        );
        assert_eq!(shared.cad_pinches.len(), NATIVE_PINCH_CAP, "still bounded");
        assert_eq!(
            shared.cad_pinches.front().map(|pinch| pinch.x),
            Some(1.0),
            "the oldest sample left, the newest stayed"
        );
        assert_eq!(
            shared.cad_pinches.back().map(|pinch| pinch.x),
            Some(-1.0),
            "the newest sample is the one just pushed"
        );
    }
}
