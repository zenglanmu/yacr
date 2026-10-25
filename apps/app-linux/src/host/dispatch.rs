//! Host-owned IPC registration: the shared command list plus the `dom-canvas`
//! seam (ADR 0024 D1/D2/D5).
//!
//! Every desktop host registers `cad_desktop_shell::commands::register_commands()`
//! — the one `generate_handler!` list, with its own `COMMAND_NAMES` registry test.
//! The `dom-canvas` carrier needs two things that list cannot express, and both
//! are host policy, not shared state rules:
//!
//! 1. **the carrier must reach the UI**: `cad_status` gains `surfaceMode` — but
//!    only on this host path. The typed `CadStatus` (its frozen camelCase field
//!    set) lives in `cad-ui-core`; the narrowest injection point is therefore this
//!    host's own `cad_status` response, which **decorates the shared command's
//!    output** ([`cad_desktop_shell::commands::cad_status_impl`], the same
//!    projection the IPC path runs) with one extra key. On `native-below` the
//!    dispatcher never reaches this wrapper, so Wayland/Windows bytes are
//!    unchanged and the field is absent (the React side defaults to
//!    `native-below`).
//! 2. **a native-session command must refuse, not pretend**: on `dom-canvas` there
//!    is no native `CadSession`, so a command that would queue work for it is
//!    rejected **explicitly** (a rejected promise with a stable reason) instead of
//!    answering `{accepted:true}` for work nothing will ever drain (iron rule 8:
//!    the wasm session is the only `cad-db` authority, so the host must not grow a
//!    second queue as a "harmless" stand-in).
//! 3. **the chrome must be able to name the drawing without the shell mirroring
//!    document state**: the wasm session reports an opaque descriptor through
//!    `canvas_session_status` and this `cad_status` wrapper relays it as one extra
//!    `canvasSession` key ([`super::canvas_session`]). The value is the session's
//!    own report, stored verbatim; the host never opens, parses or derives
//!    anything from it. On `native-below` the command is refused — that carrier's
//!    native session already reports the document through the shared `cad_status`,
//!    and a second status path must not exist.
//!
//! Registration is **not** duplicated: the shared handler is called for every
//! command it owns, so a command added to `cad-desktop-shell` is automatically
//! available here. Only the names in [`CANVAS_RUNTIME_FAILED_HOST_COMMAND`],
//! [`super::canvas_session::CANVAS_SESSION_STATUS_COMMAND`], [`CAD_STATUS_COMMAND`]
//! and [`NATIVE_SESSION_COMMANDS`] are decided here, and all of them are asserted
//! against the shared registry by a test.

use cad_ui_core::session::CadStatus;
use tauri::ipc::Invoke;

use crate::backend::{surface_mode, SurfaceMode};

use super::canvas_session::{
    self, CanvasSessionAck, CanvasSessionDescriptor, CANVAS_SESSION_STATUS_COMMAND,
};
use super::ProbeShared;

/// Commands this module answers itself, with the carrier they are routed on.
///
/// `canvas_runtime_failed` is routed on **every** carrier: on `dom-canvas` it ends
/// the process, and on `native-below` it answers with an explicit "wrong carrier"
/// refusal instead of Tauri's generic "command not found" (the release asset never
/// calls it there, so the wire of a real run is unchanged).
///
/// `cad_status` is routed **only** on `dom-canvas`, because that is the one host
/// that must decorate it with `surfaceMode`; `native-below` keeps the shared typed
/// response byte for byte.
pub const CANVAS_RUNTIME_FAILED_HOST_COMMAND: &str = "canvas_runtime_failed";

/// The shared command this module re-implements (with one extra key) on the
/// `dom-canvas` carrier.
pub const CAD_STATUS_COMMAND: &str = "cad_status";

/// Shared commands that need the native `CadSession` this carrier does not have.
///
/// They all answer `{accepted:true}` (or a session read) for work performed by the
/// per-frame owner tick on the native session. On `dom-canvas` nothing drains
/// those queues, so an `accepted` answer would be a fake success; the dispatcher
/// rejects them with [`native_session_refusal`]'s reason instead.
///
/// Deliberately **not** in this list:
/// * `cad_settings` / `cad_open_popup` / `cad_close_popup` / `cad_commands` — the
///   shell's settings/popup-gate/vocabulary policy, which ADR 0024 D2 keeps on the
///   host; without a session they already refuse explicitly (`cad_waiting`, or the
///   `textEdit` payload derivation) rather than fabricating data;
/// * `probe_input` — no input lane exists on this carrier (the wasm session takes
///   the pointer directly), and the shared path already refuses with `cad_waiting`.
///
/// The layer commands **are** in this list: their rows and their mutations (the
/// three session-only ones plus the create/rename/delete table writes and the
/// `cad_set_layer_property` attribute write) are performed by the native
/// `CadSession` on the owner tick, and this carrier has none (the wasm session in
/// the WebView is the only CAD authority there).
pub const NATIVE_SESSION_COMMANDS: &[&str] = &[
    "cad_command",
    "cad_set_tool_parameter",
    "cad_set_tool_text",
    "cad_set_selected_text",
    "cad_set_selected_property",
    "cad_standard_view",
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

/// The explicit reason a native-session command is refused on `dom-canvas`.
///
/// Machine-readable and stable: it names the carrier and the command, so a caller
/// never has to guess why the request was not queued. Pure (no state, no
/// environment) so the refusal table is unit-tested.
pub fn native_session_refusal(command: &str, mode: SurfaceMode) -> Option<String> {
    if mode != SurfaceMode::DomCanvas || !NATIVE_SESSION_COMMANDS.contains(&command) {
        return None;
    }
    Some(format!(
        "unsupported_on_surface_mode: {command} needs a native CAD session, and surfaceMode={} \
         has none (the wasm session in the WebView is the only CAD authority)",
        mode.wire_name()
    ))
}

/// The carrier report for `cad_status`: the shared projection, the carrier, and
/// the wasm session's own descriptor (when it has reported one).
///
/// `#[serde(flatten)]` keeps the frozen `CadStatus` field set and order intact and
/// appends only the carrier keys, so an existing consumer of this payload keeps
/// reading the same shape. A `Value` round-trip was rejected on purpose: it would
/// reorder the keys (and `serde_json`'s map is sorted), which would make the wire
/// difference larger than the fields this carrier actually adds.
#[derive(Debug, serde::Serialize)]
pub struct DomCanvasStatus {
    #[serde(flatten)]
    status: CadStatus,
    /// The carrier the host runs: always [`SurfaceMode::DomCanvas`] here.
    #[serde(rename = "surfaceMode")]
    surface_mode: &'static str,
    /// The wasm session's reported descriptor, or **absent** before the first
    /// report (never a fabricated null name).
    ///
    /// This is the session's own report relayed unchanged — not a mirrored
    /// document, not derived geometry, not a second `cad-db` (iron rule 8). The
    /// host only bounds it (see [`canvas_session`]) and hands it back.
    #[serde(rename = "canvasSession", skip_serializing_if = "Option::is_none")]
    canvas_session: Option<CanvasSessionDescriptor>,
}

/// `cad_status` on the `dom-canvas` carrier: the real host snapshot + the carrier
/// + the session's reported descriptor.
///
/// The snapshot itself is the shared command's own output, so this wrapper cannot
/// drift from it (no duplicated projection). There is no native CAD session on
/// this path, so the snapshot is the explicit "no native document" one — the host
/// reports what *it* knows; document state lives in the wasm session and is not
/// mirrored here (ADR 0024 D2). `canvasSession` is that session's own report, not
/// a host-side document read.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> DomCanvasStatus {
    DomCanvasStatus {
        status: cad_desktop_shell::commands::cad_status_impl(&shared.0),
        surface_mode: SurfaceMode::DomCanvas.wire_name(),
        canvas_session: canvas_session::current_descriptor(),
    }
}

/// `canvas_runtime_failed` command: the `dom-canvas` transport cannot draw.
///
/// Wire shape `{ step, detail }`, exactly what
/// `apps/desktop-ui/src/canvas-host.ts` forwards. On the `dom-canvas` carrier this
/// ends the process through [`crate::canvas_runtime::refuse`] (marker first, i18n
/// diagnostic, exit code `6`); there is no blank-canvas success and no fallback.
///
/// On `native-below` the carrier does not exist, so a report is a programming error
/// in the caller: it is refused explicitly and the process keeps running (never a
/// silent success, and never the wrong fault domain's exit code).
#[tauri::command]
pub fn canvas_runtime_failed(step: String, detail: Option<String>) -> Result<(), String> {
    let mode = surface_mode();
    if mode != SurfaceMode::DomCanvas {
        return Err(format!(
            "unsupported_on_surface_mode: {} is a surfaceMode={} report, and this host runs {}",
            crate::canvas_runtime::CANVAS_RUNTIME_FAILED_COMMAND,
            SurfaceMode::DomCanvas.wire_name(),
            mode.wire_name()
        ));
    }
    crate::canvas_runtime::refuse(&step, detail.as_deref().unwrap_or(""))
}

/// `canvas_session_status` command: the wasm session reports its own descriptor.
///
/// The command exists **only** so the `dom-canvas` chrome can name the drawing
/// without the native shell mirroring document state (iron rule 8): the body is
/// the session's own report, stored verbatim by [`canvas_session`] and relayed
/// through `cad_status.canvasSession`. Routed on every carrier
/// (see [`invoke_handler`]); on `native-below` it rejects explicitly.
#[tauri::command]
pub fn canvas_session_status(request: tauri::ipc::Request<'_>) -> Result<CanvasSessionAck, String> {
    canvas_session::report_from_body(request.body())
}

/// The host-only commands of this module, as one handler.
///
/// Separate so the `generate_handler!` closure has a concrete return type to
/// infer its runtime parameter from.
fn host_commands<R: tauri::Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![canvas_runtime_failed, canvas_session_status, cad_status]
}

/// Build the app's `invoke_handler`.
///
/// Every command not owned by this module goes to the shared handler unchanged, so
/// `native-below` keeps exactly the registration it had before this file existed.
pub fn invoke_handler<R: tauri::Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let shared = cad_desktop_shell::commands::register_commands::<R>();
    let host = host_commands::<R>();
    move |invoke: Invoke<R>| {
        // Own the command name before `invoke` is moved into a handler.
        let command = invoke.message.command().to_string();
        let mode = surface_mode();
        if command == CANVAS_RUNTIME_FAILED_HOST_COMMAND {
            // Routed on every carrier: the command itself knows which one it runs
            // on and refuses explicitly (with no exit) when it is not `dom-canvas`.
            return host(invoke);
        }
        if command == CANVAS_SESSION_STATUS_COMMAND {
            // Routed on every carrier for the same reason: it stores the wasm
            // session's descriptor on `dom-canvas` and refuses explicitly on
            // `native-below` (no silent success, no second status path).
            return host(invoke);
        }
        if mode == SurfaceMode::DomCanvas {
            if let Some(reason) = native_session_refusal(&command, mode) {
                // Explicit refusal: a rejected promise with a stable reason. No
                // queue entry, no `accepted:true`, no fabricated status.
                invoke.resolver.reject(reason);
                return true;
            }
            if command == CAD_STATUS_COMMAND {
                return host(invoke);
            }
        }
        shared(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_desktop_shell::commands::COMMAND_NAMES;

    #[test]
    fn native_session_commands_refuse_only_on_dom_canvas() {
        for command in NATIVE_SESSION_COMMANDS {
            let refusal = native_session_refusal(command, SurfaceMode::DomCanvas)
                .unwrap_or_else(|| panic!("{command} must be refused on dom-canvas"));
            assert!(
                refusal.contains("unsupported_on_surface_mode"),
                "the reason must be machine-readable: {refusal}"
            );
            assert!(
                refusal.contains(command),
                "the reason must name the command: {refusal}"
            );
            assert!(
                refusal.contains("dom-canvas"),
                "the reason must name the carrier: {refusal}"
            );
            // `native-below` keeps the shared behaviour byte for byte.
            assert_eq!(
                native_session_refusal(command, SurfaceMode::NativeBelow),
                None
            );
        }
    }

    #[test]
    fn shell_policy_commands_are_not_refused() {
        // Popup gates, the settings read and the command vocabulary are host
        // policy (ADR 0024 D2) and must keep working on the canvas carrier.
        for command in [
            "cad_status",
            "cad_commands",
            "cad_settings",
            "cad_open_popup",
            "cad_close_popup",
            "probe_input",
            "probe_status",
        ] {
            assert_eq!(
                native_session_refusal(command, SurfaceMode::DomCanvas),
                None,
                "{command} is shell policy, not native-session work"
            );
        }
        assert_eq!(
            native_session_refusal("canvas_runtime_failed", SurfaceMode::DomCanvas),
            None,
            "the refusal channel itself must never be refused"
        );
    }

    #[test]
    fn every_decided_command_exists_in_the_shared_registry() {
        // No drift: both lists name real shared commands (or the host-only
        // ones), so a rename in `cad-desktop-shell` breaks this test instead of
        // silently turning a refusal into a pass-through.
        for command in NATIVE_SESSION_COMMANDS {
            assert!(
                COMMAND_NAMES.contains(command),
                "{command} is not a registered shared command"
            );
        }
        for command in [
            CANVAS_RUNTIME_FAILED_HOST_COMMAND,
            CANVAS_SESSION_STATUS_COMMAND,
            CAD_STATUS_COMMAND,
        ] {
            let host_only = command == CANVAS_RUNTIME_FAILED_HOST_COMMAND
                || command == CANVAS_SESSION_STATUS_COMMAND;
            assert!(
                host_only || COMMAND_NAMES.contains(&command),
                "{command} is neither host-only nor shared"
            );
        }
        assert!(COMMAND_NAMES.contains(&CAD_STATUS_COMMAND));
        // `canvas_session_status` is host-only on purpose: it exists only on the
        // `dom-canvas` seam, so the shared registry must not own it.
        assert!(!COMMAND_NAMES.contains(&CANVAS_SESSION_STATUS_COMMAND));
    }

    #[test]
    fn the_dom_canvas_status_adds_exactly_one_key() {
        let base = cad_ui_core::unsupported_status(None);
        let base_json = serde_json::to_value(&base).expect("status serializes");
        let decorated = DomCanvasStatus {
            status: base,
            surface_mode: SurfaceMode::DomCanvas.wire_name(),
            canvas_session: None,
        };
        let wire = serde_json::to_value(&decorated).expect("decorated status serializes");

        let wire_object = wire.as_object().expect("object");
        let base_object = base_json.as_object().expect("object");
        assert_eq!(wire_object.len(), base_object.len() + 1);
        for (key, value) in base_object {
            assert_eq!(wire_object.get(key), Some(value), "key {key} changed");
        }
        assert_eq!(
            wire_object.get("surfaceMode").and_then(|v| v.as_str()),
            Some("dom-canvas")
        );
        // Before the first report the session key is absent, not a null name.
        assert!(
            !wire_object.contains_key("canvasSession"),
            "no descriptor measured yet: {wire}"
        );
        // The frozen field set is preserved (spot-check the camelCase spelling the
        // frontend validates).
        assert!(wire_object.contains_key("documentLabel"));
        assert!(wire_object.contains_key("importProgress"));
        assert!(wire_object.contains_key("surfaceMode"));
    }

    #[test]
    fn the_dom_canvas_status_relays_the_session_descriptor() {
        let base = cad_ui_core::unsupported_status(None);
        let base_json = serde_json::to_value(&base).expect("status serializes");
        let decorated = DomCanvasStatus {
            status: base,
            surface_mode: SurfaceMode::DomCanvas.wire_name(),
            canvas_session: Some(CanvasSessionDescriptor {
                label: "plan.dwg".into(),
                batches: 2,
                draws: 5,
            }),
        };
        let wire = serde_json::to_value(&decorated).expect("decorated status serializes");

        let wire_object = wire.as_object().expect("object");
        let base_object = base_json.as_object().expect("object");
        // Exactly the two carrier keys on top of the untouched shared snapshot.
        assert_eq!(
            wire_object.len(),
            base_object.len() + 2,
            "surfaceMode + canvasSession only: {wire}"
        );
        for (key, value) in base_object {
            assert_eq!(wire_object.get(key), Some(value), "key {key} changed");
        }

        let session = wire_object
            .get("canvasSession")
            .and_then(|value| value.as_object())
            .expect("canvasSession is the relayed descriptor");
        let mut keys: Vec<&str> = session.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["batches", "draws", "label"],
            "the relay carries no document field beyond the session's own report"
        );
        assert_eq!(
            session.get("label").and_then(|value| value.as_str()),
            Some("plan.dwg")
        );
        assert_eq!(session.get("batches"), Some(&serde_json::json!(2)));
        assert_eq!(session.get("draws"), Some(&serde_json::json!(5)));
    }
}
