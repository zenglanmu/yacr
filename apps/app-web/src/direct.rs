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
//! * [`DirectReads`] serves `cad_settings`/`cad_layers`/`cad_fonts`/`cad_api_call`
//!   (the reflected plugin-API document snapshot) from the session itself (the
//!   desktop serves them from the owner-tick mirror, because there the session
//!   is `!Send`).
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
//! true}` for a mutation, the `cad_settings`/`cad_layers`/`cad_fonts` object for
//! the reads, the `PopupAck` for `cad_open_popup`). `error` is byte-identical to
//! the desktop refusal for a malformed request; for a request the **session**
//! refuses, it is the session's own text (see the crate docs of
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
//! live session** still answers the two vocabulary reads and the crate version
//! from shared data — they are pure — and refuses every other known command with
//! [`NO_SESSION`], never with a fabricated acknowledgement.

use cad_app::{StandardView, ToolParameter, ToolValue};
use cad_ui_core::session::wire::{self, AdmissionTarget, ControlReads, PopupTarget};
use cad_ui_core::session::{
    CadApiResults, CadApiSnapshot, CadApiSubmission, CadFonts, CadLayerWrite, CadLayers,
    CadSession, CadSettingKey, CadSettingValue, CadSettings,
};
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
        crate::platform::check_image_command(&normalized)?;
        let outcome = self.session.command(&normalized);
        self.applied(outcome)
    }

    fn runtime_aliases(&self) -> Vec<(String, String)> {
        self.session.runtime_command_aliases()
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

    fn orbit(&mut self, delta: wire::OrbitDelta) -> Result<(), String> {
        let outcome = self.session.orbit(delta.yaw, delta.pitch);
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

    fn api_submit(&mut self, submission: CadApiSubmission) -> Result<(), String> {
        // Applied inline: this carrier's session is on the calling thread. The
        // answer is kept on the session, because the client polls later and the
        // sink is rebuilt for every dispatch call. A prompt that just started has
        // no answer yet; it arrives through `api_prompt_results`.
        if let Some(outcome) = self.session.submit_api_call(submission) {
            self.session.push_api_outcome(outcome);
        }
        Ok(())
    }

    fn api_results(&mut self) -> Result<CadApiResults, String> {
        // Settled submissions and completed prompts share one FIFO, exactly like
        // the desktop owner tick's shared queue.
        let mut results = self.session.take_api_outcomes();
        results.extend(self.session.api_prompt_results());
        Ok(CadApiResults { results })
    }
}

/// The [`ControlReads`] of a host that owns its session on this thread.
///
/// Every answer is the session's own; there is no mirror to keep in step and no
/// `None` state (a synchronous carrier has a session whenever it can answer at
/// all). `layers()`/`fonts()` relay the session's own projection refusal
/// unchanged.
///
/// The plugin-API snapshot is read **once**, when the reads struct is built
/// (i.e. at the start of one `invoke`): [`ControlReads::api_snapshot`] borrows
/// from `self`, so the value must live in the struct rather than be a temporary.
/// That is the same instant the desktop's mirror would be read. A live session
/// always has a snapshot (it is the document manager); `document: None` inside it
/// is the real "no document is open".
pub struct DirectReads<'a> {
    session: &'a CadSession,
    /// The live session's reflected snapshot (`document: None` = no document).
    snapshot: CadApiSnapshot,
}

impl<'a> DirectReads<'a> {
    pub fn new(session: &'a CadSession) -> Self {
        Self {
            session,
            snapshot: session.api_snapshot(),
        }
    }
}

impl ControlReads for DirectReads<'_> {
    fn settings(&self) -> Option<CadSettings> {
        Some(self.session.settings())
    }

    fn layers(&self) -> Option<Result<CadLayers, String>> {
        Some(self.session.layers())
    }

    fn fonts(&self) -> Option<Result<CadFonts, String>> {
        Some(self.session.fonts())
    }

    fn api_snapshot(&self) -> Option<&CadApiSnapshot> {
        Some(&self.snapshot)
    }
}

// ---------------------------------------------------------------------------
// Generated argument decoding and dispatch (one row per command in the registry)
// ---------------------------------------------------------------------------
//
// Everything below is derived from `cad_ui_core::session::registry`. The desktop
// `#[tauri::command]` wrappers are generated from the same rows, so an argument
// key or a reply type can never differ between the two transports.
//
// `probe_input` and `cad_status` are **separate wasm exports** (see
// `crate::probe_input` and `crate::shell_status`), not `dispatch` arms. The
// registry marks those rows `wasm_export` for the documented name set only; this
// transport keeps serving them through their own export so the pointer path and
// the status projection keep their existing shapes.

/// Emit one `#[derive(Deserialize)]` argument struct for a registry row.
macro_rules! wasm_wire_args_struct {
    ($name:ident, { $($fields:tt)* }) => {
        wasm_wire_args_struct!(@fields $name [] $($fields)*);
    };
    (@fields $name:ident [$($out:tt)*]) => {
        // camelCase, exactly like Tauri's default command decoding: the two
        // transports must accept the same argument object (`requestId`, …).
        #[derive(Debug, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct $name { $($out)* }
    };
    (@fields $name:ident [$($out:tt)*] arg(req, $field:ident, $ty:ty) $($rest:tt)*) => {
        wasm_wire_args_struct!(@fields $name [$($out)* pub $field: $ty,] $($rest)*);
    };
    (@fields $name:ident [$($out:tt)*] arg(opt, $field:ident, $ty:ty) $($rest:tt)*) => {
        wasm_wire_args_struct!(@fields $name [$($out)* #[serde(default)] pub $field: $ty,] $($rest)*);
    };
}

/// Emit the structs of every shared row that carries arguments.
macro_rules! wasm_wire_structs {
    (@munch) => {};
    (@munch @row(shared, $lane:ident, $args:ident, $name:ident, { }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        wasm_wire_structs!(@munch $($rest)*);
    };
    (@munch @row(shared, $lane:ident, $args:ident, $name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        wasm_wire_args_struct!($args, { $($fields)* });
        wasm_wire_structs!(@munch $($rest)*);
    };
    (@munch @row($($other:tt)*) $($rest:tt)*) => {
        wasm_wire_structs!(@munch $($rest)*);
    };
    ($($rows:tt)*) => {
        wasm_wire_structs!(@munch $($rows)*);
    };
}

/// A destructuring pattern for one generated argument struct.
macro_rules! wasm_wire_pattern {
    ($args:ident, { $($fields:tt)* }) => {
        wasm_wire_pattern!(@fields $args [] $($fields)*)
    };
    (@fields $args:ident [$($out:ident),*]) => {
        $args { $($out),* }
    };
    (@fields $args:ident [$($out:ident),*] arg(req, $field:ident, $ty:ty) $($rest:tt)*) => {
        wasm_wire_pattern!(@fields $args [$($out,)* $field] $($rest)*)
    };
    (@fields $args:ident [$($out:ident),*] arg(opt, $field:ident, $ty:ty) $($rest:tt)*) => {
        wasm_wire_pattern!(@fields $args [$($out,)* $field] $($rest)*)
    };
}

/// Emit `dispatch` with one arm per shared row.
///
/// The parameter names are captured from the registry's `@context` header, so the
/// row bodies (also written in the registry) resolve them without any per-row
/// rebinding.
macro_rules! wasm_wire_dispatch {
    ($sink:ident, $reads:ident, $popups:ident, $name:ident, $args_json:ident; $($rows:tt)*) => {
        wasm_wire_dispatch!(@munch [$sink] [$reads] [$popups] [$name] [$args_json] [] $($rows)*);
    };
    (@munch [$sink:ident] [$reads:ident] [$popups:ident] [$name:ident] [$args_json:ident] [$($arms:tt)*]) => {
        /// Serve one wire command by name, exactly as the desktop command of that
        /// name (generated from the shared registry).
        pub fn dispatch(
            $name: &str,
            $args_json: &str,
            $sink: &mut dyn AdmissionTarget,
            $reads: &dyn ControlReads,
            $popups: &mut dyn PopupTarget,
        ) -> Result<Value, String> {
            match $name {
                $($arms)*
                other => Err(unsupported_runtime(other)),
            }
        }
    };
    (@munch [$sink:ident] [$reads:ident] [$popups:ident] [$name:ident] [$args_json:ident] [$($arms:tt)*]
        @row(shared, $lane:ident, $args:ident, $row_name:ident, { }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        wasm_wire_dispatch!(@munch [$sink] [$reads] [$popups] [$name] [$args_json] [
            $($arms)*
            stringify!($row_name) => wire_value($($body)*),
        ] $($rest)*);
    };
    (@munch [$sink:ident] [$reads:ident] [$popups:ident] [$name:ident] [$args_json:ident] [$($arms:tt)*]
        @row(shared, $lane:ident, $args:ident, $row_name:ident, { $(arg($kind:ident, $field:ident, $ty:ty))* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        wasm_wire_dispatch!(@munch [$sink] [$reads] [$popups] [$name] [$args_json] [
            $($arms)*
            stringify!($row_name) => {
                let wasm_wire_pattern!($args, { $(arg($kind, $field, $ty))* }) =
                    args::<$args>($name, $args_json)?;
                wire_value($($body)*)
            },
        ] $($rest)*);
    };
    (@munch [$sink:ident] [$reads:ident] [$popups:ident] [$name:ident] [$args_json:ident] [$($arms:tt)*]
        @row($($other:tt)*) $($rest:tt)*) => {
        wasm_wire_dispatch!(@munch [$sink] [$reads] [$popups] [$name] [$args_json] [$($arms)*] $($rest)*);
    };
}

/// Emit `WIRE_COMMANDS` from the shared rows.
macro_rules! wasm_wire_names {
    (@munch [$($names:tt)*]) => {
        /// Every wire command this transport serves, derived from the registry.
        pub const WIRE_COMMANDS: &[&str] = &[$($names)*];
    };
    (@munch [$($names:tt)*] @row(shared, $lane:ident, $args:ident, $name:ident, { $($fields:tt)* }, $reply:ty, { $($body:tt)* }) $($rest:tt)*) => {
        wasm_wire_names!(@munch [$($names)* stringify!($name),] $($rest)*);
    };
    (@munch [$($names:tt)*] @row($($other:tt)*) $($rest:tt)*) => {
        wasm_wire_names!(@munch [$($names)*] $($rest)*);
    };
    ($($rows:tt)*) => {
        wasm_wire_names!(@munch [] $($rows)*);
    };
}

/// The `dom-canvas` emitter over the shared registry.
macro_rules! emit_wasm_wire_registry {
    (@context($sink:ident, $reads:ident, $popups:ident, $name:ident, $args_json:ident) $($rows:tt)*) => {
        wasm_wire_structs!($($rows)*);
        wasm_wire_dispatch!($sink, $reads, $popups, $name, $args_json; $($rows)*);
        wasm_wire_names!($($rows)*);
    };
}

cad_ui_core::cad_wire_registry!(emit_wasm_wire_registry);

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

/// Serialize one typed wire answer exactly once, into the `invoke` envelope value.
fn wire_value<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    let value = result?;
    serde_json::to_value(value).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Dispatch (generated above from the shared registry)
// ---------------------------------------------------------------------------

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

/// Whether `name` is one of [`WIRE_COMMANDS`] (generated from the shared registry).
pub fn is_wire_command(name: &str) -> bool {
    WIRE_COMMANDS.contains(&name)
}

/// The answers a carrier **without a live CAD session** can still give honestly.
///
/// The two vocabulary reads are pure functions of the shared tables, and the crate
/// version is a compile-time constant, so all three are served; every other known
/// command is the explicit [`NO_SESSION`] refusal and an unknown name keeps
/// [`unsupported_runtime`]. Nothing is answered as a success that this carrier
/// cannot perform.
pub(crate) fn without_session(name: &str) -> Result<Value, String> {
    match name {
        "cad_commands" => Ok(serde_json::json!(wire::commands())),
        "cad_command_aliases" => Ok(serde_json::json!(wire::aliases())),
        "cad_app_version" => Ok(serde_json::json!(cad_ui_core::app_version())),
        // The secondary-development API descriptor is a compile-time contract, not
        // document state: a page that has not started its session must still be
        // able to learn what this build supports (and what it explicitly refuses).
        "cad_api_status" => wire::api_status().and_then(|status| {
            serde_json::to_value(status).map_err(|error| format!("api.descriptor_invalid: {error}"))
        }),
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
