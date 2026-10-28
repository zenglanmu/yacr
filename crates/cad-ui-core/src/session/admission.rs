//! Typed admission for the dedicated CAD IPC channels (UI-framework-free).
//!
//! Every CAD mutation that is not a bare command name has its own typed channel
//! (`cad_set_tool_parameter`, `cad_set_tool_text`, `cad_set_selected_text`,
//! `cad_set_selected_property`, `cad_standard_view`, `cad_set_setting`,
//! `cad_set_layer_visible` / `cad_restore_layers` / `cad_set_active_layer` /
//! `cad_set_layer_property`). Those channels share one discipline, which lives
//! here so every host applies it identically:
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
//!
//! # One rule, two store shapes
//!
//! Each channel's rule is a `check_*` function: it takes the **host gate inputs**
//! (CAD mode, the modal gate, the occupancy of that channel) plus the parsed
//! value, and returns the byte-identical refusal the desktop has always produced.
//! The `admit_*` functions below are the concrete-queue store shape
//! (`check_*` + push); [`crate::session::wire`] is the sink seam shape
//! (`check_*` + `AdmissionTarget::…`). The rules — gate order, bounds and every
//! diagnostic string — live here and only here.

use std::collections::VecDeque;

use cad_app::{StandardView, ToolParameter, ToolValue};

use crate::session::{CadLayerWrite, CadSettingKey, CadSettingValue};
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

/// The gate of the tool-parameter channel: CAD mode, then the bound.
///
/// Shared by [`admit_tool_parameter`] and
/// [`crate::session::wire::set_tool_parameter`], so the desktop queue and a
/// synchronous sink cannot diverge.
pub(super) fn check_tool_parameter(cad_mode: bool, pending: usize) -> Result<(), String> {
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= CAD_TOOL_PARAMETER_CAP {
        return Err("CAD tool-parameter queue is full".into());
    }
    Ok(())
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
    check_tool_parameter(cad_mode, queue.len())?;
    queue.push((parameter, value));
    Ok(())
}

/// Bounded capacity of the dedicated tool-text queue (ADR 0019 decision A.1).
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_TOOL_TEXT_CAP: usize = CAD_COMMAND_CAP;

/// The gate of the terminal tool-text channel: CAD mode, the bound, then the two
/// **static** content rules (empty/whitespace-only and the length bound).
///
/// The refusal text names the IPC channel (`channel` is the `cad_set_…` name), so
/// the two text channels keep their distinct, long-standing messages while
/// sharing one implementation.
fn check_text(
    channel: &str,
    cap: usize,
    cad_mode: bool,
    pending: usize,
    pending_reason: &str,
    text: &str,
) -> Result<(), String> {
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= cap {
        return Err(pending_reason.to_string());
    }
    if text.trim().is_empty() {
        return Err(format!("tool.text_empty: {channel} needs a non-empty text"));
    }
    let characters = text.chars().count();
    if characters > cad_app::MAX_TOOL_TEXT_CHARS {
        return Err(format!(
            "tool.text_too_long: {channel} allows at most {} characters, got {characters}",
            cad_app::MAX_TOOL_TEXT_CHARS
        ));
    }
    Ok(())
}

/// The gate of the `cad_set_tool_text` channel (see [`check_text`]).
pub(super) fn check_tool_text(cad_mode: bool, pending: usize, text: &str) -> Result<(), String> {
    check_text(
        "cad_set_tool_text",
        CAD_TOOL_TEXT_CAP,
        cad_mode,
        pending,
        "CAD tool-text queue is full",
        text,
    )
}

/// The gate of the `cad_set_selected_text` channel (see [`check_text`]).
pub(super) fn check_selected_text(
    cad_mode: bool,
    pending: usize,
    text: &str,
) -> Result<(), String> {
    check_text(
        "cad_set_selected_text",
        CAD_SELECTED_TEXT_CAP,
        cad_mode,
        pending,
        "CAD selected-text queue is full",
        text,
    )
}

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
    check_tool_text(cad_mode, queue.len(), &text)?;
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
    check_selected_text(cad_mode, queue.len(), &text)?;
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

/// The gate of the selected-property channel: the modal popup gate, CAD mode,
/// then the bound (the order [`admit_selected_property`] has always applied).
pub(super) fn check_selected_property(
    cad_mode: bool,
    modal_open: bool,
    pending: usize,
) -> Result<(), String> {
    if modal_open {
        return Err(POPUP_MODAL_OPEN.into());
    }
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= CAD_SELECTED_PROPERTY_CAP {
        return Err("CAD selected-property queue is full".into());
    }
    Ok(())
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
    check_selected_property(cad_mode, modal_open, queue.len())?;
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

/// The gate of the standard-view channel: the modal popup gate, CAD mode, then
/// the bound (the same order as [`crate::shell::admit_command`]).
pub(super) fn check_standard_view(
    cad_mode: bool,
    modal_open: bool,
    pending: usize,
) -> Result<(), String> {
    if modal_open {
        return Err(POPUP_MODAL_OPEN.into());
    }
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= CAD_STANDARD_VIEW_CAP {
        return Err("CAD standard-view queue is full".into());
    }
    Ok(())
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
    check_standard_view(cad_mode, modal_open, queue.len())?;
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

/// The gate of the settings channel: the closed key table first (so an illegal
/// value is refused before any host gate), then CAD mode, then the bound.
///
/// Deliberately **not** gated by the modal popup gate: the Options dialog runs
/// open, and persisting a user preference is not CAD input.
pub(super) fn check_setting(
    cad_mode: bool,
    pending: usize,
    key: &CadSettingKey,
    value: &CadSettingValue,
) -> Result<(), String> {
    key.validate(value).map_err(str::to_string)?;
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= CAD_SETTING_CAP {
        return Err("settings queue is full".into());
    }
    Ok(())
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
    check_setting(cad_mode, queue.len(), &key, &value)?;
    queue.push_back((key, value));
    Ok(())
}

/// Bounded capacity of the typed layer-write queue.
///
/// It reuses the CAD command queue's capacity so every admission shares one bound.
pub const CAD_LAYER_WRITE_CAP: usize = CAD_COMMAND_CAP;

/// Parse the wire form of a `cad_set_layer_property` request.
///
/// The three fields are strings: `layer` is a layer **name** (resolved on the
/// owner thread, never here), `key` is a stable machine key and `value` is that
/// key's documented grammar (see [`cad_app::layer_property`]). Only the **key**
/// is checked here, because it is database-independent: an unknown key and the
/// known-but-unimplemented `plotstyle` are refused statically with
/// `layer.property.unsupported`, naming the key. The value's grammar can depend
/// on the drawing's own tables (a linetype name must exist), so it is the owner
/// thread's authority — never guessed or clamped here.
pub fn parse_layer_property_write(
    layer: &str,
    key: &str,
    value: &str,
) -> Result<(String, String, String), String> {
    if !cad_app::layer_property::is_supported_key(key) {
        return Err(format!(
            "layer.property.unsupported: '{key}' is not a writable layer property"
        ));
    }
    Ok((layer.to_string(), key.to_string(), value.to_string()))
}

/// The gate of the layer-write channel: the modal popup gate, CAD mode, then the
/// bound (the same order as [`admit_selected_property`]).
pub(super) fn check_layer_write(
    cad_mode: bool,
    modal_open: bool,
    pending: usize,
) -> Result<(), String> {
    if modal_open {
        return Err(POPUP_MODAL_OPEN.into());
    }
    if !cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if pending >= CAD_LAYER_WRITE_CAP {
        return Err("CAD layer queue is full".into());
    }
    Ok(())
}

/// Admit one typed layer write into the bounded main-thread queue.
///
/// The gates match [`admit_selected_property`]: the modal popup gate first (while
/// a popup owns input, a CAD mutation is refused by name with
/// `popup.modal_open`), then a disabled CAD session (probe mode), then the bound.
///
/// Nothing is resolved here: a layer name is matched against the drawing's own
/// layer table on the owner thread (`CadSession::set_layer_visibility` /
/// `set_active_layer`), so no name is guessed, trimmed or defaulted off the main
/// thread and a refusal is the session's real error.
pub fn admit_layer_write(
    queue: &mut VecDeque<CadLayerWrite>,
    cad_mode: bool,
    modal_open: bool,
    write: CadLayerWrite,
) -> Result<(), String> {
    check_layer_write(cad_mode, modal_open, queue.len())?;
    queue.push_back(write);
    Ok(())
}

/// Drain the layer-write queue in admission order, applying each write.
///
/// `apply` is the owner-thread funnel ([`CadLayerWrite::apply`]), so the real
/// resolution and any real refusal happen on the main thread. Refusals are
/// returned so the caller can log them; nothing is swallowed and nothing is
/// invented.
pub fn drain_layer_writes(
    queue: &mut VecDeque<CadLayerWrite>,
    mut apply: impl FnMut(&CadLayerWrite) -> Result<(), String>,
) -> Vec<String> {
    let mut refusals = Vec::new();
    while let Some(write) = queue.pop_front() {
        if let Err(error) = apply(&write) {
            refusals.push(error);
        }
    }
    refusals
}
