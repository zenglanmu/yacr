//! The **`dom-canvas` transport**: one `invoke(name, args_json)` entry over the
//! shared command semantics (`cad_ui_core::session::wire`).
//!
//! # What this module is
//!
//! The desktop hosts reach CAD through Tauri IPC: `apps/desktop-ui` calls
//! `invoke("cad_set_layer_visible", {...})`, Tauri deserializes the arguments into
//! a `#[tauri::command]` signature, and `cad-desktop-shell` pushes the request
//! onto a bounded queue for the owner thread. A browser (or an Android WebView, or
//! the Linux X11 `dom-canvas` path) has **no IPC**, so the same JS call has to be
//! served in-process.
//!
//! This module is that transport, and only that transport:
//!
//! * [`dispatch`] decodes `args_json` into **exactly the typed values the Tauri
//!   signature produces** (one small `serde` struct per command) and calls the
//!   *same* `wire::*` function the desktop command calls — one function per
//!   command, no second copy of the rules;
//! * [`DirectSink`] is the [`AdmissionTarget`] of a host that owns its session on
//!   the calling thread: it **applies** each already-admitted request through the
//!   `CadSession` method the desktop owner tick would call, and returns the
//!   session's own refusal verbatim;
//! * [`DirectReads`] serves `cad_settings`/`cad_layers` from the session itself
//!   (the desktop serves them from the owner-tick mirror, because there the
//!   session is `!Send`).
//!
//! Nothing here decides *what* a command means, and nothing here may turn a
//! refusal into a success: the admission verdict, the gate order, the queue
//! bounds and every diagnostic string come from `cad_ui_core`.
//!
//! # The wire contract
//!
//! ```text
//! invoke(name, args_json) -> {"ok":true,"value":<answer>}
//!                         |  {"ok":false,"error":"<verbatim refusal>"}
//! ```
//!
//! `value` is the answer the desktop command returns for the same name (`{accepted:
//! true}` for a mutation, the `cad_settings`/`cad_layers` object for the reads, the
//! `PopupAck` for `cad_open_popup`). `error` is byte-identical to the desktop
//! refusal for a malformed request; for a request the **session** refuses, it is
//! the session's own text (see the crate docs of
//! `cad_ui_core::session::wire` for the queued-vs-applied timing difference).
//!
//! An unknown name is refused with `unsupported-runtime: <name>` — the stable
//! machine prefix the carriers use — never an empty success.
//!
//! # Feature boundary
//!
//! There is none any more: the production session ([`crate::live`] /
//! [`crate::page`]) is what serves this transport, so `invoke` exists on every
//! wasm build and answers from the live session. A build (or a moment) with **no
//! live session** still answers the two vocabulary reads from the shared tables —
//! they are pure — and refuses every other known command with [`NO_SESSION`], never
//! with a fabricated acknowledgement.

use cad_app::{StandardView, ToolParameter, ToolValue};
use cad_ui_core::session::wire::{self, AdmissionTarget, CadCommandAck, ControlReads, PopupTarget};
use cad_ui_core::session::{
    CadLayerWrite, CadLayers, CadSession, CadSettingKey, CadSettingValue, CadSettings,
};
use serde::Deserialize;
use serde_json::Value;

/// Stable machine prefix of every name this transport does not serve.
pub const UNSUPPORTED_RUNTIME: &str = "unsupported-runtime";

/// Stable refusal of a session-backed command when this carrier has no live CAD
/// session (not started yet, still starting, or the failed state).
///
/// A caller must never read this as a success: the whole point of the envelope is
/// that a request either ran on the session or says why it did not.
pub const NO_SESSION: &str = "no live CAD session on this carrier (start the canvas session first)";

/// The refusal of a name this transport has no wire command for.
///
/// The prefix comes first so a caller can key on it; the sentence names the
/// selector so a log is readable.
pub fn unsupported_runtime(name: &str) -> String {
    format!("{UNSUPPORTED_RUNTIME}: {name}: no such wire command on this carrier")
}

// ---------------------------------------------------------------------------
// The sink
// ---------------------------------------------------------------------------

/// The [`AdmissionTarget`] of a host whose session is on the calling thread.
///
/// Every method applies the request through the session method the desktop owner
/// tick calls (`CadSession::command`, `CadLayerWrite::apply`, …), so the two hosts
/// resolve the same names, run the same tables and produce the same refusals.
///
/// `pending` is always `0`: a synchronous sink holds nothing between calls, so the
/// queue bound (which exists for the desktop's cross-thread buffer) is never
/// reached here. `cad_mode`/`modal_open` are the host's declared gate inputs — a
/// live wasm session means CAD is enabled, and `modal_open` is the projected popup
/// gate ([`crate::live::PopupSlot::modal_open`]), exactly like the desktop's.
pub struct DirectSink<'a> {
    session: &'a CadSession,
    cad_mode: bool,
    modal_open: bool,
    /// The last refusal an *apply* produced, so a caller can tell "the shared gate
    /// refused" from "the session refused" without re-running anything.
    refusal: Option<String>,
}

impl<'a> DirectSink<'a> {
    /// The production sink: CAD enabled, no modal popup.
    ///
    /// [`Self::with_gates`] is what the production entry uses (it passes the live
    /// popup gate); this constructor is the CAD-enabled/no-modal case on its own.
    pub fn new(session: &'a CadSession) -> Self {
        Self::with_gates(session, true, false)
    }

    /// A sink with explicit gate inputs, for the equivalence harness.
    ///
    /// The production entry always uses [`Self::new`].
    pub fn with_gates(session: &'a CadSession, cad_mode: bool, modal_open: bool) -> Self {
        Self {
            session,
            cad_mode,
            modal_open,
            refusal: None,
        }
    }

    /// Take the refusal the last apply produced (`None` when it succeeded).
    ///
    /// Diagnostic only: the same text was already returned by the failing call.
    pub fn take_refusal(&mut self) -> Option<String> {
        self.refusal.take()
    }

    /// Remember an apply outcome and hand it back unchanged.
    fn applied(&mut self, outcome: Result<(), String>) -> Result<(), String> {
        if let Err(error) = &outcome {
            self.refusal = Some(error.clone());
        }
        outcome
    }
}

impl AdmissionTarget for DirectSink<'_> {
    fn cad_mode(&self) -> bool {
        self.cad_mode
    }

    fn modal_open(&self) -> bool {
        self.modal_open
    }

    fn pending(&self, _channel: wire::Channel) -> usize {
        // A synchronous sink never holds a request: it applied it or refused it.
        0
    }

    fn command(&mut self, normalized: String) -> Result<(), String> {
        let outcome = self.session.command(&normalized);
        self.applied(outcome)
    }

    fn tool_parameter(&mut self, parameter: ToolParameter, value: ToolValue) -> Result<(), String> {
        let outcome = self.session.set_tool_parameter(parameter, value);
        self.applied(outcome)
    }

    fn tool_text(&mut self, text: String) -> Result<(), String> {
        let outcome = self.session.set_tool_text(&text);
        self.applied(outcome)
    }

    fn selected_text(&mut self, text: String) -> Result<(), String> {
        let outcome = self.session.set_selected_text(&text);
        self.applied(outcome)
    }

    fn selected_property(&mut self, key: String, value: String) -> Result<(), String> {
        let outcome = self.session.set_selected_property(&key, &value);
        self.applied(outcome)
    }

    fn standard_view(&mut self, view: StandardView) -> Result<(), String> {
        let outcome = self.session.standard_view(view);
        self.applied(outcome)
    }

    fn setting(&mut self, key: CadSettingKey, value: CadSettingValue) -> Result<(), String> {
        let outcome = self.session.apply_setting(key, value).map(|_settings| ());
        self.applied(outcome)
    }

    fn layer_write(&mut self, write: CadLayerWrite) -> Result<(), String> {
        let outcome = write.apply(self.session);
        self.applied(outcome)
    }
}

/// The [`ControlReads`] of a host that owns its session on this thread.
///
/// Both answers are the session's own; there is no mirror to keep in step and no
/// `None` state (a synchronous carrier has a session whenever it can answer at
/// all). `layers()` relays the session's own projection refusal unchanged.
pub struct DirectReads<'a> {
    session: &'a CadSession,
}

impl<'a> DirectReads<'a> {
    pub fn new(session: &'a CadSession) -> Self {
        Self { session }
    }
}

impl ControlReads for DirectReads<'_> {
    fn settings(&self) -> Option<CadSettings> {
        Some(self.session.settings())
    }

    fn layers(&self) -> Option<Result<CadLayers, String>> {
        Some(self.session.layers())
    }
}

// ---------------------------------------------------------------------------
// Argument decoding (one struct per command, mirroring the Tauri signature)
// ---------------------------------------------------------------------------

/// `cad_command(shared, command: String)`.
#[derive(Debug, Deserialize)]
pub struct CommandArgs {
    pub command: String,
}

/// `cad_set_tool_parameter(shared, parameter: String, number: Option<f64>, flag:
/// Option<bool>)`.
#[derive(Debug, Deserialize)]
pub struct ToolParameterArgs {
    pub parameter: String,
    #[serde(default)]
    pub number: Option<f64>,
    #[serde(default)]
    pub flag: Option<bool>,
}

/// `cad_set_tool_text(shared, text: String)`.
#[derive(Debug, Deserialize)]
pub struct TextArgs {
    pub text: String,
}

/// `cad_set_selected_property(shared, key: String, value: String)`.
#[derive(Debug, Deserialize)]
pub struct PropertyArgs {
    pub key: String,
    pub value: String,
}

/// `cad_standard_view(shared, view: String)`.
#[derive(Debug, Deserialize)]
pub struct StandardViewArgs {
    pub view: String,
}

/// `cad_set_setting(shared, key: String, value: serde_json::Value)`.
#[derive(Debug, Deserialize)]
pub struct SettingArgs {
    pub key: String,
    pub value: Value,
}

/// `cad_set_layer_visible(shared, name: String, visible: bool)`.
#[derive(Debug, Deserialize)]
pub struct LayerVisibleArgs {
    pub name: String,
    pub visible: bool,
}

/// `cad_set_active_layer(shared, name: String)` / `cad_create_layer(shared,
/// name: String)`.
#[derive(Debug, Deserialize)]
pub struct LayerNameArgs {
    pub name: String,
}

/// `cad_rename_layer(shared, layer: String, name: String)`.
#[derive(Debug, Deserialize)]
pub struct LayerRenameArgs {
    pub layer: String,
    pub name: String,
}

/// `cad_delete_layer(shared, layer: String)`.
#[derive(Debug, Deserialize)]
pub struct LayerArgs {
    pub layer: String,
}

/// `cad_set_layer_property(shared, layer: String, key: String, value: String)`.
#[derive(Debug, Deserialize)]
pub struct LayerPropertyArgs {
    pub layer: String,
    pub key: String,
    pub value: String,
}

/// `cad_open_popup(shared, kind: String)`.
#[derive(Debug, Deserialize)]
pub struct PopupArgs {
    pub kind: String,
}

/// Decode one argument object, naming the command when it does not fit.
///
/// A wrong shape is a **transport** error (the Tauri IPC would refuse it before
/// the command body too), so it is refused here and never reaches the shared gate.
fn args<T: serde::de::DeserializeOwned>(name: &str, args_json: &str) -> Result<T, String> {
    let text = if args_json.trim().is_empty() {
        "{}"
    } else {
        args_json
    };
    serde_json::from_str(text).map_err(|error| format!("invalid-args: {name}: {error}"))
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Serve one wire command by name, exactly as the desktop command of that name.
///
/// The name set is the desktop's: `cad_command`, `cad_command_aliases`,
/// `cad_commands`, `cad_settings`, `cad_layers`, `cad_set_tool_parameter`,
/// `cad_set_tool_text`, `cad_set_selected_text`, `cad_set_selected_property`,
/// `cad_standard_view`, `cad_set_setting`, `cad_set_layer_visible`,
/// `cad_restore_layers`, `cad_set_active_layer`, `cad_create_layer`,
/// `cad_rename_layer`, `cad_delete_layer`, `cad_set_layer_property`,
/// `cad_open_popup`, `cad_close_popup`.
///
/// Every mutation arm calls the `wire::*` function of the same name - the one the
/// desktop `#[tauri::command]` calls - and answers with the same
/// `{accepted:true}` object. A name outside the set is [`unsupported_runtime`],
/// never an empty success.
pub fn dispatch(
    name: &str,
    args_json: &str,
    sink: &mut dyn AdmissionTarget,
    reads: &dyn ControlReads,
    popups: &mut dyn PopupTarget,
) -> Result<Value, String> {
    match name {
        "cad_command" => admit(wire::command(
            sink,
            &args::<CommandArgs>(name, args_json)?.command,
        )),
        "cad_command_aliases" => Ok(serde_json::json!(wire::aliases())),
        "cad_commands" => Ok(serde_json::json!(wire::commands())),
        "cad_settings" => Ok(serde_json::json!(wire::settings(reads)?)),
        "cad_layers" => Ok(serde_json::json!(wire::layers(reads)?)),
        "cad_set_tool_parameter" => {
            let parsed: ToolParameterArgs = args(name, args_json)?;
            admit(wire::set_tool_parameter(
                sink,
                &parsed.parameter,
                parsed.number,
                parsed.flag,
            ))
        }
        "cad_set_tool_text" => admit(wire::set_tool_text(
            sink,
            &args::<TextArgs>(name, args_json)?.text,
        )),
        "cad_set_selected_text" => admit(wire::set_selected_text(
            sink,
            &args::<TextArgs>(name, args_json)?.text,
        )),
        "cad_set_selected_property" => {
            let parsed: PropertyArgs = args(name, args_json)?;
            admit(wire::set_selected_property(
                sink,
                &parsed.key,
                &parsed.value,
            ))
        }
        "cad_standard_view" => admit(wire::standard_view(
            sink,
            &args::<StandardViewArgs>(name, args_json)?.view,
        )),
        "cad_set_setting" => {
            let parsed: SettingArgs = args(name, args_json)?;
            admit(wire::set_setting(sink, &parsed.key, &parsed.value))
        }
        "cad_set_layer_visible" => {
            let parsed: LayerVisibleArgs = args(name, args_json)?;
            admit(wire::set_layer_visible(sink, &parsed.name, parsed.visible))
        }
        "cad_restore_layers" => admit(wire::restore_layers(sink)),
        "cad_set_active_layer" => admit(wire::set_active_layer(
            sink,
            &args::<LayerNameArgs>(name, args_json)?.name,
        )),
        "cad_create_layer" => admit(wire::create_layer(
            sink,
            &args::<LayerNameArgs>(name, args_json)?.name,
        )),
        "cad_rename_layer" => {
            let parsed: LayerRenameArgs = args(name, args_json)?;
            admit(wire::rename_layer(sink, &parsed.layer, &parsed.name))
        }
        "cad_delete_layer" => admit(wire::delete_layer(
            sink,
            &args::<LayerArgs>(name, args_json)?.layer,
        )),
        "cad_set_layer_property" => {
            let parsed: LayerPropertyArgs = args(name, args_json)?;
            admit(wire::set_layer_property(
                sink,
                &parsed.layer,
                &parsed.key,
                &parsed.value,
            ))
        }
        "cad_open_popup" => {
            let parsed: PopupArgs = args(name, args_json)?;
            let popup = wire::open_popup(popups, &parsed.kind)?;
            serde_json::to_value(popup).map_err(|error| error.to_string())
        }
        "cad_close_popup" => admit(wire::close_popup(popups)),
        other => Err(unsupported_runtime(other)),
    }
}

/// The wire answer of a mutation: the shared `{accepted:true}` object.
fn admit(ack: Result<CadCommandAck, String>) -> Result<Value, String> {
    let ack = ack?;
    serde_json::to_value(ack).map_err(|error| error.to_string())
}

/// The `{"ok":...}` envelope of the `invoke` export.
///
/// One shape on every target, so the JS side decodes one thing.
pub fn envelope(result: Result<Value, String>) -> String {
    match result {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
        Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
    }
}

// ---------------------------------------------------------------------------
// The export
// ---------------------------------------------------------------------------

/// Every wire command this transport serves: exactly the desktop command set.
///
/// Kept as a list (not derived from the match arms) so the "no session in this
/// build" refusal can tell a *known* command from an unknown name, and so a test
/// can assert the two sets are the same.
pub const WIRE_COMMANDS: [&str; 20] = [
    "cad_command",
    "cad_command_aliases",
    "cad_commands",
    "cad_settings",
    "cad_layers",
    "cad_set_tool_parameter",
    "cad_set_tool_text",
    "cad_set_selected_text",
    "cad_set_selected_property",
    "cad_standard_view",
    "cad_set_setting",
    "cad_set_layer_visible",
    "cad_restore_layers",
    "cad_set_active_layer",
    "cad_create_layer",
    "cad_rename_layer",
    "cad_delete_layer",
    "cad_set_layer_property",
    "cad_open_popup",
    "cad_close_popup",
];

/// Whether `name` is one of [`WIRE_COMMANDS`].
pub fn is_wire_command(name: &str) -> bool {
    WIRE_COMMANDS.contains(&name)
}

/// The answers a carrier **without a live CAD session** can still give honestly.
///
/// The two vocabulary reads are pure functions of the shared tables, so they are
/// served; every other known command is the explicit [`NO_SESSION`] refusal and an
/// unknown name keeps [`unsupported_runtime`]. Nothing is answered as a success
/// that this carrier cannot perform.
pub(crate) fn without_session(name: &str) -> Result<Value, String> {
    match name {
        "cad_commands" => Ok(serde_json::json!(wire::commands())),
        "cad_command_aliases" => Ok(serde_json::json!(wire::aliases())),
        known if is_wire_command(known) => {
            Err(format!("{UNSUPPORTED_RUNTIME}: {known}: {NO_SESSION}"))
        }
        other => Err(unsupported_runtime(other)),
    }
}

/// **The one `dom-canvas` entry point**: serve a Tauri command name in-process.
///
/// `args_json` is the `invoke` argument object (`{"command":"LINE"}`); the answer
/// is `{"ok":true,"value":…}` on success and `{"ok":false,"error":"<reason>"}` on
/// refusal — one shape for every command, so the transport decodes one thing.
///
/// The request is served by the **real** desktop session the page carrier owns
/// ([`crate::page`]); the semantics are [`cad_ui_core::session::wire`]'s, i.e. the
/// *same* functions the Tauri commands call, so a name, an argument or a refusal
/// cannot differ between the two hosts.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn invoke(name: &str, args_json: &str) -> String {
    crate::page::invoke_impl(name, args_json)
}

/// Native builds have no browser session at all, so they answer exactly like a page
/// that has not started one — and never fabricate an `ok: true`.
#[cfg(not(target_arch = "wasm32"))]
pub fn invoke(name: &str, args_json: &str) -> String {
    let _ = args_json;
    envelope(without_session(name))
}

#[cfg(test)]
mod tests;
