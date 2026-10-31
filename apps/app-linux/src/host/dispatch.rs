//! Host-owned IPC registration: the shared command list plus the `dom-canvas`
//! seam (ADR 0024 D1/D2/D5).
//!
//! Every desktop host registers `cad_desktop_shell::commands::register_commands()`
//! — the one `generate_handler!` list, with its own `COMMAND_NAMES` registry test.
//! The `dom-canvas` carrier needs two things that list cannot express, and both
//! are host policy, not shared state rules:
//!
//! 1. **the carrier must reach the UI**: every `cad_status` carries a typed
//!    `hostProfile` object (`cad_ui_core::session::HostProfile`). This host's
//!    `cad_status` answers the shared [`WireStatus`] envelope with
//!    [`HostProfile::dom_canvas`] plus the wasm session's descriptor; the
//!    `native-below` path answers the shared command's own envelope with
//!    [`HostProfile::native_below`]. The old story ("the field is present only on
//!    this carrier and absent on native-below") is dead: every carrier names
//!    itself in the same key.
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
//! 4. **the vocabulary must describe *this* carrier**: `cad_commands` and
//!    `cad_command_aliases` are shared commands, but their answer
//!    ([`command_vocabulary`], [`command_aliases`]) is table-driven and describes
//!    commands only the native `CadSession` can execute. On `dom-canvas` there is
//!    no such session — every one of those names is refused by
//!    [`native_session_refusal`] — so this host answers **empty** there. The chrome
//!    then refuses by name (`unsupported-runtime: <command>`) exactly as it does
//!    in a browser, instead of offering commands nothing can run.
//!
//! Registration is **not** duplicated: the shared handler is called for every
//! command it owns, so a command added to `cad-desktop-shell` is automatically
//! available here. Only the names in [`CANVAS_RUNTIME_FAILED_HOST_COMMAND`],
//! [`super::canvas_session::CANVAS_SESSION_STATUS_COMMAND`], [`CAD_STATUS_COMMAND`],
//! [`CAD_COMMANDS_COMMAND`], [`CAD_COMMAND_ALIASES_COMMAND`] and
//! [`NATIVE_SESSION_COMMANDS`] are decided here, and all of them are asserted
//! against the shared registry by a test.

use cad_desktop_shell::commands::CadCommandAlias;
use cad_ui_core::session::{HostProfile, WireStatus};
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
/// `cad_status` is routed **only** on `dom-canvas`, because this host answers it
/// with the `dom-canvas` `hostProfile` plus the session descriptor; `native-below`
/// keeps the shared envelope (its own `native-below` profile, no canvas).
pub const CANVAS_RUNTIME_FAILED_HOST_COMMAND: &str = "canvas_runtime_failed";

/// The shared command this module re-implements on the `dom-canvas` carrier (the
/// typed `hostProfile` plus the optional session descriptor).
pub const CAD_STATUS_COMMAND: &str = "cad_status";

/// The shared vocabulary command, answered **empty** on `dom-canvas`.
pub const CAD_COMMANDS_COMMAND: &str = "cad_commands";

/// The shared alias-table command, answered **empty** on `dom-canvas`.
pub const CAD_COMMAND_ALIASES_COMMAND: &str = "cad_command_aliases";

/// The command vocabulary this host publishes on `mode`.
///
/// `native-below` is the shared table byte for byte
/// (`cad_ui_core::session::admissible_names`). `dom-canvas` is **empty**, and that
/// is a real answer, not a missing one: the desktop table describes commands only
/// the native `CadSession` can execute, this carrier has none (iron rule 8), and
/// every one of those names is refused by [`native_session_refusal`] anyway. An
/// empty list makes the chrome refuse by name — the vocabulary a browser and an
/// Android WebView already see, because theirs is the wasm session's.
///
/// Pure (no state, no environment), so the rule is unit-tested.
pub fn command_vocabulary(mode: SurfaceMode) -> Vec<&'static str> {
    match mode {
        SurfaceMode::NativeBelow => cad_ui_core::session::admissible_names(),
        SurfaceMode::DomCanvas => Vec::new(),
    }
}

/// The alias table this host publishes on `mode`; empty on `dom-canvas` for the
/// same reason as [`command_vocabulary`] (an alias is a suggestion that would
/// resolve onto a command this carrier refuses).
pub fn command_aliases(mode: SurfaceMode) -> Vec<CadCommandAlias> {
    match mode {
        SurfaceMode::NativeBelow => cad_desktop_shell::commands::cad_command_aliases_impl(),
        SurfaceMode::DomCanvas => Vec::new(),
    }
}

/// Shared commands that need the native `CadSession` this carrier does not have.
///
/// They all answer `{accepted:true}` (or a session read) for work performed by the
/// per-frame owner tick on the native session. On `dom-canvas` nothing drains
/// those queues, so an `accepted` answer would be a fake success; the dispatcher
/// rejects them with [`native_session_refusal`]'s reason instead.
///
/// Deliberately **not** in this list:
/// * `cad_settings` / `cad_open_popup` / `cad_close_popup` — the shell's
///   settings/popup-gate policy, which ADR 0024 D2 keeps on the host; without a
///   session they already refuse explicitly (`cad_waiting`, or the `textEdit`
///   payload derivation) rather than fabricating data;
/// * `cad_commands` / `cad_command_aliases` — shell policy too, but their answer is
///   the **vocabulary**, so refusing would leave the chrome with no way to learn
///   what this carrier cannot do. They are answered empty instead
///   ([`command_vocabulary`]): a real "nothing here", which the chrome turns into a
///   refusal by name;
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

/// `cad_status` on the `dom-canvas` carrier: the real host snapshot + the typed
/// carrier profile + the session's reported descriptor.
///
/// The snapshot itself is the shared command's own output, so this wrapper cannot
/// drift from it (no duplicated projection). There is no native CAD session on
/// this path, so the snapshot is the explicit "no native document" one — the host
/// reports what *it* knows; document state lives in the wasm session and is not
/// mirrored here (ADR 0024 D2). `canvasSession` is that session's own report, not
/// a host-side document read.
///
/// The envelope is [`WireStatus`] (the shared definition), so this host no longer
/// keeps a look-alike `DomCanvasStatus`: the frozen `CadStatus` fields stay first,
/// then the always-present `hostProfile` ([`HostProfile::dom_canvas`]), then the
/// optional `canvasSession`. The old top-level `surfaceMode` key is gone.
#[tauri::command]
pub fn cad_status(
    shared: tauri::State<'_, ProbeShared>,
) -> WireStatus<'static, CanvasSessionDescriptor> {
    WireStatus::owned(
        cad_desktop_shell::commands::cad_status_impl(&shared.0),
        HostProfile::dom_canvas(),
        canvas_session::current_descriptor(),
    )
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

/// `cad_commands` on this host: [`command_vocabulary`] for the installed carrier.
///
/// Registered here (not only in the shared handler) so the `dom-canvas` carrier
/// can answer **empty** instead of advertising the desktop table. `native-below`
/// answers exactly what the shared command answers.
#[tauri::command]
pub fn cad_commands() -> Vec<&'static str> {
    command_vocabulary(surface_mode())
}

/// `cad_command_aliases` on this host: [`command_aliases`] for the installed
/// carrier (empty on `dom-canvas`, see [`command_vocabulary`]).
#[tauri::command]
pub fn cad_command_aliases() -> Vec<CadCommandAlias> {
    command_aliases(surface_mode())
}

/// The host-only commands of this module, as one handler.
///
/// Separate so the `generate_handler!` closure has a concrete return type to
/// infer its runtime parameter from.
fn host_commands<R: tauri::Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        canvas_runtime_failed,
        canvas_session_status,
        cad_status,
        cad_commands,
        cad_command_aliases
    ]
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
            if command == CAD_STATUS_COMMAND
                || command == CAD_COMMANDS_COMMAND
                || command == CAD_COMMAND_ALIASES_COMMAND
            {
                // The status carries the carrier (and the session descriptor); the
                // two vocabulary answers must describe this carrier, not the
                // native one it does not have.
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
    use cad_ui_core::session::WIRE_COMMAND_NAMES;

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
            "cad_command_aliases",
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
    fn the_dom_canvas_carrier_advertises_no_vocabulary() {
        // Defence in depth: the chrome now asks the wasm session as soon as the
        // carrier is reported, but a caller that still asks this host must not get
        // a catalogue of commands this carrier refuses (every desktop name is in
        // `NATIVE_SESSION_COMMANDS`). Empty is the honest answer - the same shape
        // a browser or an Android WebView receives.
        assert!(command_vocabulary(SurfaceMode::DomCanvas).is_empty());
        assert!(command_aliases(SurfaceMode::DomCanvas).is_empty());
        // `native-below` stays byte for byte the shared table: the Wayland and
        // Windows vocabulary is unchanged.
        assert_eq!(
            command_vocabulary(SurfaceMode::NativeBelow),
            cad_ui_core::session::admissible_names()
        );
        assert_eq!(
            command_aliases(SurfaceMode::NativeBelow),
            cad_desktop_shell::commands::cad_command_aliases_impl()
        );
        assert!(!command_vocabulary(SurfaceMode::NativeBelow).is_empty());
    }

    #[test]
    fn the_vocabulary_override_is_routed_only_on_dom_canvas() {
        // Source contract: both names must reach the host handler *inside* the
        // `dom-canvas` branch, so `native-below` keeps the shared command untouched
        // (no duplicated registration on the carrier that has a native session).
        let source = include_str!("dispatch.rs");
        let branch = source
            .split_once("if mode == SurfaceMode::DomCanvas {")
            .expect("the carrier branch must exist")
            .1;
        assert!(branch.contains("command == CAD_COMMANDS_COMMAND"));
        assert!(branch.contains("command == CAD_COMMAND_ALIASES_COMMAND"));
        assert!(branch.contains("return host(invoke);"));
    }

    #[test]
    fn every_decided_command_exists_in_the_shared_registry() {
        // No drift: both lists name real shared commands (or the host-only
        // ones), so a rename in `cad-desktop-shell` breaks this test instead of
        // silently turning a refusal into a pass-through.
        for command in NATIVE_SESSION_COMMANDS {
            // The policy list is a **subset** of the shared registry: it may decide
            // that a shared command is native-only on this carrier, but it must
            // never name a command the registry does not own.
            assert!(
                WIRE_COMMAND_NAMES.contains(command),
                "{command} is not in the shared wire registry"
            );
            assert!(
                COMMAND_NAMES.contains(command),
                "{command} is not a registered desktop command"
            );
        }
        for command in [
            CANVAS_RUNTIME_FAILED_HOST_COMMAND,
            CANVAS_SESSION_STATUS_COMMAND,
            CAD_STATUS_COMMAND,
            CAD_COMMANDS_COMMAND,
            CAD_COMMAND_ALIASES_COMMAND,
        ] {
            let host_only = command == CANVAS_RUNTIME_FAILED_HOST_COMMAND
                || command == CANVAS_SESSION_STATUS_COMMAND;
            assert!(
                host_only || COMMAND_NAMES.contains(&command),
                "{command} is neither host-only nor shared"
            );
        }
        assert!(COMMAND_NAMES.contains(&CAD_STATUS_COMMAND));
        // The two vocabulary commands are shared: this module only overrides the
        // answer on the carrier that has no native session.
        assert!(COMMAND_NAMES.contains(&CAD_COMMANDS_COMMAND));
        assert!(COMMAND_NAMES.contains(&CAD_COMMAND_ALIASES_COMMAND));
        // `canvas_session_status` is host-only on purpose: it exists only on the
        // `dom-canvas` seam, so the shared registry must not own it.
        assert!(!COMMAND_NAMES.contains(&CANVAS_SESSION_STATUS_COMMAND));
    }

    #[test]
    fn the_dom_canvas_status_uses_the_typed_host_profile() {
        let base = cad_ui_core::unsupported_status(None);
        let base_json = serde_json::to_value(&base).expect("status serializes");
        let profile = HostProfile::dom_canvas();
        let decorated: WireStatus<'_, CanvasSessionDescriptor> =
            WireStatus::new(&base, &profile, None);
        let wire = serde_json::to_value(&decorated).expect("decorated status serializes");

        let wire_object = wire.as_object().expect("object");
        let base_object = base_json.as_object().expect("object");
        // Exactly one carrier key on top of the untouched shared snapshot.
        assert_eq!(wire_object.len(), base_object.len() + 1);
        for (key, value) in base_object {
            assert_eq!(wire_object.get(key), Some(value), "key {key} changed");
        }
        assert_eq!(
            wire_object.get("hostProfile"),
            Some(&serde_json::json!({
                "surfaceMode": "dom-canvas",
                "documentOwner": "wasm",
                "palettesAvailable": true,
                "standardViews": [],
            }))
        );
        // The old top-level carrier keys are gone, not merely duplicated.
        for key in ["surfaceMode", "palettesAvailable", "standardViews"] {
            assert!(
                !wire_object.contains_key(key),
                "{key} must only appear inside hostProfile: {wire}"
            );
        }
        // Before the first report the session key is absent, not a null name.
        assert!(
            !wire_object.contains_key("canvasSession"),
            "no descriptor measured yet: {wire}"
        );
        // The frozen field set is preserved (spot-check the camelCase spelling the
        // frontend validates).
        assert!(wire_object.contains_key("documentLabel"));
        assert!(wire_object.contains_key("importProgress"));
    }

    #[test]
    fn the_dom_canvas_status_relays_the_session_descriptor() {
        let base = cad_ui_core::unsupported_status(None);
        let base_json = serde_json::to_value(&base).expect("status serializes");
        let descriptor = CanvasSessionDescriptor {
            label: "plan.dwg".into(),
            batches: 2,
            draws: 5,
        };
        let profile = HostProfile::dom_canvas();
        let decorated: WireStatus<'_, CanvasSessionDescriptor> =
            WireStatus::new(&base, &profile, Some(&descriptor));
        let wire = serde_json::to_value(&decorated).expect("decorated status serializes");

        let wire_object = wire.as_object().expect("object");
        let base_object = base_json.as_object().expect("object");
        // Exactly the carrier keys on top of the untouched shared snapshot.
        assert_eq!(
            wire_object.len(),
            base_object.len() + 2,
            "hostProfile + canvasSession only: {wire}"
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
