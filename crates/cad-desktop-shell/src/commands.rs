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
//! `cad_status.error`. The two reads (`cad_settings`, `cad_layers`) answer from
//! the tick's mirror for the same reason.
//!
//! ## Registration
//!
//! [`register_commands`] is the single `generate_handler!` list. [`COMMAND_NAMES`]
//! is the command set captured from the pre-migration host; the `registration`
//! test asserts the macro list and the constant are the same set.

use std::sync::{Arc, Mutex};

use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{wire, CadLayers, CadSettings};
use cad_ui_core::shell::{
    modal_gate_open, projected_popup, projected_status, queue_cad_input, POPUP_MODAL_OPEN,
};
use cad_ui_core::unsupported_status;

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

use cad_ui_core::popup::PopupAck;

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

/// `cad_command` command: admit a known CAD command into the bounded main-thread
/// queue.
///
/// An unknown command is rejected immediately (never queued). `{accepted:true}`
/// means the command was **queued**, not that it completed; the real outcome
/// surfaces through `cad_status.error`/`loading`. The gate order (modal popup
/// gate, command vocabulary, probe mode, bound) and the one argument form
/// (`ZOOM <argument>`) are [`wire::command`]'s.
#[tauri::command]
pub fn cad_command(
    shared: tauri::State<'_, ProbeShared>,
    command: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::command(&mut state.queue_sink(), &command)
}

/// `cad_set_tool_parameter` command: admit one typed tool parameter into the
/// bounded main-thread queue (ADR 0016 decision A).
///
/// This is the **single** general parameter entry point that replaces ADR 0015
/// decision C's `cad_set_polygon_sides`. It is a typed IPC, deliberately not an
/// argument added to `cad_command` (command-line text-payload parsing was rejected
/// in an earlier phase). The wire shape is `{ parameter: <key>, number?: <f64>,
/// flag?: <bool> }` with exactly one of `number`/`flag`; a malformed request (an
/// unknown key, or neither/both value fields) is refused immediately. No session
/// state is touched off the main thread: the value is queued and the owner tick
/// routes it through `CadSession::set_tool_parameter`, whose authoritative host
/// validates the parameter's range/kind and Viewer mode.
#[tauri::command]
pub fn cad_set_tool_parameter(
    shared: tauri::State<'_, ProbeShared>,
    parameter: String,
    number: Option<f64>,
    flag: Option<bool>,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_tool_parameter(&mut state.queue_sink(), &parameter, number, flag)
}

/// `cad_set_tool_text` command: admit one **terminal** text into the bounded
/// main-thread queue (ADR 0019 decision A.1).
///
/// This is the only string channel into the CAD session. It is a dedicated typed
/// IPC and deliberately **not** an argument of `cad_command`: that funnel accepts
/// a command *name* only, and parsing a text payload out of the command line was
/// rejected earlier. The wire shape is `{ text: <string> }`; the owner tick routes
/// it through `CadSession::set_tool_text`, which validates the live state and
/// completes the command immediately (text is terminal).
#[tauri::command]
pub fn cad_set_tool_text(
    shared: tauri::State<'_, ProbeShared>,
    text: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_tool_text(&mut state.queue_sink(), &text)
}

/// `cad_set_selected_text` command: admit one replacement text for the current
/// single selection into the bounded main-thread queue (ADR 0020 amendment B).
///
/// The wire shape is `{ text: <string> }` and deliberately carries **no entity
/// id**: the host resolves and validates its own current selection at apply
/// time, so a UI can never retarget another object with a stale id. The field is
/// refused with `textedit.selection_empty` / `textedit.selection_multiple` /
/// `textedit.target_not_text` when the selection is not exactly one directly
/// addressable `Text`; empty/over-long drafts are refused with the same
/// `tool.text_empty` / `tool.text_too_long` names the text step uses.
#[tauri::command]
pub fn cad_set_selected_text(
    shared: tauri::State<'_, ProbeShared>,
    text: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_selected_text(&mut state.queue_sink(), &text)
}

/// `cad_set_selected_property` command: queue one property write for every
/// entity of the current selection.
///
/// The wire shape is `{ key: <string>, value: <string> }` and deliberately
/// carries **no entity id**: the host resolves and validates its own current
/// selection at apply time, so a stale UI can never retarget another object. An
/// unknown key is refused immediately with the stable code
/// `property.unsupported_key`; a value the key's grammar (or the drawing's
/// tables) rejects is refused by the host write path and surfaces through
/// `cad_status.error` as `property.invalid_value` / `property.unknown_layer` /
/// `property.unknown_linetype` / `property.ambiguous_name`.
#[tauri::command]
pub fn cad_set_selected_property(
    shared: tauri::State<'_, ProbeShared>,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_selected_property(&mut state.queue_sink(), &key, &value)
}

/// `cad_standard_view` command: admit one typed standard view into the bounded
/// main-thread queue.
///
/// `STANDARDVIEW` maps to a payload-carrying [`cad_app::CommandId::StandardView`]
/// (`CommandPayload::StandardView`), which the name-only `cad_command` funnel
/// cannot carry: that funnel queues a normalized name and the router refuses a
/// payload-carrying variant. This dedicated typed IPC is the sanctioned channel
/// for it (the same shape as `cad_set_tool_parameter`), deliberately **not** an
/// argument added to `cad_command`. The wire shape is `{ view: <key> }` with a
/// stable key from [`cad_app::StandardView::ALL`], matched case-insensitively.
#[tauri::command]
pub fn cad_standard_view(
    shared: tauri::State<'_, ProbeShared>,
    view: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::standard_view(&mut state.queue_sink(), &view)
}

/// `cad_open_popup` command: open the allowlisted modal popup, or report the one
/// already open (ADR-0021 D1/D2, WP3).
///
/// The wire carries a **kind key only** (`{ kind: String }`): a caller can never
/// choose a label, URL or script. `"about"`, `"textEdit"` and `"options"` are
/// the allowlisted kinds; anything else is refused with `popup.unknown_kind: …`
/// before any gate change. `textEdit` derives its payload from the host's
/// current single selection (never from the caller); a selection that is not
/// exactly one editable text is refused with `popup.no_text_selection: …` and no
/// gate is set. `options` carries no payload at all: the dialog reads
/// `cad_settings` itself. While the popup is open the host refuses CAD input
/// (`popup.modal_open`), independently of the UI's own pointer suspension.
///
/// The popup is an in-DOM overlay in the single UI tree; the acknowledgement's
/// additive `payload` field carries the content that overlay needs (see
/// `cad_ui_core::protocol`). Only the main-thread payload derivation belongs to
/// the concrete host behind [`crate::state::ShellPopupHost`]; the state machine
/// and the gate are `cad_ui_core::popup`.
#[tauri::command]
pub fn cad_open_popup(
    shared: tauri::State<'_, ProbeShared>,
    kind: String,
) -> Result<PopupAck, String> {
    let mut popups = DesktopPopupTarget::new(&shared.0)?;
    wire::open_popup(&mut popups, &kind)
}

/// `cad_close_popup` command: close the open modal popup (ADR-0021 D1/D2, WP3).
///
/// Closing when nothing is open is an **idempotent success**, not an error. This
/// is the explicit-close trigger of the four-trigger lifecycle: it clears the
/// modal gate directly, exactly like the overlay's close button and Esc, because
/// the overlay and the gate belong to the same document.
#[tauri::command]
pub fn cad_close_popup(shared: tauri::State<'_, ProbeShared>) -> Result<CadCommandAck, String> {
    let mut popups = DesktopPopupTarget::new(&shared.0)?;
    wire::close_popup(&mut popups)
}

/// `cad_settings` command: the latest effective settings snapshot.
///
/// The owner-thread session is `!Send`, so the tick mirrors its
/// `CadSession::settings()` snapshot into the shared state exactly like
/// `cad_status`. Before a session exists (or in probe mode) the read is an
/// explicit `cad_waiting`, never a fabricated default object.
#[tauri::command]
pub fn cad_settings(shared: tauri::State<'_, ProbeShared>) -> Result<CadSettings, String> {
    cad_settings_impl(&shared.0)
}

/// Command body of [`cad_settings`], without the Tauri state wrapper.
///
/// Split out so the wire-shape tests read the **same** snapshot the IPC path
/// serves (no duplicated projection in tests).
pub fn cad_settings_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadSettings, String> {
    wire::settings(&lock(shared).queue_reads())
}

/// `cad_set_setting` command: admit one typed setting for the owner thread.
///
/// `{accepted:true}` means **queued**, not completed: the tick routes it through
/// `CadSession::apply_setting`, which persists the preference leaf and applies
/// the change to the live scene/session. A malformed request (unknown key, wrong
/// type, unsupported plot format) is refused immediately with a stable code and
/// never reaches the queue. This path is intentionally **not** behind the modal
/// popup gate, so the Options dialog can write while it is open.
#[tauri::command]
pub fn cad_set_setting(
    shared: tauri::State<'_, ProbeShared>,
    key: String,
    value: serde_json::Value,
) -> Result<CadCommandAck, String> {
    cad_set_setting_impl(&shared.0, &key, &value)
}

/// Command body of [`cad_set_setting`], without the Tauri state wrapper.
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

/// `cad_layers` command: the layer-panel projection of the open drawing.
///
/// This is the React layer panel's explicit read path — deliberately **not** a
/// `cad_status` field, so the frozen status contract is untouched. The
/// owner-thread session is `!Send`, so the tick mirrors its
/// `CadSession::layers()` answer into the shared state exactly like
/// `cad_settings`. Before a session exists (or in probe mode) the read is an
/// explicit `cad_waiting`, never a fabricated layer table; with a session but no
/// drawing it is the honest empty answer (`layers: []`, `active: null`).
#[tauri::command]
pub fn cad_layers(shared: tauri::State<'_, ProbeShared>) -> Result<CadLayers, String> {
    cad_layers_impl(&shared.0)
}

/// Command body of [`cad_layers`], without the Tauri state wrapper.
///
/// Split out so the wire-shape/golden tests can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_layers_impl(shared: &Arc<Mutex<Shared>>) -> Result<CadLayers, String> {
    wire::layers(&lock(shared).queue_reads())
}

/// `cad_set_layer_visible` command: queue one **temporary** layer visibility.
///
/// The wire shape is `{ name: <string>, visible: <bool> }` and carries the
/// layer's **name**, never a `LayerId`: the host resolves the name against the
/// drawing's own layer table on the owner thread, so a stale UI can never flip a
/// layer that no longer exists (that is an explicit `layer.unknown` refusal). An
/// unknown or ambiguous name changes nothing; the DWG layer-table flag is never
/// written.
#[tauri::command]
pub fn cad_set_layer_visible(
    shared: tauri::State<'_, ProbeShared>,
    name: String,
    visible: bool,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_layer_visible(&mut state.queue_sink(), &name, visible)
}

/// `cad_restore_layers` command: queue a **restore** of every temporary layer
/// override.
///
/// The wire carries no fields. `{accepted:true}` means **queued**, not
/// completed: the owner tick routes it through `CadSession::restore_layers` (the
/// ordinary `RESTORELAYERS` funnel). Without a drawing the session refuses
/// explicitly (`layer.no_document`) instead of reporting a restore of nothing.
#[tauri::command]
pub fn cad_restore_layers(shared: tauri::State<'_, ProbeShared>) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::restore_layers(&mut state.queue_sink())
}

/// `cad_set_active_layer` command: queue one active-layer change.
///
/// The wire shape is `{ name: <string> }` and carries the layer's **name**; the
/// host resolves it against the drawing's layer table on the owner thread and
/// dispatches `SETACTIVELAYER`. An unknown or ambiguous name changes nothing and
/// surfaces as `cad_status.error`.
#[tauri::command]
pub fn cad_set_active_layer(
    shared: tauri::State<'_, ProbeShared>,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    wire::set_active_layer(&mut state.queue_sink(), &name)
}

/// `cad_create_layer` command: queue one **layer table** write.
///
/// Unlike the visibility/active commands this is a real database write (one
/// revision bump, not undoable). The wire shape is `{ name: <string> }`; the name
/// is validated on the owner thread by the database (trimmed, non-empty, the
/// AutoCAD-illegal character set, unique case-insensitively) and a refusal is the
/// database's own stable reason (`layer.name_empty`, `layer.name_illegal_char`,
/// `layer.duplicate_name`), never a repaired name.
#[tauri::command]
pub fn cad_create_layer(
    shared: tauri::State<'_, ProbeShared>,
    name: String,
) -> Result<CadCommandAck, String> {
    cad_create_layer_impl(&shared.0, name)
}

/// Command body of [`cad_create_layer`], without the Tauri state wrapper.
pub fn cad_create_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::create_layer(&mut state.queue_sink(), &name)
}

/// `cad_rename_layer` command: queue one layer rename (`RENAMELAYER`).
///
/// The wire shape is `{ layer: <string>, name: <string> }`: `layer` addresses the
/// existing row by **name** (the host resolves it against the drawing's own table,
/// so an unknown or ambiguous selector is an explicit `layer.unknown` /
/// `layer.ambiguous_name` refusal) and `name` is the new name, validated by the
/// same database rules as a create. Entity references are by id, so nothing is
/// retargeted. Not undoable.
#[tauri::command]
pub fn cad_rename_layer(
    shared: tauri::State<'_, ProbeShared>,
    layer: String,
    name: String,
) -> Result<CadCommandAck, String> {
    cad_rename_layer_impl(&shared.0, layer, name)
}

/// Command body of [`cad_rename_layer`], without the Tauri state wrapper.
pub fn cad_rename_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    name: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::rename_layer(&mut state.queue_sink(), &layer, &name)
}

/// `cad_delete_layer` command: queue one layer delete (`DELETELAYER`).
///
/// The wire shape is `{ layer: <string> }`, addressing the row by name. The
/// database refuses the default layer `0` (`layer.default_undeletable`) and any
/// layer an entity still references (`layer.in_use`, naming the count), so no
/// entity is ever left pointing at a missing layer. Deleting the active layer
/// makes `0` current again — that shows up in the next `cad_layers` read
/// (`active`), never as a stale name. Not undoable.
#[tauri::command]
pub fn cad_delete_layer(
    shared: tauri::State<'_, ProbeShared>,
    layer: String,
) -> Result<CadCommandAck, String> {
    cad_delete_layer_impl(&shared.0, layer)
}

/// Command body of [`cad_delete_layer`], without the Tauri state wrapper.
pub fn cad_delete_layer_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::delete_layer(&mut state.queue_sink(), &layer)
}

/// `cad_set_layer_property` command: queue one **layer attribute** write
/// (`SETLAYERPROPERTY`).
///
/// The wire shape is `{ layer: <string>, key: <string>, value: <string> }`:
/// `layer` addresses the row by **name** (the host resolves it against the
/// drawing's own table, so an unknown or ambiguous selector is an explicit
/// `layer.unknown` / `layer.ambiguous_name` refusal), `key` is one of
/// [`cad_app::layer_property::SUPPORTED_KEYS`] and `value` follows that key's
/// documented grammar — **the layer's own units**: `lineweight` is 1/100 mm and
/// `transparency` is percent transparent, deliberately different from
/// `SETENTITYPROPERTY`'s millimetres/percent but the same *idea*.
///
/// The key is checked before the queue (`layer.property.unsupported`, naming the
/// key — `plotstyle` is refused here because the writer only emits a style
/// handle); the value is the owner thread's call, because it can depend on the
/// drawing's linetype table (`layer.linetype_missing`).
#[tauri::command]
pub fn cad_set_layer_property(
    shared: tauri::State<'_, ProbeShared>,
    layer: String,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    cad_set_layer_property_impl(&shared.0, layer, key, value)
}

/// Command body of [`cad_set_layer_property`], without the Tauri state wrapper.
pub fn cad_set_layer_property_impl(
    shared: &Arc<Mutex<Shared>>,
    layer: String,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(shared);
    wire::set_layer_property(&mut state.queue_sink(), &layer, &key, &value)
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> cad_ui_core::session::CadStatus {
    cad_status_impl(&shared.0)
}

/// Command body of [`cad_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_status_impl(shared: &Arc<Mutex<Shared>>) -> cad_ui_core::session::CadStatus {
    let (status, popup, telemetry) = {
        let state = lock(shared);
        let host_error = state
            .shell
            .cad_host_error
            .as_ref()
            .map(|error| error.text.as_str());
        let status = match state.shell.cad_status.clone() {
            Some(snapshot) => projected_status(snapshot, host_error),
            None => unsupported_status(None),
        };
        // The session never owns a window: project the host's live popup state
        // (ADR-0021 D1/D2) onto every snapshot, including the unsupported one.
        (status, state.shell.popup.clone(), state.telemetry.clone())
    };
    let status = projected_popup(status, popup.as_ref());
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

/// `cad_commands` command: every command name the Rust router admits, so the
/// chrome never keeps a hand-written mirror that drifts from the host.
///
/// This is the single source of truth (`cad_ui_core::session::admissible_names`,
/// i.e. the canonical `COMMAND_TABLE` selectors plus their aliases, deduped). The
/// frontend still owns an explicit, documented exclusion list for names the
/// **React host** cannot service yet (native file-dialog/printer owners), but it
/// no longer invents which names the router would accept. The `dom-canvas` host
/// serves the identical list from the same `wire::commands`, so the chrome sees
/// one vocabulary everywhere.
#[tauri::command]
pub fn cad_commands() -> Vec<&'static str> {
    wire::commands()
}

/// `cad_command_aliases` command: the alias table the chrome offers in its
/// suggestions.
///
/// Aliases are a **display** aid only: admission still goes through
/// `cad_command`, whose router normalizes the typed text itself. The answer is
/// derived from the same Rust table (`cad_ui_core::session::alias_pairs`), so the
/// frontend never keeps a second copy that could drift from what the host accepts.
#[tauri::command]
pub fn cad_command_aliases() -> Vec<CadCommandAlias> {
    cad_command_aliases_impl()
}

/// Command body of [`cad_command_aliases`], without the Tauri command wrapper.
pub fn cad_command_aliases_impl() -> Vec<CadCommandAlias> {
    wire::aliases()
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
            eprintln!(
                "[probe] input kind={} seq={sequence} accepted=false reason={POPUP_MODAL_OPEN}",
                kind.as_str()
            );
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

/// The registered command set, in the order `generate_handler!` lists it.
///
/// Captured from the pre-migration Linux host (`apps/app-linux/src/host/
/// commands.rs` + its `generate_handler!`); the `registration` test asserts the
/// macro list below stays exactly this set.
pub const COMMAND_NAMES: &[&str] = &[
    "probe_status",
    "probe_input",
    "cad_command",
    "cad_command_aliases",
    "cad_status",
    "cad_commands",
    "cad_set_tool_parameter",
    "cad_set_tool_text",
    "cad_set_selected_text",
    "cad_set_selected_property",
    "cad_standard_view",
    "cad_open_popup",
    "cad_close_popup",
    "cad_settings",
    "cad_set_setting",
    "cad_layers",
    "cad_set_layer_visible",
    "cad_restore_layers",
    "cad_set_active_layer",
    "cad_create_layer",
    "cad_rename_layer",
    "cad_delete_layer",
    "cad_set_layer_property",
];

/// Build the `invoke_handler` for the desktop hosts.
///
/// This is the only `generate_handler!` invocation; moving it here moved the
/// `__cmd__*`/`__tauri_command_name_*` resolution with the commands (no
/// re-export shim remains in the app).
pub fn register_commands<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        probe_status,
        probe_input,
        cad_command,
        cad_command_aliases,
        cad_status,
        cad_commands,
        cad_set_tool_parameter,
        cad_set_tool_text,
        cad_set_selected_text,
        cad_set_selected_property,
        cad_standard_view,
        cad_open_popup,
        cad_close_popup,
        cad_settings,
        cad_set_setting,
        cad_layers,
        cad_set_layer_visible,
        cad_restore_layers,
        cad_set_active_layer,
        cad_create_layer,
        cad_rename_layer,
        cad_delete_layer,
        cad_set_layer_property
    ]
}
