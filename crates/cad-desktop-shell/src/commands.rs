//! Tauri IPC commands shared by every desktop host.
//!
//! These are the thin `#[tauri::command]` wrappers plus host-neutral wiring. The
//! **rules** they apply — typed wire parsing, static content limits, gate order
//! and queue bounds — live in `cad_ui_core::session` (`admission`) and
//! `cad_ui_core::shell`; this module only maps them onto [`Shared`].
//!
//! The renderer telemetry that `cad_status`/`probe_input` used to read directly
//! is now behind [`ShellTelemetry`]; no `cad-render-wgpu`/`cad-engine` type is
//! named here.
//!
//! ## Registration
//!
//! [`register_commands`] is the single `generate_handler!` list. [`COMMAND_NAMES`]
//! is the command set captured from the pre-migration host; the `registration`
//! test asserts the macro list and the constant are the same set.

use std::sync::{Arc, Mutex};

use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{
    admit_selected_property, admit_setting, admit_standard_view, admit_tool_parameter,
    admit_tool_text, parse_property_write, parse_standard_view, parse_tool_parameter,
    CadSettingKey, CadSettingValue, CadSettings, CadStatus, SETTINGS_INVALID_VALUE,
    SETTINGS_UNKNOWN_KEY,
};
use cad_ui_core::shell::{
    admit_command, modal_gate_open, projected_popup, projected_status, queue_cad_input,
    POPUP_MODAL_OPEN,
};
use cad_ui_core::unsupported_status;

use crate::state::{lock, ProbeShared, Shared};

// Re-exported for the evidence driver, which must queue through the **same**
// selected-text admission `cad_set_selected_text` runs.
pub use cad_ui_core::session::admit_selected_text;
// Re-exported for the owner tick, which drains the queue with the shared rule.
pub use cad_ui_core::session::drain_standard_views;

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

/// Admission result of `cad_command`: queued, **not** completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

/// `cad_command` command: admit a known CAD command into the bounded main-thread
/// queue.
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
    admit_command(&mut state.shell, &command)?;
    Ok(CadCommandAck { accepted: true })
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
/// validates the parameter's range/kind and Viewer mode. `{accepted:true}` means
/// **queued**, not completed; the real outcome (including an explicit refusal)
/// surfaces through `cad_status.error`.
#[tauri::command]
pub fn cad_set_tool_parameter(
    shared: tauri::State<'_, ProbeShared>,
    parameter: String,
    number: Option<f64>,
    flag: Option<bool>,
) -> Result<CadCommandAck, String> {
    let (parameter, value) = parse_tool_parameter(&parameter, number, flag)?;
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    admit_tool_parameter(&mut state.cad_tool_parameters, cad_mode, parameter, value)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_set_tool_text` command: admit one **terminal** text into the bounded
/// main-thread queue (ADR 0019 decision A.1).
///
/// This is the only string channel into the CAD session. It is a dedicated typed
/// IPC and deliberately **not** an argument of `cad_command`: that funnel accepts
/// a command *name* only, and parsing a text payload out of the command line was
/// rejected earlier. The wire shape is `{ text: <string> }`.
///
/// `{accepted:true}` means **queued**, not completed: the owner tick routes it
/// through `CadSession::set_tool_text`, which validates the live state and
/// completes the command immediately (text is terminal). Any refusal
/// (`tool.text_not_awaiting`, `tool.text_points_incomplete`, `tool.text_empty`,
/// `tool.text_too_long`) surfaces through `cad_status.error`.
#[tauri::command]
pub fn cad_set_tool_text(
    shared: tauri::State<'_, ProbeShared>,
    text: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    admit_tool_text(&mut state.cad_tool_texts, cad_mode, text)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_set_selected_text` command: admit one replacement text for the current
/// single selection into the bounded main-thread queue (ADR 0020 amendment B).
///
/// The wire shape is `{ text: <string> }` and deliberately carries **no entity
/// id**: the host resolves and validates its own current selection at apply
/// time, so a UI can never retarget another object with a stale id. The field is
/// refused with `textedit.selection_empty` / `textedit.selection_multiple` /
/// `textedit.target_not_text` when the selection is not exactly one directly
/// addressable `Text`; empty/over-long drafts are refused here with the same
/// `tool.text_empty` / `tool.text_too_long` names the text step uses.
///
/// `{accepted:true}` means **queued**, not completed: the owner tick routes it
/// through `CadSession::set_selected_text`, and the real outcome (including any
/// refusal) surfaces through `cad_status.error` / the next `selection.text`.
#[tauri::command]
pub fn cad_set_selected_text(
    shared: tauri::State<'_, ProbeShared>,
    text: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    admit_selected_text(&mut state.cad_selected_texts, cad_mode, text)?;
    Ok(CadCommandAck { accepted: true })
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
///
/// `{accepted:true}` means **queued**, not completed: the owner tick routes it
/// through `CadSession::set_selected_property`, and the real outcome (including
/// any refusal) surfaces through `cad_status.error` and the next
/// `cad_status.selection.rows` — the display-attribute rows are the canonical
/// write grammar, so the written value reads back unchanged.
#[tauri::command]
pub fn cad_set_selected_property(
    shared: tauri::State<'_, ProbeShared>,
    key: String,
    value: String,
) -> Result<CadCommandAck, String> {
    let (key, value) = parse_property_write(&key, &value)?;
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    let modal_open = modal_gate_open(&state.shell);
    admit_selected_property(
        &mut state.cad_selected_properties,
        cad_mode,
        modal_open,
        key,
        value,
    )?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_standard_view` command: admit one typed standard view into the bounded
/// main-thread queue.
///
/// `STANDARDVIEW` maps to a payload-carrying [`cad_app::CommandId::StandardView`]
/// (`CommandPayload::StandardView`), which the name-only `cad_command` funnel
/// cannot carry: that funnel queues a normalized name and the router refuses a
/// payload-carrying variant. This dedicated typed IPC is the sanctioned channel
/// for it (the same shape as `cad_set_tool_parameter`), deliberately **not** an
/// argument added to `cad_command`.
///
/// The wire shape is `{ view: <key> }` with a stable key from
/// [`cad_app::StandardView::ALL`] (`top`, `bottom`, `front`, `back`, `left`,
/// `right`, `isometric`), matched case-insensitively. `{accepted:true}` means
/// **queued**, not completed: the owner tick drains the queue through
/// `CadSession::standard_view`, and the real outcome (including any refusal)
/// surfaces through `cad_status.error` and the next camera block.
#[tauri::command]
pub fn cad_standard_view(
    shared: tauri::State<'_, ProbeShared>,
    view: String,
) -> Result<CadCommandAck, String> {
    let view = parse_standard_view(&view)?;
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    let modal_open = modal_gate_open(&state.shell);
    admit_standard_view(&mut state.cad_standard_views, cad_mode, modal_open, view)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_open_popup` command: open the allowlisted modal popup, or focus the one
/// already open (ADR-0021 D1/D2).
///
/// The wire carries a **kind key only** (`{ kind: String }`): a caller can never
/// choose a window label, URL or script. `"about"`, `"textEdit"` and `"options"`
/// are the allowlisted kinds; anything else is refused with `popup.unknown_kind: …`
/// before any window or gate change. `textEdit` derives its payload from the
/// host's current single selection (never from the caller); a selection that is
/// not exactly one editable text is refused with `popup.no_text_selection: …`
/// and no window is created. `options` carries no payload at all: the dialog
/// reads `cad_settings` itself. While the popup is open the host refuses CAD input
/// (`popup.modal_open`), independently of the UI's own pointer suspension.
///
/// The window effects and the main-thread marshalling belong to the concrete
/// host behind [`crate::state::ShellPopupHost`]; the state machine and the gate
/// are `cad_ui_core::popup`.
#[tauri::command]
pub fn cad_open_popup(
    shared: tauri::State<'_, ProbeShared>,
    kind: String,
) -> Result<PopupAck, String> {
    let popup_host = {
        let state = lock(&shared.0);
        state.popup_host.clone()
    };
    let Some(popup_host) = popup_host else {
        return Err("popup.host_unavailable: no desktop popup host is registered".to_string());
    };
    popup_host.open(&shared.0, &kind)
}

/// `cad_close_popup` command: close the open modal popup (ADR-0021 D1/D2).
///
/// Closing when nothing is open is an **idempotent success**, not an error. The
/// modal gate itself is cleared by the popup window's `Destroyed` event, which
/// also covers Esc in the popup UI and a window-manager close.
#[tauri::command]
pub fn cad_close_popup(shared: tauri::State<'_, ProbeShared>) -> Result<CadCommandAck, String> {
    let popup_host = {
        let state = lock(&shared.0);
        state.popup_host.clone()
    };
    let Some(popup_host) = popup_host else {
        return Err("popup.host_unavailable: no desktop popup host is registered".to_string());
    };
    popup_host.close(&shared.0)?;
    Ok(CadCommandAck { accepted: true })
}

/// Parse one `cad_set_setting` request into the typed key/value pair.
///
/// The wire is `{ key: <stable camelCase key>, value: <bool|string|integer> }`.
/// An unknown key is refused with `settings.unknown_key`; a wrong value shape is
/// refused with `settings.invalid_value`; a `plotFormat` other than `svg`/`pdf`
/// is refused by the key table's own validation, as is a colour that is not
/// `#rrggbb` and a crosshair percentage outside `1..=100` (the same authority the
/// owner thread uses). Nothing is coerced and no file is touched here.
pub fn parse_setting_request(
    key: &str,
    value: &serde_json::Value,
) -> Result<(CadSettingKey, CadSettingValue), String> {
    let key = CadSettingKey::parse(key).ok_or_else(|| SETTINGS_UNKNOWN_KEY.to_string())?;
    let value = match value {
        serde_json::Value::Bool(enabled) => CadSettingValue::Bool(*enabled),
        serde_json::Value::String(text) => CadSettingValue::Text(text.clone()),
        // Only a non-negative integer that fits `u32` is a number here; a
        // negative, fractional or out-of-range JSON number is a wrong shape.
        serde_json::Value::Number(number) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .map(CadSettingValue::Number)
            .ok_or_else(|| SETTINGS_INVALID_VALUE.to_string())?,
        _ => return Err(SETTINGS_INVALID_VALUE.to_string()),
    };
    key.validate(&value).map_err(str::to_string)?;
    Ok((key, value))
}

/// `cad_settings` command: the latest effective settings snapshot.
///
/// The owner-thread session is `!Send`, so the tick mirrors its
/// `CadSession::settings()` snapshot into the shared state exactly like
/// `cad_status`. Before a session exists (or in probe mode) the read is an
/// explicit `cad_waiting`, never a fabricated default object.
#[tauri::command]
pub fn cad_settings(shared: tauri::State<'_, ProbeShared>) -> Result<CadSettings, String> {
    lock(&shared.0)
        .cad_settings
        .clone()
        .ok_or_else(|| "cad_waiting".to_string())
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
    let (key, value) = parse_setting_request(&key, &value)?;
    let mut state = lock(&shared.0);
    let cad_mode = state.shell.cad_mode;
    admit_setting(&mut state.cad_setting_writes, cad_mode, key, value)?;
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
    cad_status_impl(&shared.0)
}

/// Command body of [`cad_status`], without the Tauri state wrapper.
///
/// Split out so the wire-output goldens can call the **same** implementation
/// the IPC path runs (no duplicated projection in tests).
pub fn cad_status_impl(shared: &Arc<Mutex<Shared>>) -> CadStatus {
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
/// no longer invents which names the router would accept.
#[tauri::command]
pub fn cad_commands() -> Vec<&'static str> {
    cad_ui_core::session::admissible_names()
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
        cad_set_setting
    ]
}
