//! Tauri IPC commands: probe status, CAD command admission, CAD status and the
//! sequence-gated input funnel.

use cad_render_wgpu::PulledState;
use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::CadStatus;
use cad_ui_core::shell::{admit_command, projected_status, queue_cad_input};
use cad_ui_core::unsupported_status;

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

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side/// (transport) error merged in.
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
            .shell
            .cad_host_error
            .as_ref()
            .map(|error| error.text.as_str());
        match state.shell.cad_status.clone() {
            Some(snapshot) => projected_status(snapshot, host_error),
            None => unsupported_status(None),
        }
    };
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
