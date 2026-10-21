//! Tauri IPC commands: probe status, CAD command admission, CAD status and the
//! sequence-gated input funnel.

use std::collections::VecDeque;

use cad_render_wgpu::PulledState;
use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{
    CadSettingKey, CadSettingValue, CadSettings, CadStatus, SETTINGS_INVALID_VALUE,
    SETTINGS_UNKNOWN_KEY,
};
use cad_ui_core::shell::{
    admit_command, modal_gate_open, projected_popup, projected_status, queue_cad_input,
    POPUP_MODAL_OPEN,
};
use cad_ui_core::unsupported_status;

use super::popup::{self, PopupAck, TauriPopupOps};
use super::postload::PostloadPanPhase;
use super::{lock, ProbeShared};

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    lock(&shared.0).shell.status.clone()
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

/// Bounded capacity of the typed tool-parameter queue. It reuses the CAD command
/// queue's capacity so both admissions share one bound.
pub const CAD_TOOL_PARAMETER_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

/// Parse the wire form of a `cad_set_tool_parameter` request into the typed pair.
///
/// The wire carries the parameter's **stable machine key** (camelCase, e.g.
/// `"polygonSides"`) plus **exactly one** of `number`/`flag`. An unknown key, or
/// neither/both value fields, is an explicit refusal (a malformed request never
/// reaches the queue). The parameter's own range/kind is **not** checked here:
/// that is the host session's authority (`CadSession::set_tool_parameter`), applied
/// on the owner thread when the tick drains the queue.
pub fn parse_tool_parameter(
    parameter: &str,
    number: Option<f64>,
    flag: Option<bool>,
) -> Result<(cad_app::ToolParameter, cad_app::ToolValue), String> {
    let parameter = cad_app::ToolParameter::from_key(parameter)
        .ok_or_else(|| format!("unknown tool parameter '{parameter}'"))?;
    match (number, flag) {
        (Some(number), None) => Ok((parameter, cad_app::ToolValue::Number(number))),
        (None, Some(flag)) => Ok((parameter, cad_app::ToolValue::Flag(flag))),
        (None, None) => Err("cad_set_tool_parameter needs exactly one of number/flag".into()),
        (Some(_), Some(_)) => {
            Err("cad_set_tool_parameter takes exactly one of number/flag, not both".into())
        }
    }
}

/// Admit one typed tool parameter into the bounded main-thread queue.
///
/// Pure and toolkit-free so the admission discipline (disabled CAD session,
/// bounded queue) is directly testable. The value is deliberately **not**
/// validated or clamped here: the parameter's own range/kind and the Viewer-mode
/// refusal are the host session's authority (`CadSession::set_tool_parameter`),
/// applied on the owner thread when the tick drains this queue, and the real
/// outcome surfaces through `cad_status.error` — exactly like `cad_command`.
pub fn admit_tool_parameter(
    queue: &mut Vec<(cad_app::ToolParameter, cad_app::ToolValue)>,
    cad_mode: bool,
    parameter: cad_app::ToolParameter,
    value: cad_app::ToolValue,
) -> Result<(), String> {
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_TOOL_PARAMETER_CAP {
        return Err("CAD tool-parameter queue is full".into());
    }
    queue.push((parameter, value));
    Ok(())
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

/// Bounded capacity of the dedicated tool-text queue (ADR 0019 decision A.1).
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_TOOL_TEXT_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

/// Admit one terminal tool text into the bounded main-thread queue.
///
/// Modelled exactly on [`admit_tool_parameter`]: disabled CAD sessions, a full
/// queue and the two **static** content refusals (empty/whitespace-only, more
/// than [`cad_app::MAX_TOOL_TEXT_CHARS`] characters) are refused *before* the
/// queue.
/// Everything that needs live session state — whether a tool is awaiting text,
/// whether its points are complete — is the host session's authority
/// (`CadSession::set_tool_text`) applied on the owner thread, so this function
/// deliberately knows no session state.
pub fn admit_tool_text(
    queue: &mut Vec<String>,
    cad_mode: bool,
    text: String,
) -> Result<(), String> {
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_TOOL_TEXT_CAP {
        return Err("CAD tool-text queue is full".into());
    }
    if text.trim().is_empty() {
        return Err("tool.text_empty: cad_set_tool_text needs a non-empty text".into());
    }
    let characters = text.chars().count();
    if characters > cad_app::MAX_TOOL_TEXT_CHARS {
        return Err(format!(
            "tool.text_too_long: cad_set_tool_text allows at most {} characters, got {characters}",
            cad_app::MAX_TOOL_TEXT_CHARS
        ));
    }
    queue.push(text);
    Ok(())
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

/// Bounded capacity of the dedicated selected-text queue (ADR 0020 amendment B).
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SELECTED_TEXT_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

/// Admit one replacement text for the **current single selection** into the
/// bounded main-thread queue.
///
/// Modelled exactly on [`admit_tool_text`]: a disabled CAD session, a full queue
/// and the two **static** content refusals (empty/whitespace-only, more than
/// [`cad_app::MAX_TOOL_TEXT_CHARS`] characters) are refused *before* the queue.
/// Whether the current selection is exactly one directly addressable `Text` is
/// live session state and therefore the host session's authority
/// (`CadSession::set_selected_text` → the `TextEdit` command) applied on the
/// owner thread; this function deliberately knows no session state.
pub fn admit_selected_text(
    queue: &mut Vec<String>,
    cad_mode: bool,
    text: String,
) -> Result<(), String> {
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_SELECTED_TEXT_CAP {
        return Err("CAD selected-text queue is full".into());
    }
    if text.trim().is_empty() {
        return Err("tool.text_empty: cad_set_selected_text needs a non-empty text".into());
    }
    let characters = text.chars().count();
    if characters > cad_app::MAX_TOOL_TEXT_CHARS {
        return Err(format!(
            "tool.text_too_long: cad_set_selected_text allows at most {} characters, got \
             {characters}",
            cad_app::MAX_TOOL_TEXT_CHARS
        ));
    }
    queue.push(text);
    Ok(())
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

/// Bounded capacity of the dedicated selected-property queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SELECTED_PROPERTY_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

/// Parse the wire form of a `cad_set_selected_property` request.
///
/// Both fields are strings: `key` is a stable machine key and `value` is that
/// key's documented grammar (see [`cad_app::property_value`]). Only the **key**
/// is checked here, because it is database-independent
/// (`property.unsupported_key`, the same stable code the host write path uses
/// for `text` and for any other unknown key). The value's grammar depends on the
/// drawing's own tables (a layer/linetype name), so it is the host session's
/// authority on the owner thread — never guessed or clamped here. No
/// human-readable text is produced for a refused request: only stable codes.
pub fn parse_property_write(key: &str, value: &str) -> Result<(String, String), String> {
    if !cad_app::property_value::is_supported_key(key) {
        return Err("property.unsupported_key".to_string());
    }
    Ok((key.to_string(), value.to_string()))
}

/// Admit one property write for the **current selection** into the bounded
/// main-thread queue.
///
/// The gates match the other CAD mutation admissions
/// ([`admit_standard_view`], [`admit_command`]): the modal popup gate first
/// (while a popup owns input, a CAD mutation is refused by name with
/// `popup.modal_open`), then a disabled CAD session (probe mode), then the bound.
/// (`admit_selected_text` deliberately has no modal gate because the text-edit
/// popup itself writes through it; this channel is the docked panel's, so it is
/// gated like every other CAD mutation.)
///
/// Nothing is applied here: the key/value pair only carries strings, and whether
/// the selection is addressable, whether the value is in the drawing's tables
/// and whether anything actually changes are all live session state resolved on
/// the owner thread (`CadSession::set_selected_property`) when the tick drains
/// the queue.
pub fn admit_selected_property(
    queue: &mut Vec<(String, String)>,
    cad_mode: bool,
    modal_open: bool,
    key: String,
    value: String,
) -> Result<(), String> {
    if modal_open {
        return Err(POPUP_MODAL_OPEN.into());
    }
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_SELECTED_PROPERTY_CAP {
        return Err("CAD selected-property queue is full".into());
    }
    queue.push((key, value));
    Ok(())
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

/// Bounded capacity of the dedicated standard-view queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one
/// bound.
pub const CAD_STANDARD_VIEW_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

/// Parse the wire form of a `cad_standard_view` request into the typed view.
///
/// The wire carries the view's **stable machine key** (`StandardView::key`, e.g.
/// `"isometric"`), matched ASCII case-insensitively against
/// [`cad_app::StandardView::ALL`]. An empty request and an unknown name are
/// explicit refusals with distinct names; there is deliberately **no** default
/// view and no clamp to a neighbouring one, so a malformed request can never
/// silently change the camera.
pub fn parse_standard_view(view: &str) -> Result<cad_app::StandardView, String> {
    let name = view.trim();
    if name.is_empty() {
        return Err("view.standard_empty: cad_standard_view needs a view name".into());
    }
    cad_app::StandardView::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.key().eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let expected = cad_app::StandardView::ALL
                .iter()
                .map(|view| view.key())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "view.standard_unknown: cad_standard_view has no view '{name}' (expected one of \
                 {expected})"
            )
        })
}

/// Admit one typed standard view into the bounded main-thread queue.
///
/// Pure and toolkit-free, with the **same** gates as [`admit_command`]: the modal
/// popup gate first (while a popup owns input, every CAD command is refused by
/// name), then a disabled CAD session (probe mode), then the bound. The view is
/// not applied here — the owner tick drains the queue through
/// `CadSession::standard_view`, so no camera state is touched off the main
/// thread and a real refusal surfaces through `cad_status.error`.
pub fn admit_standard_view(
    queue: &mut VecDeque<cad_app::StandardView>,
    cad_mode: bool,
    modal_open: bool,
    view: cad_app::StandardView,
) -> Result<(), String> {
    if modal_open {
        return Err(cad_ui_core::shell::POPUP_MODAL_OPEN.into());
    }
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_STANDARD_VIEW_CAP {
        return Err("CAD standard-view queue is full".into());
    }
    queue.push_back(view);
    Ok(())
}

/// Drain the standard-view queue in admission order, applying each view.
///
/// `apply` is the host session funnel (`CadSession::standard_view`), so the real
/// camera transition — and any real refusal — happens on the owner thread. The
/// last admitted view is the one left applied. Refusals are returned so the
/// caller can log them; the session has already recorded them for
/// `cad_status.error`, so nothing is swallowed and nothing is invented.
pub fn drain_standard_views(
    queue: &mut VecDeque<cad_app::StandardView>,
    mut apply: impl FnMut(cad_app::StandardView) -> Result<(), String>,
) -> Vec<String> {
    let mut refusals = Vec::new();
    while let Some(view) = queue.pop_front() {
        if let Err(error) = apply(view) {
            refusals.push(error);
        }
    }
    refusals
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
#[tauri::command]
pub fn cad_open_popup(
    app: tauri::AppHandle,
    shared: tauri::State<'_, ProbeShared>,
    kind: String,
) -> Result<PopupAck, String> {
    // The CAD session and the native window are main-thread state; the whole
    // open runs there (see `popup::on_main_thread`).
    let shared = shared.0.clone();
    let task_shared = shared.clone();
    let task_app = app.clone();
    popup::on_main_thread(&app, move || {
        let ops = TauriPopupOps::new(task_app, task_shared.clone());
        popup::open_popup(&task_shared, &kind, &ops)
    })
}

/// `cad_close_popup` command: close the open modal popup (ADR-0021 D1/D2).
///
/// Closing when nothing is open is an **idempotent success**, not an error. The
/// modal gate itself is cleared by the popup window's `Destroyed` event, which
/// also covers Esc in the popup UI and a window-manager close.
#[tauri::command]
pub fn cad_close_popup(
    app: tauri::AppHandle,
    shared: tauri::State<'_, ProbeShared>,
) -> Result<CadCommandAck, String> {
    let ops = TauriPopupOps::new(app, shared.0.clone());
    popup::close_popup(&shared.0, &ops)?;
    Ok(CadCommandAck { accepted: true })
}

/// Bounded capacity of the typed settings queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SETTING_CAP: usize = cad_ui_core::shell::CAD_COMMAND_CAP;

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

/// Admit one typed setting into the bounded main-thread queue.
///
/// Deliberately **not** gated by the modal popup gate: the Options dialog runs
/// in its own window, and persisting a user preference is not CAD input. A
/// disabled CAD session (probe mode), a full queue and a value outside the
/// closed key table are refused before the queue.
pub fn admit_setting(
    queue: &mut VecDeque<(CadSettingKey, CadSettingValue)>,
    cad_mode: bool,
    key: CadSettingKey,
    value: CadSettingValue,
) -> Result<(), String> {
    key.validate(&value).map_err(str::to_string)?;
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if queue.len() >= CAD_SETTING_CAP {
        return Err("settings queue is full".into());
    }
    queue.push_back((key, value));
    Ok(())
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

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side/// (transport) error merged in.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> CadStatus {
    let (status, popup) = {
        let state = lock(&shared.0);
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
        (status, state.shell.popup.clone())
    };
    let status = projected_popup(status, popup.as_ref());
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    // The pulled organisation state is appended as a separate token (the JSON is
    // unchanged and stays first) so a skipped accelerated organisation is visible
    // next to the status payload.
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        let pulled = lock(&shared.0)
            .last_pulled_state
            .as_ref()
            .map(PulledState::to_string)
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
    let kind = input.kind;
    let sequence = input.sequence;
    // Modal popup gate (ADR-0021 D2): while a popup is open, every interaction
    // input is refused by name before the tracker sees it. `resize` still passes
    // because it carries no CAD intent and keeps the layout truthful while the
    // modal is up. This gate is host-owned, so the command line's blur path can
    // never release it.
    if !matches!(kind, InputKind::Resize) {
        let refused = {
            let state = lock(&shared.0);
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
    let ack = lock(&shared.0).shell.input.apply(&input);
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
    // to the pan).
    {
        let mut state = lock(&shared.0);
        if let Some(pan) = state.postload_pan.as_mut() {
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
    if matches!(kind, InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(&shared.0);
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
