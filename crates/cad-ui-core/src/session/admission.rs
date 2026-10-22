//! Typed admission for the dedicated CAD IPC channels (UI-framework-free).
//!
//! Every CAD mutation that is not a bare command name has its own typed channel
//! (`cad_set_tool_parameter`, `cad_set_tool_text`, `cad_set_selected_text`,
//! `cad_set_selected_property`, `cad_standard_view`, `cad_set_setting`). Those
//! channels share one discipline, which lives here so every host applies it
//! identically:
//!
//! - the wire form is parsed into a typed value, with **named** refusals and no
//!   silent fallback;
//! - static content rules (non-empty, bounded length), the modal popup gate,
//!   probe mode and the queue bound are checked *before* the queue;
//! - everything that needs **live session state** (a tool awaiting text, the
//!   current selection, the drawing's own name tables, the parameter's range) is
//!   the owner-thread session's authority on drain. Nothing is applied, clamped
//!   or validated against session state here.
//!
//! `{accepted:true}` on the wire therefore means **queued**, never completed.

use std::collections::VecDeque;

use cad_app::{StandardView, ToolParameter, ToolValue};

use crate::session::{CadSettingKey, CadSettingValue};
use crate::shell::{CAD_COMMAND_CAP, POPUP_MODAL_OPEN};

/// Bounded capacity of the typed tool-parameter queue. It reuses the CAD command
/// queue's capacity so both admissions share one bound.
pub const CAD_TOOL_PARAMETER_CAP: usize = CAD_COMMAND_CAP;

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
) -> Result<(ToolParameter, ToolValue), String> {
    let parameter = ToolParameter::from_key(parameter)
        .ok_or_else(|| format!("unknown tool parameter '{parameter}'"))?;
    match (number, flag) {
        (Some(number), None) => Ok((parameter, ToolValue::Number(number))),
        (None, Some(flag)) => Ok((parameter, ToolValue::Flag(flag))),
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
    queue: &mut Vec<(ToolParameter, ToolValue)>,
    cad_mode: bool,
    parameter: ToolParameter,
    value: ToolValue,
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

/// Bounded capacity of the dedicated tool-text queue (ADR 0019 decision A.1).
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_TOOL_TEXT_CAP: usize = CAD_COMMAND_CAP;

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

/// Bounded capacity of the dedicated selected-text queue (ADR 0020 amendment B).
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SELECTED_TEXT_CAP: usize = CAD_COMMAND_CAP;

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

/// Bounded capacity of the dedicated selected-property queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SELECTED_PROPERTY_CAP: usize = CAD_COMMAND_CAP;

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
/// ([`admit_standard_view`], [`crate::shell::admit_command`]): the modal popup
/// gate first (while a popup owns input, a CAD mutation is refused by name with
/// `popup.modal_open`), then a disabled CAD session (probe mode), then the bound.
/// ([`admit_selected_text`] deliberately has no modal gate because the text-edit
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

/// Bounded capacity of the dedicated standard-view queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one
/// bound.
pub const CAD_STANDARD_VIEW_CAP: usize = CAD_COMMAND_CAP;

/// Parse the wire form of a `cad_standard_view` request into the typed view.
///
/// The wire carries the view's **stable machine key** (`StandardView::key`, e.g.
/// `"isometric"`), matched ASCII case-insensitively against
/// [`cad_app::StandardView::ALL`]. An empty request and an unknown name are
/// explicit refusals with distinct names; there is deliberately **no** default
/// view and no clamp to a neighbouring one, so a malformed request can never
/// silently change the camera.
pub fn parse_standard_view(view: &str) -> Result<StandardView, String> {
    let name = view.trim();
    if name.is_empty() {
        return Err("view.standard_empty: cad_standard_view needs a view name".into());
    }
    StandardView::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.key().eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let expected = StandardView::ALL
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
/// Pure and toolkit-free, with the **same** gates as
/// [`crate::shell::admit_command`]: the modal popup gate first (while a popup
/// owns input, every CAD command is refused by name), then a disabled CAD
/// session (probe mode), then the bound. The view is not applied here — the
/// owner tick drains the queue through `CadSession::standard_view`, so no camera
/// state is touched off the main thread and a real refusal surfaces through
/// `cad_status.error`.
pub fn admit_standard_view(
    queue: &mut VecDeque<StandardView>,
    cad_mode: bool,
    modal_open: bool,
    view: StandardView,
) -> Result<(), String> {
    if modal_open {
        return Err(POPUP_MODAL_OPEN.into());
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
    queue: &mut VecDeque<StandardView>,
    mut apply: impl FnMut(StandardView) -> Result<(), String>,
) -> Vec<String> {
    let mut refusals = Vec::new();
    while let Some(view) = queue.pop_front() {
        if let Err(error) = apply(view) {
            refusals.push(error);
        }
    }
    refusals
}

/// Bounded capacity of the typed settings queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_SETTING_CAP: usize = CAD_COMMAND_CAP;

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
