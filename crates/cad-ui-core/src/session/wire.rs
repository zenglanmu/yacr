//! **The shared command-semantics layer**: one function per wire command, the
//! acknowledgement shape, and the sink seam a host implements.
//!
//! # Why this exists
//!
//! The desktop hosts (`apps/app-linux`, `apps/app-windows`) reach CAD through
//! Tauri IPC; the `dom-canvas` hosts (`apps/app-web`, the Linux X11 path) call
//! the same session **directly** from wasm. Their command *semantics* — wire
//! argument parsing, the modal/probe-mode gates, the queue bounds, every refusal
//! string and the `{accepted:true}` acknowledgement — are one authority, and they
//! live here. A host supplies only the **transport**:
//!
//! | host | wire args | [`AdmissionTarget`] | reads ([`ControlReads`]) |
//! |---|---|---|---|
//! | desktop | `#[tauri::command]` signature | [`QueueSink`] over `Shared`'s bounded queues | the owner-tick mirror (`Shared::cad_settings`/`cad_layers`) |
//! | `dom-canvas` | JSON args → the same typed values | `DirectSink` over the live `CadSession` | the session itself (`CadSession::settings`/`layers`) |
//!
//! Every `wire::*` function parses, applies the **shared** gate (the `check_*`
//! functions in [`super::admission`] and [`crate::shell::check_command`]), then
//! hands the already-admitted value to the sink. The sink **stores** (desktop) or
//! **applies** (`dom-canvas`); it never re-invents a rule.
//!
//! # Queued vs applied — the one documented asymmetry
//!
//! `CadSession` is `!Send` and owned by the desktop's main thread, so the desktop
//! cannot apply a command on the IPC thread: it queues it and the owner tick
//! applies it. `{accepted:true}` therefore means **queued** there, and a refusal
//! the session makes (an unknown layer name, a selection that is not text, …)
//! surfaces through the next `cad_status.error`. A synchronous sink applies the
//! request inside the same call, so its `Ok` means **applied** and the session's
//! refusal is returned immediately — while still being recorded in the session's
//! own error state exactly as the queued path records it on drain.
//!
//! The **admission verdict is identical on both paths**: the same gate, the same
//! order, the same string. Only *when* the session's own verdict is observable
//! differs, which is a property of the transport, not of the semantics. Hosts
//! that need the desktop's deferred shape keep the queued path — that is exactly
//! what the sink seam is for.
//!
//! # Bounded queues are a transport property
//!
//! The caps stay in the shared layer (`CAD_COMMAND_CAP` and its siblings in
//! [`super::admission`] / [`crate::shell`]) and are enforced through
//! [`AdmissionTarget::pending`]: a queue
//! sink reports what it still holds, a synchronous sink reports `0` because it
//! holds nothing between calls. The *refusal strings* for a full queue are shared
//! and byte-identical; a synchronous sink simply never reaches them.

use std::collections::VecDeque;

use cad_app::{StandardView, ToolParameter, ToolValue};

use crate::popup::PopupAck;
use crate::shell::check_command;

use super::admission::{
    check_layer_write, check_selected_property, check_selected_text, check_setting,
    check_standard_view, check_tool_parameter, check_tool_text, parse_layer_property_write,
    parse_property_write, parse_standard_view, parse_tool_parameter,
};
use super::commands::{admissible_names, alias_pairs};
use super::layers::{CadLayerWrite, CadLayers};
use super::settings::{CadSettingKey, CadSettingValue, CadSettings};

/// The stable reason a per-host read answers before any CAD state exists.
///
/// Shared so the desktop (`cad_settings`/`cad_layers` before the first owner tick)
/// and any other host answer the same word for "no snapshot yet".
pub const CAD_WAITING: &str = "cad_waiting";

/// The stable refusal when a host has no modal-popup seam registered.
///
/// The desktop reaches this when its `ShellPopupHost` is not installed; the
/// `dom-canvas` carriers reach it always, because an in-DOM popup overlay needs a
/// native host that they do not have. One string, one meaning.
pub const POPUP_HOST_UNAVAILABLE: &str =
    "popup.host_unavailable: no desktop popup host is registered";

/// Admission result of a queued/applied command: **accepted**, not completed.
///
/// `{accepted:true}` is the whole promise: on the desktop it means *queued* onto
/// the owner thread, and on a synchronous sink it means *applied*. The real
/// outcome surfaces through `cad_status.error`. It lives here (rather than in the
/// Tauri layer) so both hosts answer the identical wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

impl CadCommandAck {
    /// The only acknowledgement this layer produces.
    pub fn accepted() -> Self {
        Self { accepted: true }
    }
}

/// One AutoCAD-style abbreviation the router accepts, as the chrome consumes it.
///
/// `command` is the canonical selector the alias normalizes onto (never another
/// alias), so a frontend can display the target without re-resolving anything.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadCommandAlias {
    /// The typed abbreviation, e.g. `L`.
    pub alias: String,
    /// The canonical selector it resolves to, e.g. `LINE`.
    pub command: String,
}

/// The bounded channel an admission is about (used for the queue bound only).
///
/// One variant per queue family member; a host that keeps no queue answers `0`
/// for every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The name-only command queue (`shell.cad_commands`).
    Command,
    /// Typed tool parameters (ADR 0016 decision A).
    ToolParameter,
    /// Terminal tool texts (ADR 0019 decision A.1).
    ToolText,
    /// Replacement texts for the current single selection (ADR 0020 amendment B).
    SelectedText,
    /// Property writes for the current selection.
    SelectedProperty,
    /// Typed standard views.
    StandardView,
    /// Typed settings writes.
    Setting,
    /// Typed layer writes (visibility, restore, active and the table writes).
    LayerWrite,
}

/// The **sink seam**: how one host stores or applies an already-admitted request.
///
/// The rules are *not* here. [`super::admission`] and
/// [`crate::shell::check_command`] own the gate order, the bounds and every
/// diagnostic; a `wire::*` function runs them and then calls exactly one method
/// below with the parsed value. Implementations therefore contain no policy:
///
/// * a queue sink pushes onto its bounded queue (the desktop, which cannot touch
///   the `!Send` session off the owner thread);
/// * a direct sink calls the matching `CadSession` method and returns the
///   session's own refusal verbatim (`dom-canvas`, single-threaded wasm).
///
/// Do not call these methods directly: they assume the shared gate already ran
/// (that is what [`AdmissionTarget::pending`] and the gate queries are for).
pub trait AdmissionTarget {
    /// Whether the CAD session path is enabled (`false` in probe mode).
    fn cad_mode(&self) -> bool;

    /// Whether a modal popup currently owns host input (`popup.modal_open`).
    fn modal_open(&self) -> bool;

    /// How many requests of `channel` the target still holds undrained.
    ///
    /// A queue sink reports its real length (that is what bounds the queue); a
    /// synchronous sink reports `0` because nothing is ever pending there.
    fn pending(&self, channel: Channel) -> usize;

    /// Store or apply one **already normalized** command line
    /// ([`crate::shell::check_command`] produced it: the canonical selector, or
    /// `"<SELECTOR> <argument>"`).
    fn command(&mut self, normalized: String) -> Result<(), String>;

    /// Store or apply one typed tool parameter.
    fn tool_parameter(&mut self, parameter: ToolParameter, value: ToolValue) -> Result<(), String>;

    /// Store or apply one terminal tool text.
    fn tool_text(&mut self, text: String) -> Result<(), String>;

    /// Store or apply one replacement text for the current single selection.
    fn selected_text(&mut self, text: String) -> Result<(), String>;

    /// Store or apply one property write for the current selection.
    fn selected_property(&mut self, key: String, value: String) -> Result<(), String>;

    /// Store or apply one typed standard view.
    fn standard_view(&mut self, view: StandardView) -> Result<(), String>;

    /// Store or apply one typed setting.
    fn setting(&mut self, key: CadSettingKey, value: CadSettingValue) -> Result<(), String>;

    /// Store or apply one typed layer write.
    fn layer_write(&mut self, write: CadLayerWrite) -> Result<(), String>;
}

/// The **read seam** for the two control-panel reads (`cad_settings`,
/// `cad_layers`).
///
/// These are not sink methods: they are projections, and each host serves them
/// from the only source it has.
///
/// * Desktop: the owner tick mirrors `CadSession::settings()`/`layers()` into
///   `Shared`, because the session is `!Send` and the IPC thread must never touch
///   it. `None` before the first tick — the read then answers [`CAD_WAITING`].
/// * `dom-canvas`: the session lives in the same thread as the caller, so the
///   answer is the session's own `settings()`/`layers()` call.
///
/// [`layers`] keeps the desktop's existing `Option<Result<..>>` shape: `None`
/// means "no snapshot yet" ([`CAD_WAITING`]) while `Some(Err(..))` relays the
/// session's own projection refusal instead of serving stale rows.
pub trait ControlReads {
    /// The effective settings snapshot, or `None` before any exists.
    fn settings(&self) -> Option<CadSettings>;

    /// The layer projection, or `None` before any exists.
    fn layers(&self) -> Option<Result<CadLayers, String>>;
}

/// The seam the two popup commands go through.
///
/// The popup **state machine** and the four-trigger lifecycle are
/// [`crate::popup`]; a host only supplies the platform half (the Linux/Windows
/// hosts marshal the open onto their main thread and derive the `textEdit`
/// payload there). A host without that half — every `dom-canvas` carrier — must
/// use [`NoPopups`], whose answer is the shared [`POPUP_HOST_UNAVAILABLE`], never
/// a fabricated acknowledgement.
pub trait PopupTarget {
    /// Open (or focus) the allowlisted popup for `kind`.
    fn open(&mut self, kind: &str) -> Result<PopupAck, String>;

    /// Close the open popup (idempotent) and clear the modal gate.
    fn close(&mut self) -> Result<(), String>;
}

/// The honest [`PopupTarget`] of a host that has no popup seam at all.
///
/// Both commands refuse with [`POPUP_HOST_UNAVAILABLE`]; the state machine is
/// never entered, so no gate is set and nothing is half-open.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoPopups;

impl PopupTarget for NoPopups {
    fn open(&mut self, _kind: &str) -> Result<PopupAck, String> {
        Err(POPUP_HOST_UNAVAILABLE.to_string())
    }

    fn close(&mut self) -> Result<(), String> {
        Err(POPUP_HOST_UNAVAILABLE.to_string())
    }
}

/// The bounded-queue family of one desktop host, as the sink seam sees it.
///
/// A borrow of the host's own queues (no copy, no second authority). The desktop
/// `Shared` builds one of these per IPC call; see [`QueueSink::new`].
pub struct QueueFamily<'a> {
    /// `shell.cad_commands` — normalized command lines.
    pub commands: &'a mut Vec<String>,
    /// `cad_tool_parameters`.
    pub tool_parameters: &'a mut Vec<(ToolParameter, ToolValue)>,
    /// `cad_tool_texts`.
    pub tool_texts: &'a mut Vec<String>,
    /// `cad_selected_texts`.
    pub selected_texts: &'a mut Vec<String>,
    /// `cad_selected_properties`.
    pub selected_properties: &'a mut Vec<(String, String)>,
    /// `cad_standard_views`.
    pub standard_views: &'a mut VecDeque<StandardView>,
    /// `cad_setting_writes`.
    pub setting_writes: &'a mut VecDeque<(CadSettingKey, CadSettingValue)>,
    /// `cad_layer_writes`.
    pub layer_writes: &'a mut VecDeque<CadLayerWrite>,
}

/// The **queue sink**: stores into a desktop host's bounded queue family.
///
/// It carries the host's gate inputs as values (read once when the sink is
/// built, exactly as each command used to read them under its own lock) and a
/// borrow of the queues, so admission and store happen in one uninterrupted
/// step. Every store is a plain `push`: the shared gate already ran.
pub struct QueueSink<'a> {
    cad_mode: bool,
    modal_open: bool,
    queues: QueueFamily<'a>,
}

impl<'a> QueueSink<'a> {
    /// Sink over one host's queue family with its current gate inputs.
    pub fn new(cad_mode: bool, modal_open: bool, queues: QueueFamily<'a>) -> Self {
        Self {
            cad_mode,
            modal_open,
            queues,
        }
    }
}

impl AdmissionTarget for QueueSink<'_> {
    fn cad_mode(&self) -> bool {
        self.cad_mode
    }

    fn modal_open(&self) -> bool {
        self.modal_open
    }

    fn pending(&self, channel: Channel) -> usize {
        match channel {
            Channel::Command => self.queues.commands.len(),
            Channel::ToolParameter => self.queues.tool_parameters.len(),
            Channel::ToolText => self.queues.tool_texts.len(),
            Channel::SelectedText => self.queues.selected_texts.len(),
            Channel::SelectedProperty => self.queues.selected_properties.len(),
            Channel::StandardView => self.queues.standard_views.len(),
            Channel::Setting => self.queues.setting_writes.len(),
            Channel::LayerWrite => self.queues.layer_writes.len(),
        }
    }

    fn command(&mut self, normalized: String) -> Result<(), String> {
        self.queues.commands.push(normalized);
        Ok(())
    }

    fn tool_parameter(&mut self, parameter: ToolParameter, value: ToolValue) -> Result<(), String> {
        self.queues.tool_parameters.push((parameter, value));
        Ok(())
    }

    fn tool_text(&mut self, text: String) -> Result<(), String> {
        self.queues.tool_texts.push(text);
        Ok(())
    }

    fn selected_text(&mut self, text: String) -> Result<(), String> {
        self.queues.selected_texts.push(text);
        Ok(())
    }

    fn selected_property(&mut self, key: String, value: String) -> Result<(), String> {
        self.queues.selected_properties.push((key, value));
        Ok(())
    }

    fn standard_view(&mut self, view: StandardView) -> Result<(), String> {
        self.queues.standard_views.push_back(view);
        Ok(())
    }

    fn setting(&mut self, key: CadSettingKey, value: CadSettingValue) -> Result<(), String> {
        self.queues.setting_writes.push_back((key, value));
        Ok(())
    }

    fn layer_write(&mut self, write: CadLayerWrite) -> Result<(), String> {
        self.queues.layer_writes.push_back(write);
        Ok(())
    }
}

/// The [`ControlReads`] of a desktop host: the owner-tick mirror.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueReads<'a> {
    /// `Shared::cad_settings`.
    pub settings: Option<&'a CadSettings>,
    /// `Shared::cad_layers`.
    pub layers: Option<&'a Result<CadLayers, String>>,
}

impl<'a> QueueReads<'a> {
    /// Reads over one host's mirrored projections (`None` = no tick yet).
    pub fn new(
        settings: Option<&'a CadSettings>,
        layers: Option<&'a Result<CadLayers, String>>,
    ) -> Self {
        Self { settings, layers }
    }
}

impl ControlReads for QueueReads<'_> {
    fn settings(&self) -> Option<CadSettings> {
        self.settings.cloned()
    }

    fn layers(&self) -> Option<Result<CadLayers, String>> {
        self.layers.cloned()
    }
}

// ---------------------------------------------------------------------------
// The wire commands. One function per IPC name; the desktop `#[tauri::command]`
// wrappers and the `dom-canvas` dispatcher both call exactly these.
// ---------------------------------------------------------------------------

/// `cad_commands`: every selector the router admits (`admissible_names`).
pub fn commands() -> Vec<&'static str> {
    admissible_names()
}

/// `cad_command_aliases`: the alias table the chrome offers (`alias_pairs`).
pub fn aliases() -> Vec<CadCommandAlias> {
    alias_pairs()
        .into_iter()
        .map(|(alias, command)| CadCommandAlias {
            alias: alias.to_string(),
            command: command.to_string(),
        })
        .collect()
}

/// `cad_command`: admit one command line into the bounded command channel.
///
/// An unknown name is refused immediately (never queued). The one argument form
/// is admitted by its base selector (see [`crate::shell::check_command`]).
pub fn command(target: &mut dyn AdmissionTarget, text: &str) -> Result<CadCommandAck, String> {
    let normalized = check_command(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::Command),
        text,
    )?;
    target.command(normalized)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_set_tool_parameter`: admit one typed tool parameter.
///
/// The wire form is exactly one of `number`/`flag`; a malformed request (an
/// unknown key, neither or both) is refused before the queue. The parameter's own
/// range/kind is the session's authority on apply.
pub fn set_tool_parameter(
    target: &mut dyn AdmissionTarget,
    parameter: &str,
    number: Option<f64>,
    flag: Option<bool>,
) -> Result<CadCommandAck, String> {
    let (parameter, value) = parse_tool_parameter(parameter, number, flag)?;
    check_tool_parameter(target.cad_mode(), target.pending(Channel::ToolParameter))?;
    target.tool_parameter(parameter, value)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_set_tool_text`: admit one terminal tool text.
pub fn set_tool_text(
    target: &mut dyn AdmissionTarget,
    text: &str,
) -> Result<CadCommandAck, String> {
    check_tool_text(target.cad_mode(), target.pending(Channel::ToolText), text)?;
    target.tool_text(text.to_string())?;
    Ok(CadCommandAck::accepted())
}

/// `cad_set_selected_text`: admit one replacement text for the current selection.
///
/// Deliberately **not** behind the modal popup gate: this is the text-edit
/// popup's own write channel.
pub fn set_selected_text(
    target: &mut dyn AdmissionTarget,
    text: &str,
) -> Result<CadCommandAck, String> {
    check_selected_text(
        target.cad_mode(),
        target.pending(Channel::SelectedText),
        text,
    )?;
    target.selected_text(text.to_string())?;
    Ok(CadCommandAck::accepted())
}

/// `cad_set_selected_property`: queue one property write for the selection.
///
/// Only the key is checked here (`property.unsupported_key`); the value's grammar
/// can depend on the drawing's own tables, so it is the session's authority.
pub fn set_selected_property(
    target: &mut dyn AdmissionTarget,
    key: &str,
    value: &str,
) -> Result<CadCommandAck, String> {
    let (key, value) = parse_property_write(key, value)?;
    check_selected_property(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::SelectedProperty),
    )?;
    target.selected_property(key, value)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_standard_view`: admit one typed standard view.
pub fn standard_view(
    target: &mut dyn AdmissionTarget,
    view: &str,
) -> Result<CadCommandAck, String> {
    let view = parse_standard_view(view)?;
    check_standard_view(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::StandardView),
    )?;
    target.standard_view(view)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_settings`: the effective settings snapshot, or [`CAD_WAITING`].
///
/// Served by the host: the desktop mirrors the owner thread's snapshot (the
/// session is `!Send`), a synchronous carrier reads `CadSession::settings()`
/// directly. See [`ControlReads`].
pub fn settings(reads: &dyn ControlReads) -> Result<CadSettings, String> {
    reads.settings().ok_or_else(|| CAD_WAITING.to_string())
}

/// `cad_set_setting`: admit one typed setting for the owner thread.
///
/// Deliberately **not** behind the modal popup gate, so the Options dialog can
/// write while it is open.
pub fn set_setting(
    target: &mut dyn AdmissionTarget,
    key: &str,
    value: &serde_json::Value,
) -> Result<CadCommandAck, String> {
    let (key, value) = parse_setting_request(key, value)?;
    check_setting(
        target.cad_mode(),
        target.pending(Channel::Setting),
        &key,
        &value,
    )?;
    target.setting(key, value)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_layers`: the layer-panel projection, or [`CAD_WAITING`].
///
/// A mirror refusal (`Some(Err(..))`) is relayed, never replaced by stale rows.
pub fn layers(reads: &dyn ControlReads) -> Result<CadLayers, String> {
    reads.layers().ok_or_else(|| CAD_WAITING.to_string())?
}

/// `cad_set_layer_visible`: queue one temporary layer visibility.
pub fn set_layer_visible(
    target: &mut dyn AdmissionTarget,
    name: &str,
    visible: bool,
) -> Result<CadCommandAck, String> {
    layer_write(
        target,
        CadLayerWrite::SetVisible {
            name: name.to_string(),
            visible,
        },
    )
}

/// `cad_restore_layers`: queue a restore of every temporary layer override.
pub fn restore_layers(target: &mut dyn AdmissionTarget) -> Result<CadCommandAck, String> {
    layer_write(target, CadLayerWrite::Restore)
}

/// `cad_set_active_layer`: queue one active-layer change.
pub fn set_active_layer(
    target: &mut dyn AdmissionTarget,
    name: &str,
) -> Result<CadCommandAck, String> {
    layer_write(
        target,
        CadLayerWrite::SetActive {
            name: name.to_string(),
        },
    )
}

/// `cad_create_layer`: queue one layer-table create.
pub fn create_layer(target: &mut dyn AdmissionTarget, name: &str) -> Result<CadCommandAck, String> {
    layer_write(
        target,
        CadLayerWrite::Create {
            name: name.to_string(),
        },
    )
}

/// `cad_rename_layer`: queue one layer rename.
pub fn rename_layer(
    target: &mut dyn AdmissionTarget,
    layer: &str,
    name: &str,
) -> Result<CadCommandAck, String> {
    layer_write(
        target,
        CadLayerWrite::Rename {
            layer: layer.to_string(),
            name: name.to_string(),
        },
    )
}

/// `cad_delete_layer`: queue one layer delete.
pub fn delete_layer(
    target: &mut dyn AdmissionTarget,
    layer: &str,
) -> Result<CadCommandAck, String> {
    layer_write(
        target,
        CadLayerWrite::Delete {
            layer: layer.to_string(),
        },
    )
}

/// `cad_set_layer_property`: queue one layer attribute write.
///
/// The key is checked before the queue (`layer.property.unsupported`, naming the
/// key); the value is the session's call, because it may depend on the drawing's
/// linetype table.
pub fn set_layer_property(
    target: &mut dyn AdmissionTarget,
    layer: &str,
    key: &str,
    value: &str,
) -> Result<CadCommandAck, String> {
    let (layer, key, value) = parse_layer_property_write(layer, key, value)?;
    layer_write(target, CadLayerWrite::SetProperty { layer, key, value })
}

/// Every layer mutation shares this one admission and admission order.
fn layer_write(
    target: &mut dyn AdmissionTarget,
    write: CadLayerWrite,
) -> Result<CadCommandAck, String> {
    check_layer_write(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::LayerWrite),
    )?;
    target.layer_write(write)?;
    Ok(CadCommandAck::accepted())
}

/// `cad_open_popup`: open the allowlisted modal popup, or report the open one.
pub fn open_popup(popups: &mut dyn PopupTarget, kind: &str) -> Result<PopupAck, String> {
    popups.open(kind)
}

/// `cad_close_popup`: close the open popup (an idempotent success).
pub fn close_popup(popups: &mut dyn PopupTarget) -> Result<CadCommandAck, String> {
    popups.close()?;
    Ok(CadCommandAck::accepted())
}

/// Parse one `cad_set_setting` request into the typed key/value pair.
///
/// The wire is `{ key: <stable camelCase key>, value: <bool|string|number> }`.
/// An unknown key is refused with `settings.unknown_key`; a wrong value shape is
/// refused with `settings.invalid_value`; a `plotFormat` other than `svg`/`pdf`
/// is refused by the key table's own validation, as is a colour that is not
/// `#rrggbb`, a crosshair percentage outside `1..=100` and a GPU budget outside
/// `10..=100` (the same authority the owner thread uses). Nothing is coerced and
/// no file is touched here.
///
/// A JSON number has two typed shapes: a non-negative integer that fits `u32`
/// becomes [`CadSettingValue::Number`]; a fractional one becomes
/// [`CadSettingValue::Real`] (never truncated into the integer shape, so
/// `snapSpacing: 2.5` cannot become `2`). Negative integers and integers outside
/// `u32` stay refused at this boundary: no key accepts them, and widening them
/// into `Real` would silently re-validate a value that was never a legal number.
/// Which keys accept which shape — and that a float on a boolean-or-integer key
/// is still [`super::SETTINGS_INVALID_VALUE`] — is [`CadSettingKey::validate`]'s
/// decision, not this parser's.
pub fn parse_setting_request(
    key: &str,
    value: &serde_json::Value,
) -> Result<(CadSettingKey, CadSettingValue), String> {
    let key = CadSettingKey::parse(key).ok_or_else(|| super::SETTINGS_UNKNOWN_KEY.to_string())?;
    let value = match value {
        serde_json::Value::Bool(enabled) => CadSettingValue::Bool(*enabled),
        serde_json::Value::String(text) => CadSettingValue::Text(text.clone()),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
            {
                CadSettingValue::Number(integer)
            } else if number.is_f64() {
                // A real that is not a non-negative `u32` integer. A non-finite
                // real cannot arrive from JSON, but the shape is refused rather
                // than carried to the host as a special value.
                match number.as_f64() {
                    Some(real) if real.is_finite() => CadSettingValue::Real(real),
                    _ => return Err(super::SETTINGS_INVALID_VALUE.to_string()),
                }
            } else {
                // A negative or out-of-`u32` integer: no key accepts it.
                return Err(super::SETTINGS_INVALID_VALUE.to_string());
            }
        }
        _ => return Err(super::SETTINGS_INVALID_VALUE.to_string()),
    };
    key.validate(&value).map_err(str::to_string)?;
    Ok((key, value))
}
