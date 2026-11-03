//! Tauri IPC commands shared by every desktop host.
//!
//! These are thin `#[tauri::command]` wrappers. The **semantics** — wire argument
//! parsing, the modal/probe-mode gates, the queue bounds, every refusal string and
//! the `{accepted:true}` acknowledgement — live in `cad_ui_core::session::wire`
//! (one function per command) on top of `cad_ui_core::session::admission` and
//! `cad_ui_core::shell`; this module only moves the Tauri arguments into those
//! functions and hands them [`Shared::queue_sink`] (the bounded-queue sink) or
//! [`Shared::queue_reads`] (the owner-tick mirror). The same `wire::*` functions
//! are what the `dom-canvas` (direct wasm) host calls, so the two hosts cannot
//! answer differently for the same request.
//!
//! The renderer telemetry that `cad_status`/`probe_input` used to read directly
//! is behind [`ShellTelemetry`]; no `cad-render-wgpu`/`cad-engine` type is named
//! here.
//!
//! ## The queued shape
//!
//! `CadSession` is `!Send` and owned by the host main thread, so nothing here
//! touches it: an admitted request is pushed onto a bounded queue and the owner
//! tick applies it. `{accepted:true}` therefore means **queued**, never
//! completed; the real outcome (including a session refusal) surfaces through
//! `cad_status.error`. The reads (`cad_settings`, `cad_layers`, `cad_fonts`)
//! answer from the tick's mirror for the same reason.
//!
//! ## Registration
//!
//! [`register_commands`] is the single `generate_handler!` list and
//! [`COMMAND_NAMES`] is the same set, both generated from
//! `cad_ui_core::session::registry`. The three desktop-only commands
//! (`probe_status`, `probe_input`, `cad_status`) stay handwritten because their
//! bodies read host state that the shared registry deliberately does not name.

use std::sync::{Arc, Mutex};

use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{wire, CadFonts, CadLayers, CadSettings, HostProfile, WireStatus};
use cad_ui_core::shell::{modal_gate_open, queue_cad_input, POPUP_MODAL_OPEN};

use crate::popup::DesktopPopupTarget;
use crate::state::{lock, ProbeShared, Shared};

// Re-exported for the evidence driver, which must queue through the **same**
// selected-text admission `cad_set_selected_text` runs.
pub use cad_ui_core::session::admit_selected_text;
// Re-exported for the owner tick, which drains the queue with the shared rule.
pub use cad_ui_core::session::drain_standard_views;
// Re-exported for the owner tick, which drains the layer-write queue with the
// shared rule (the React layer panel's three mutations).
pub use cad_ui_core::session::drain_layer_writes;
// The wire shapes moved into `cad_ui_core::session::wire` so both hosts answer
// the identical JSON; these aliases keep every existing import path working.
pub use cad_ui_core::session::wire::{parse_setting_request, CadCommandAck, CadCommandAlias};

// ---------------------------------------------------------------------------
// Generated transport: the shared registry rows become the typed wrappers,
// the `COMMAND_NAMES` constant and the one `generate_handler!` list.
// ---------------------------------------------------------------------------

/// Emit one typed `#[tauri::command]` wrapper per shared registry row.
macro_rules! tauri_wire_wrappers {
    ($sink:ident, $reads:ident, $popups:ident; $($rows:tt)*) => {
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rows)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;) => {};
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, sink, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let mut state = lock(&shared.0);
            let mut queue = state.queue_sink();
            let $sink = &mut queue;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, reads, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let state = lock(&shared.0);
            let queue_reads = state.queue_reads();
            let $reads = &queue_reads;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, popups, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            let mut host_popups = DesktopPopupTarget::new(&shared.0)?;
            let $popups = &mut host_popups;
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row(shared, pure, $args:ident, $fn_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* })
        $($rest:tt)*) => {
        #[tauri::command]
        pub fn $fn_name(
            _shared: tauri::State<'_, ProbeShared>
            $(, $field: $ty)*
        ) -> Result<$reply, String> {
            $($body)*
        }
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
    (@munch $sink:ident $reads:ident $popups:ident;
        @row($($other:tt)*) $($rest:tt)*) => {
        tauri_wire_wrappers!(@munch $sink $reads $popups; $($rest)*);
    };
}

/// Emit `COMMAND_NAMES` from every registry row, in registration order.
macro_rules! tauri_wire_command_names {
    (@munch [$($names:tt)*]) => {
        /// The registered command set, in the order `generate_handler!` lists it.
        ///
        /// Both this constant and the handler list are generated from
        /// `cad_ui_core::session::registry`; the `registration` test pins the set
        /// against the pre-migration fixture.
        pub const COMMAND_NAMES: &[&str] = &[$($names)*];
    };
    (@munch [$($names:tt)*] @row($kind:ident $($extra:ident)*, $lane:ident, $args:ident, $row_name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        tauri_wire_command_names!(@munch [$($names)* stringify!($row_name),] $($rest)*);
    };
    ($($rows:tt)*) => {
        tauri_wire_command_names!(@munch [] $($rows)*);
    };
}

/// Emit the one `generate_handler!` list from every registry row.
macro_rules! tauri_wire_register {
    (@munch [$($names:ident,)*]) => {
        /// Build the `invoke_handler` for the desktop hosts.
        ///
        /// This is the only `generate_handler!` invocation; the list and the
        /// typed wrappers are both generated from the shared registry.
        pub fn register_commands<R: tauri::Runtime>(
        ) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($names),*]
        }
    };
    (@munch [$($names:ident,)*] @row($kind:ident $($extra:ident)*, $lane:ident, $args:ident, $row_name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        tauri_wire_register!(@munch [$($names,)* $row_name,] $($rest)*);
    };
    ($($rows:tt)*) => {
        tauri_wire_register!(@munch [] $($rows)*);
    };
}

/// The desktop emitter over the shared registry.
macro_rules! emit_tauri_wire_registry {
    (@context($sink:ident, $reads:ident, $popups:ident, $name:ident, $args_json:ident) $($rows:tt)*) => {
        tauri_wire_wrappers!($sink, $reads, $popups; $($rows)*);
        tauri_wire_command_names!($($rows)*);
        tauri_wire_register!($($rows)*);
    };
}

// ---------------------------------------------------------------------------
// Desktop-only commands: status and probe use host state the registry does not
// name, so their wrappers (and bodies) are handwritten.
// ---------------------------------------------------------------------------

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    probe_status_impl(&shared.0)
}

/// Command body of [`probe_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn probe_status_impl(shared: &Arc<Mutex<Shared>>) -> ProbeStatus {
    lock(shared).shell.status.clone()
}

/// Command body of `cad_settings`, without the Tauri state wrapper.
///
/// Split out so the wire-shape tests read the **same** snapshot the IPC path
/// serves (no duplicated projection in tests).
pub fn cad_settings_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadSettings, String> {
    wire::settings(&lock(shared).queue_reads())
}

/// Command body of `cad_set_setting`, without the Tauri state wrapper.
///
/// Split out so the wire-shape tests drive the **same** parse-and-admit path the
/// IPC command runs (never a re-implemented copy in the test).
pub fn cad_set_setting_impl(
    shared: &Arc<Mutex<Shared>>,
    key: &str,
    value: &serde_json::Value,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::set_setting(&mut state.queue_sink(), key, value)
}

/// Command body of `cad_layers`, without the Tauri state wrapper.
///
/// Split out so the wire-shape/golden tests can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_layers_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadLayers, String> {
    wire::layers(&lock(shared).queue_reads())
}

/// Command body of `cad_fonts`, without the Tauri state wrapper.
///
/// Split out so the wire test can call the **same** implementation the IPC path
/// runs; the answer is the owner-tick mirror, never a read of the `!Send`
/// session.
pub fn cad_fonts_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadFonts, String> {
    wire::fonts(&lock(shared).queue_reads())
}

/// Command body of `cad_create_layer`, without the Tauri state wrapper.
pub fn cad_create_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::create_layer(&mut state.queue_sink(), &name)
}

/// Command body of `cad_rename_layer`, without the Tauri state wrapper.
pub fn cad_rename_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::rename_layer(&mut state.queue_sink(), &layer, &name)
}

/// Command body of `cad_delete_layer`, without the Tauri state wrapper.
pub fn cad_delete_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::delete_layer(&mut state.queue_sink(), &layer)
}

/// Command body of `cad_set_layer_property`, without the Tauri state wrapper.
pub fn cad_set_layer_property_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::set_layer_property(&mut state.queue_sink(), &layer, &key, &value)
}

/// Command body of `cad_command_aliases`, without the Tauri command wrapper.
pub fn cad_command_aliases_impl() -> Vec<CadCommandAlias> {
    wire::aliases()
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in and the carrier described by one typed
/// `hostProfile` object.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
///
/// The envelope is [`WireStatus`]: the frozen [`cad_ui_core::session::CadStatus`]
/// fields (unchanged, same order) followed by the always-present `hostProfile`.
/// On this carrier the profile is [`HostProfile::native_below`] (native document
/// owner, palettes served, standard views undeclared) and there is no
/// `canvasSession` key. The old "carrier signalled by the presence/absence of
/// `surfaceMode`" story is dead: every carrier now names itself.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> WireStatus<'static> {
    cad_status_wire_impl(&shared.0)
}

/// Command body of [`cad_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_status_impl(shared: &Arc<Mutex<Shared>>) -> cad_ui_core::session::CadStatus {
    // One projection for both paths: the host error and the live popup are
    // applied by `events::project_current_status`, the same function the per-tick
    // event flush calls, so `cad_status` and `status-changed` cannot diverge.
    let (status, telemetry) = {
        let state = lock(shared);
        (
            crate::events::project_current_status(&state),
            state.telemetry.clone(),
        )
    };
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    // The pulled organisation state is appended as a separate token (the JSON is
    // unchanged and stays first) so a skipped accelerated organisation is visible
    // next to the status payload.
    //
    // Lock order: the shell guard is released before the telemetry seam runs, so
    // the app-side implementation only ever locks the telemetry mutex (it never
    // re-enters `Shared`).
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        let pulled = telemetry
            .as_ref()
            .map(|telemetry| telemetry.cad_status_evidence())
            .unwrap_or_else(|| "-".to_string());
        match serde_json::to_string(&status) {
            Ok(json) => eprintln!("[probe] cad_status {json} pulled={pulled}"),
            Err(error) => eprintln!("[probe] cad_status serialize failed: {error} pulled={pulled}"),
        }
    }
    status
}

/// The exact `cad_status` wire object: the shared projection plus this carrier's
/// typed [`HostProfile`], with no `canvasSession` (native sessions report their
/// document through the shared snapshot itself).
///
/// Split out so the wire goldens serialize the **same** envelope the IPC command
/// returns; a native host cannot return references to its locally built values,
/// so the envelope owns them ([`WireStatus::owned`]).
pub fn cad_status_wire_impl(shared: &Arc<Mutex<Shared>>) -> WireStatus<'static> {
    WireStatus::owned(
        cad_status_impl(shared),
        HostProfile::native_below(),
        None::<serde_json::Value>,
    )
}

/// `probe_input` command: sequence-gate one frontend input and route it.
///
/// In CAD mode a pointer/wheel input is queued for the main thread only when the
/// CAD session is ready; before that it is refused with an explicit `cad_waiting`
/// reason (never a pretend success), and a full queue is refused explicitly.
/// `resize` is applied by the tick through the CAD rect path.
#[tauri::command]
pub fn probe_input(shared: tauri::State<'_, ProbeShared>, input: ProbeInput) -> ProbeInputAck {
    probe_input_impl(&shared.0, input)
}

/// Whether the diagnostic input line is enabled.
///
/// Shares the host's existing evidence gate (`YACR_PROBE_CAD_LOG`) rather than
/// adding a second switch: evidence drivers already set it, and normal sessions
/// keep their stderr free of one line per pointer move.
fn input_trace_enabled() -> bool {
    std::env::var_os("YACR_PROBE_CAD_LOG").is_some()
}

/// Command body of [`probe_input`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated gate/queue logic in tests).
pub fn probe_input_impl(shared: &Arc<Mutex<Shared>>, input: ProbeInput) -> ProbeInputAck {
    let kind = input.kind;
    let sequence = input.sequence;
    // Modal popup gate (ADR-0021 D2): while a popup is open, every interaction
    // input is refused by name before the tracker sees it. `resize` still passes
    // because it carries no CAD intent and keeps the layout truthful while the
    // modal is up. This gate is host-owned, so the command line's blur path can
    // never release it.
    if !matches!(kind, InputKind::Resize) {
        let refused = {
            let state = lock(shared);
            modal_gate_open(&state.shell).then(|| state.shell.input.marker_offset())
        };
        if let Some(marker_offset) = refused {
            if input_trace_enabled() {
                eprintln!(
                    "[probe] input kind={} seq={sequence} accepted=false reason={POPUP_MODAL_OPEN}",
                    kind.as_str()
                );
            }
            return ProbeInputAck {
                accepted: false,
                sequence,
                marker_offset,
                terminal: kind.is_terminal(),
                reason: Some(POPUP_MODAL_OPEN.to_string()),
            };
        }
    }
    let ack = lock(shared).shell.input.apply(&input);
    // Evidence line: shows the real IPC path (invoke -> command -> tracker).
    // x/y/dx/dy/width are logged so the accepted input can be tied to real OS
    // coordinates (diagnosis only; identifiers stay English). Gated exactly like
    // the other probe evidence: a pointer move arrives at up to display rate, so
    // an always-on `eprintln!` is both log spam and avoidable main/IPC-thread work
    // in normal use. Evidence runs already set `YACR_PROBE_CAD_LOG=1`.
    if input_trace_enabled() {
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
    }
    if !ack.accepted {
        return ack;
    }
    // Post-load pan driver accounting: count host-admitted gesture inputs, and
    // flag a concurrent resize (which would make any view change unattributable
    // to the pan). The app-side implementation locks only the telemetry mutex.
    let telemetry = lock(shared).telemetry.clone();
    if let Some(telemetry) = telemetry {
        telemetry.note_input_accepted(kind);
    }
    if matches!(kind, InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(shared);
        queue_cad_input(&mut state.shell, &input)
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

// The shared wrappers, `COMMAND_NAMES` and `register_commands` are generated
// from the one registry table. This is the only desktop emitter invocation.
cad_ui_core::cad_wire_registry!(emit_tauri_wire_registry);
