//! The **host-agnostic shell surface**: the `cad_status` snapshot and command
//! vocabulary a browser (and an Android WebView) serves the shared React chrome
//! (`apps/desktop-ui`) from, with **no Tauri IPC and no native shell process**
//! behind it.
//!
//! # Why this module exists
//!
//! The shared chrome (`apps/desktop-ui/src/App.tsx`) polls one `cad_status`
//! command every 250 ms and validates the payload with `parseCadStatus`
//! (`src/cad/cad-status.ts`). On the desktop hosts that command is a Tauri
//! command; on the `dom-canvas` carriers there is no IPC, so the page shell has
//! nothing to poll. This module is the serialisation half of that seam: it turns
//! the **session's own** [`CadStatus`] into the exact `cad_status` wire object plus
//! the four carrier keys, and publishes the command vocabulary the session really
//! executes.
//!
//! # `shell_status`: the session's snapshot, not a second projection
//!
//! [`status_json`] is a pure function of two inputs: the [`CadStatus`] the session
//! produced ([`CadSession::snapshot`](cad_ui_core::session::CadSession::snapshot),
//! which is the same call the desktop hosts answer `cad_status` with) and the
//! [`Carrier`] keys this carrier adds. So the field set, every capability flag and
//! every document fact come from `cad-ui-core` — this module adds no policy:
//!
//! | field | value here | why |
//! |---|---|---|
//! | `documentLabel`, `entityCount`, `camera`, `selection`, `tool`, `commandLog`, `popup` | the live session's own values | the real session is the `cad-db` authority; these are read from it, not mirrored |
//! | `canNew`/`canOpen`/`canSave`/`canPlot` | `false` | the carrier advertises no host-side New/Open/Save/Plot path ([`cad_platform::HostCapabilities::default`]): there is no filesystem, no save dialog and no plot export, so a `true` would be a claim this carrier cannot keep. Opening is the **transport's** command (the page's file picker, `apps/desktop-ui/src/host/web-adapter.ts`), which lifts `canOpen` itself |
//! | `canFit` | `has_document && !loading` | the session's own projection (a live document can be refit; nothing else can) |
//! | `backend`/`adapter` | `webgl2` + the presenter's adapter name | only a live presenter proves the backend came up; the name is the browser's GL string (possibly masked) and is never a hardware claim |
//! | `frameCount` | frames that actually reached the canvas | the presenter's own counter |
//! | `loading`/`importProgress` | the host's real import state | a synchronous install is never "loading" |
//! | `snapSpacing`/`snapEnabled`/`gridEnabled`/`orthoEnabled`/`hoverPreview` | the session's own switches | real session state (F7/F8/F9 work here) |
//!
//! Four keys are added on top of the frozen `CadStatus` shape (the same way the
//! Linux `dom-canvas` host adds them):
//!
//! * `surfaceMode: "dom-canvas"` — the carrier
//!   (`apps/desktop-ui/src/cad/surface.ts` selects the in-DOM canvas path on
//!   exactly this string);
//! * `palettesAvailable: false` — the chrome derives this flag from `surfaceMode`
//!   (`App.tsx`: `surfaceMode !== SURFACE_MODE_DOM_CANVAS`), so a `true` here
//!   would be ignored and would only misreport the carrier. The docked palettes
//!   read `cad_layers`/`cad_properties`, which this carrier answers — enabling
//!   them is the front-end lane's decision, not this field's;
//! * `standardViews: []` — the standard-view capability list, **empty**: there is
//!   no `cad_standard_view` transport here. The chrome reads a present list as
//!   authoritative and disables the seven view buttons with their own reason; an
//!   absent list would mean "the host declared nothing" and keep them enabled;
//! * `canvasSession: {label, batches, draws}` — the session's own descriptor,
//!   present only while a session is live. `batches` is the prepared scene's
//!   resident batch count; `draws` is `null` because the session's engine owns the
//!   draw submission count and does not publish it to this layer (never the same
//!   number reported twice under two names).
//!
//! # `shell_commands`/`shell_aliases`: the router's own vocabulary
//!
//! [`commands_json`] is `cad_ui_core::session::admissible_names()` and
//! [`aliases_json`] is `cad_ui_core::session::alias_pairs()` — the desktop tables,
//! unfiltered, because the session behind them **is** the desktop session
//! (`cad_ui_core::session::CadSession`) and executes the same names. Nothing is
//! invented and nothing is dropped: a name the router refuses is refused by name
//! when it is called, exactly as on the desktop, instead of being hidden from a
//! chrome that could otherwise offer it.
//!
//! # `shell_command`: execute, or a stable refusal — never an empty success
//!
//! The wasm side ([`crate::page::shell_command`]) runs the line through the
//! **shared** wire layer (`cad_ui_core::session::wire::command`), so the gate
//! order, the argument grammar and every diagnostic are the desktop's. A name the
//! shared gate refuses keeps the stable machine prefix
//! [`SHELL_COMMAND_REFUSAL`] followed by the gate's own sentence; a refusal the
//! *session* makes is returned verbatim (it is the session's own diagnostic, not a
//! transport problem). `{"accepted":true}` is the only success shape.
//!
//! # Native builds
//!
//! The JSON contract is target-independent, so `cargo test -p app-web` asserts the
//! exact payload on the host — against a **real** session built from the synthetic
//! fixture's bytes (no GPU, no DOM needed). A native build has no live session, so
//! [`status_json`] is called with the crate's honest empty status and the
//! vocabulary is still the real one (it is a property of the router, not of the
//! target).

use cad_ui_core::session::{admissible_names, alias_pairs, CadCommandAlias, CadStatus};
use serde_json::{json, Value};

/// The carrier this crate implements, as `cad_status.surfaceMode` spells it.
///
/// `apps/desktop-ui/src/cad/surface.ts` selects the in-DOM canvas path on exactly
/// this string, so it is a constant, not a computed value.
pub const SURFACE_MODE_DOM_CANVAS: &str = "dom-canvas";

/// The renderer backend a live wasm session uses.
///
/// Neither browser runtime exposes `navigator.gpu`, so the presenter asks for
/// `Backends::GL` and presents through WebGL2. It is reported only for a live
/// session: without one, nothing proves the backend came up.
pub const BACKEND_WEBGL2: &str = "webgl2";

/// The stable, machine-readable prefix of a [`crate::page::shell_command`] refusal
/// of a name the shared gate does not admit.
///
/// A caller keys on this (like the `yacr: unsupported-runtime: webgl2` marker); the
/// gate's own sentence follows it unchanged, so nothing is rewritten.
pub const SHELL_COMMAND_REFUSAL: &str = "unsupported-runtime: cad_command";

/// The session descriptor the chrome reads to name the open drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasSession {
    /// The label the session was opened with.
    pub label: String,
    /// Resident draw batches of the prepared scene (`None` before preparation).
    pub batches: Option<u64>,
    /// Draw submissions: **`always None`** — see the module docs.
    pub draws: Option<u64>,
}

/// Everything [`status_json`] may know beyond the session's own snapshot.
#[derive(Debug, Clone, Default)]
pub struct Carrier {
    /// The session's descriptor, or `None` without a live session (an absent key,
    /// so "no session" cannot be confused with "a session with no label").
    pub canvas_session: Option<CanvasSession>,
}

/// The JSON `cad_status` snapshot for the `dom-canvas` carrier.
///
/// `status` is the session's own snapshot (or
/// [`cad_ui_core::session::unsupported_status`] when there is none); `carrier`
/// carries only the four keys this carrier adds. See the module docs for the
/// per-field table.
pub fn status_json(status: &CadStatus, carrier: &Carrier) -> String {
    let mut wire =
        serde_json::to_value(status).expect("CadStatus is a plain struct of JSON values");
    let object = wire
        .as_object_mut()
        .expect("a serialized CadStatus is a JSON object");
    // The carrier flag the chrome dispatches on. It describes the carrier this
    // module implements, not a per-call measurement.
    object.insert("surfaceMode".to_string(), json!(SURFACE_MODE_DOM_CANVAS));
    // The palettes are hidden by the chrome on this carrier because it derives the
    // flag from `surfaceMode`; reporting `true` would only misreport the carrier.
    object.insert("palettesAvailable".to_string(), json!(false));
    // The standard-view capability list: **empty, on purpose** — this carrier has
    // no `cad_standard_view` transport. `[]` makes the chrome disable the seven
    // view buttons with their own reason instead of leaving a live-looking button
    // that would throw `unsupported-runtime: cad_standard_view`.
    object.insert("standardViews".to_string(), json!([]));
    if let Some(session) = &carrier.canvas_session {
        object.insert(
            "canvasSession".to_string(),
            json!({
                "label": session.label,
                "batches": session.batches,
                "draws": session.draws,
            }),
        );
    }
    wire.to_string()
}

/// The command names this carrier executes: the router's whole vocabulary.
///
/// `admissible_names()` is `cad_ui_core`'s single source (canonical selectors, then
/// the interaction-only selectors, then the aliases), and the session behind this
/// carrier is the same `CadSession` the desktop hosts drive — so advertising the
/// full list is the honest answer, not an over-claim.
pub fn commands_json() -> String {
    json!(admissible_names()).to_string()
}

/// The alias table, in the frozen `{alias, command}` wire shape.
///
/// `alias_pairs()` is the desktop table; a published row points at a canonical
/// selector the router admits, so every row resolves.
pub fn aliases_json() -> String {
    json!(alias_pairs()
        .into_iter()
        .map(|(alias, command)| CadCommandAlias {
            alias: alias.to_string(),
            command: command.to_string(),
        })
        .collect::<Vec<_>>())
    .to_string()
}

/// The wire reply a completed command answers with.
///
/// `queueCadCommand` (`apps/desktop-ui/src/cad/cad-command.ts`) requires exactly
/// `accepted: true`; anything else counts as a failure, so this is the only success
/// shape emitted.
pub fn accepted_json() -> String {
    json!({ "accepted": true }).to_string()
}

/// The stable refusal for a name the shared gate did not admit.
///
/// The gate's own sentence is kept byte for byte after the stable prefix; the
/// requested text is already in it (the gate echoes it with `{:?}`), so nothing is
/// duplicated here.
pub fn refusal(reason: &str) -> String {
    format!("{SHELL_COMMAND_REFUSAL}: {reason}")
}

/// The honest empty status of a carrier with no session: explicit nulls, no
/// invented document.
pub fn empty_status() -> CadStatus {
    cad_ui_core::session::unsupported_status(None)
}

/// One serialized status object, parsed back (tests and diagnostics only).
pub fn status_value(status: &CadStatus, carrier: &Carrier) -> Value {
    serde_json::from_str(&status_json(status, carrier)).expect("status_json emits valid JSON")
}

#[cfg(test)]
mod tests;
