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
//! command; on the browser carrier there is no IPC at all, so the page shell has
//! nothing to poll and the chrome cannot run. This module is that seam: four
//! exported functions that return the **same wire contract** the Tauri hosts
//! serve, computed from what the wasm session actually knows.
//!
//! It is deliberately *not* a second session, a second document or a command
//! router (ADR 0024 D2 / iron rule 8): it only **projects** facts the session
//! already owns, and it refuses the command channel instead of pretending to
//! implement it.
//!
//! # Exports (all return JSON strings)
//!
//! ```text
//! shell_status()            -> JSON `cad_status` snapshot
//! shell_commands()          -> JSON array of executable command names ([])
//! shell_aliases()           -> JSON array of `[alias, canonical]` pairs ([])
//! shell_command(command)    -> Err("unsupported-runtime: cad_command: …")
//! ```
//!
//! # `shell_status`: every field is real or honestly empty
//!
//! The baseline is [`cad_ui_core::session::unsupported_status`] — the same
//! all-capabilities-off, no-document snapshot a probe-mode desktop host reports
//! (there is no `CadStatus::empty()`; that function is the crate's honest empty
//! status, and re-using it is what keeps the field set, the `selection` block and
//! the `snapSpacing` default from drifting). Overlaid on it, field by field, are
//! only facts this wasm instance can observe:
//!
//! | field | value here | why |
//! |---|---|---|
//! | `documentLabel` | the live session's own label | the session imported that file; `null` with no session |
//! | `entityCount` | always `null` | the imported `Database` is **released** after the display build ([`crate::budget::DB_RETAINED`]); there is no query path, so a count would be invented |
//! | `loading` | `true` only while `start`/`open_bytes` is in flight | the real registry state (`State::Starting`), not a guess |
//! | `importProgress` | always `null` | a JS caller's in-flight `open_bytes` promise *is* the progress channel; no tick stream exists here |
//! | `error` | the live session's last failure step+message, or a failed start's | the session's own [`Failure`](crate::session) text; technical, never localized |
//! | `canNew` | `false` | no blank-document path exists in this crate |
//! | `canOpen` | `false` | opening is the page's `open_bytes(bytes)`, not a command; the chrome must not advertise a host OPEN |
//! | `canSave`/`canPlot` | `false` | no save/plot path, and the database is released (no second document to write) |
//! | `canFit` | `false` | there is **no refit entry point** in this crate — the fit happens once inside the open, from bounds that are dropped with the database. A `true` would advertise a command that cannot run |
//! | `backend` | `"webgl2"` **only while a session is live** | the presenter is the real proof the WebGL2 backend came up; a failed/unstarted carrier never claims a backend |
//! | `adapter` | the presenter's adapter name, tagged as the wasm session | names *which* session is drawing; the name itself is the browser's GL string (possibly masked), never a hardware claim |
//! | `camera` | the session camera's real `{center, scale}` | `camera2d_params()`; `null` when the camera is not a valid 2D view |
//! | `selection` | the honest empty block | there is no picking path on this carrier |
//! | `tool`/`popup`/`commandLog`/`gripDrag` | `null`/`[]`/`false` | no draw/measure/command/grip state exists here (ADR 0024: no grip path) |
//! | `snapSpacing` | `cad_app::DEFAULT_SNAP_SPACING` | the baseline's real default, so the frozen `> 0` contract holds; every switch that would use it is `false` |
//! | `frameCount` | frames the presenter actually put on the canvas | `presented_frames()`; dropped/refused frames never count |
//! | `importCount` | documents this wasm instance **installed** | incremented only after import + upload + first presented frame ([`crate::session`]), so a refused file never counts |
//! | `cursorWorld`/`crosshair` | `null`/`false` | no pointer stream reaches this crate; a synthesized coordinate would be a fabricated readout |
//!
//! Three fields are added on top of the frozen `CadStatus` shape (the same way
//! the Linux `dom-canvas` host adds them):
//!
//! * `surfaceMode: "dom-canvas"` — the carrier (`apps/desktop-ui/src/cad/surface.ts`
//!   selects the in-DOM canvas path on exactly this string);
//! * `palettesAvailable: false` — this carrier holds **no host-side document
//!   state** for the layers/properties/diagnostics palettes to project (iron rule
//!   8). Note for the page shell: `palettesOffered()` reads that flag from the
//!   **shell** object, so a wrapper that builds `{ status }` must lift this value
//!   out of the status instead of nesting it;
//! * `canvasSession: {label, batches, draws}` — the session's own descriptor,
//!   present only while a session is live (the same shape `status()` publishes and
//!   the Linux host relays), never a mirrored document.
//!
//! # `shell_commands`/`shell_aliases`: the empty vocabulary, on purpose
//!
//! `cad_ui_core::session::admissible_names()` is the *desktop* command
//! vocabulary: those names are routed by `cad_host`/`CadSession`, which this crate
//! does not have. Claiming them would make the chrome (ribbon, command line) offer
//! buttons that nothing can execute, and the first click would be a silent
//! nothing. `OPEN` is the sharpest example: opening a file here is
//! [`open_bytes`](crate::open_bytes) (the canvas transport owns the bytes and the
//! canvas), not a command, so it is **not** in the list either.
//!
//! So both lists are `[]` until a command genuinely runs. When one does, it is
//! added here **at the same time** as its implementation and its contract test —
//! never ahead of it.
//!
//! # `shell_command`: a stable refusal, never an empty success
//!
//! The browser carrier is view/navigate only (open, pan, zoom, resize). An
//! unimplemented command must be *loud*: the stable machine prefix
//! [`SHELL_COMMAND_REFUSAL`] plus one sentence, so a caller can key on the prefix
//! and a human reads why. Returning `Ok("")` would look like a success to the
//! chrome and to any future caller.
//!
//! # Native builds
//!
//! The JSON contract is target-independent (it describes the carrier this crate
//! implements, and degrades to the honest no-session snapshot where no session
//! exists), which is what lets `cargo test -p app-web` assert the exact payload on
//! the host instead of only in a browser. The one platform refusal is the command
//! channel: [`shell_command`] refuses on every target, because no target of this
//! crate has a command router.

use serde_json::{json, Value};

use cad_ui_core::session::{unsupported_status, CadCamera, CadStatus};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The carrier this crate implements, as `cad_status.surfaceMode` spells it.
///
/// `apps/desktop-ui/src/cad/surface.ts` selects the in-DOM canvas path on exactly
/// this string, so it is a constant, not a computed value.
pub const SURFACE_MODE_DOM_CANVAS: &str = "dom-canvas";

/// The renderer backend a live wasm session uses.
///
/// Neither browser runtime exposes `navigator.gpu`, so the presenter asks for
/// `Backends::GL` and presents through WebGL2 (see `crate::session`). It is
/// reported only for a live session: without one, nothing proves the backend came
/// up, and a `backend` string is a measurement of the running carrier, not a
/// capability advertisement.
pub const BACKEND_WEBGL2: &str = "webgl2";

/// The stable, machine-readable prefix of every [`shell_command`] refusal.
///
/// A caller keys on this (like the `yacr: unsupported-runtime: webgl2` marker);
/// the sentence that follows it is for a human. It always names `cad_command`, the
/// only channel this carrier refuses wholesale.
pub const SHELL_COMMAND_REFUSAL: &str = "unsupported-runtime: cad_command";

/// The real facts of the live wasm session, as the status projection needs them.
///
/// Every field is something the session **observed**; nothing here is derived from
/// a UI guess or from the file name. `None` anywhere means "not observable", which
/// the projection reports as `null` rather than inventing a value.
#[derive(Debug, Clone)]
pub struct LiveSession {
    /// The label the session was opened with (its own display name).
    pub label: String,
    /// Resident draw batches after the upload.
    pub batches: u64,
    /// Draw submissions of the last presented frame.
    pub draws: u64,
    /// Frames that actually reached the canvas (`presented_frames()`).
    pub presented: u64,
    /// The presenter's adapter descriptor, tagged as the wasm session.
    pub adapter: String,
    /// The camera's real `{center, scale}`, or `None` when it is not a valid 2D
    /// view (a failed navigation leaves it unusable; the status then reports
    /// `camera: null`).
    pub camera: Option<CadCamera>,
    /// The live session's last technical failure (step + message), or `None`.
    pub error: Option<String>,
}

/// Everything the status projection is allowed to know, and nothing more.
///
/// This is the **only** input of [`status_json`], which is why the projection is a
/// pure function of observed facts and can be asserted in a host unit test. The
/// wasm side fills it from the session registry ([`crate::session`]).
#[derive(Debug, Clone, Default)]
pub struct SessionFacts {
    /// The live session, or `None` when none exists (idle / failed / starting).
    pub session: Option<LiveSession>,
    /// Whether `start`/`open_bytes` is genuinely in flight right now.
    pub loading: bool,
    /// Documents this wasm instance installed (a real count, never a guess).
    pub import_count: u64,
    /// A failure that ended a start attempt (there is no live session to carry
    /// it), or `None`.
    pub error: Option<String>,
}

/// The JSON `cad_status` snapshot for the browser carrier.
///
/// See the module docs for the per-field table; the short version is that the
/// all-off baseline is overlaid with the live session's real facts, three carrier
/// keys are added, and every remaining field stays at the honest empty value.
pub fn status_json(facts: &SessionFacts) -> String {
    let mut status = baseline();
    status.loading = facts.loading;
    status.import_count = facts.import_count;

    if let Some(live) = &facts.session {
        // The session imported this file and is presenting it: both are its own
        // facts, not a host-side read.
        status.document_label = Some(live.label.clone());
        // Only a live presenter proves the backend and names the adapter.
        status.backend = Some(BACKEND_WEBGL2.to_string());
        status.adapter = Some(live.adapter.clone());
        status.camera = live.camera;
        status.frame_count = live.presented;
    }
    // A live session's own last failure wins; otherwise report the failure that
    // ended a start attempt, so a failed carrier never looks healthy.
    status.error = facts
        .session
        .as_ref()
        .and_then(|live| live.error.clone())
        .or_else(|| facts.error.clone());

    // `entityCount`, `canNew`, `canOpen`, `canSave`, `canPlot`, `canFit`,
    // `importProgress`, `tool`, `popup`, `commandLog`, `gripDrag`,
    // `cursorWorld`/`crosshair`, `selection`, `snap*`/`ortho`/`grid` and the
    // snap spacing stay exactly as the baseline set them: `null`/`false`/empty,
    // or the documented `DEFAULT_SNAP_SPACING` for the one field the frozen wire
    // shape requires to be a usable number. None of them has an implementation on
    // this carrier, so none of them may claim one.

    let mut wire = to_json_value(&status);
    if let Some(object) = wire.as_object_mut() {
        // The carrier flag the chrome dispatches on. It describes the carrier this
        // module implements, not a per-call measurement.
        object.insert("surfaceMode".to_string(), json!(SURFACE_MODE_DOM_CANVAS));
        // No host-side document state exists on this carrier, so the palettes have
        // nothing to project (iron rule 8). An affirmative `false`, never a
        // silently missing key.
        object.insert("palettesAvailable".to_string(), json!(false));
        // The session's own descriptor, in the shape `status()` publishes and the
        // Linux host relays. Absent (not `null`) without a live session, so
        // "no session" and "a session with no label" cannot be confused.
        if let Some(live) = &facts.session {
            object.insert(
                "canvasSession".to_string(),
                json!({
                    "label": live.label,
                    "batches": live.batches,
                    "draws": live.draws,
                }),
            );
        }
    }
    wire.to_string()
}

/// The command names this carrier can execute: **none, today** (see the module
/// docs).
///
/// An empty array is the honest answer, not a placeholder: this crate has no
/// command router, and the names `cad_ui_core::session::admissible_names()` lists
/// belong to the desktop session. `OPEN` is not listed either — opening here is
/// the carrier's own `open_bytes`, which the canvas transport calls with the file
/// bytes.
pub fn commands_json() -> String {
    json!([]).to_string()
}

/// The command aliases this carrier resolves: **none**, for the same reason as
/// [`commands_json`].
///
/// The desktop alias table (`cad_ui_core::session::alias_pairs()`) maps names onto
/// a router this carrier does not have, so publishing it would let the chrome
/// accept an alias and then silently do nothing.
pub fn aliases_json() -> String {
    json!([]).to_string()
}

/// Refuse one command with the stable [`SHELL_COMMAND_REFUSAL`] prefix.
///
/// The requested name is echoed with `{:?}` so a caller sees exactly what it sent
/// (including a newline or quote) without the refusal prose being spoofable. This
/// never returns `Ok`: the browser carrier is view/navigate only (open, pan, zoom,
/// resize), and an unimplemented command must fail loudly rather than look like an
/// empty success.
pub fn command_json(command: &str) -> Result<String, String> {
    Err(format!(
        "{SHELL_COMMAND_REFUSAL}: {command:?} is not executable — the browser carrier is \
         view/navigate only (open, pan, zoom, resize; it has no command router)"
    ))
}

/// The honest all-capabilities-off baseline every field starts from.
///
/// Kept in one place so the module never hand-builds a `CadStatus`: the frozen
/// field set, the always-present `selection` block and the `DEFAULT_SNAP_SPACING`
/// rule come from `cad-ui-core` itself. (Note: the empty status is
/// `unsupported_status(None)` — `CadStatus` has no `empty()` constructor; the
/// `empty()` in that module belongs to `CadSelection`, which this baseline
/// carries.)
fn baseline() -> CadStatus {
    unsupported_status(None)
}

/// Serialize the frozen status, then treat the object as a map we may extend.
///
/// `CadStatus` is a plain struct of strings/numbers/bools/`Option`s/`Vec`s, so it
/// always serializes; the `expect` documents that this is not a fallible parse.
fn to_json_value(status: &CadStatus) -> Value {
    serde_json::to_value(status).expect("CadStatus is a plain struct of JSON values")
}

// ---------------------------------------------------------------------------
// Exported entry points
// ---------------------------------------------------------------------------

/// The JSON `cad_status` snapshot for the shared React chrome (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_status() -> String {
    status_json(&crate::session::shell_facts())
}

/// `[]`: no command is executable on this carrier (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_commands() -> String {
    commands_json()
}

/// `[]`: no alias resolves on this carrier (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_aliases() -> String {
    aliases_json()
}

/// Always the stable `unsupported-runtime: cad_command` refusal (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_command(command: &str) -> Result<String, JsValue> {
    command_json(command).map_err(|message| JsValue::from_str(&message))
}

/// Native builds have no browser registry to project, so the status is the honest
/// no-session snapshot — the same contract, with nothing invented.
///
/// This is deliberately **not** a `NOT_IMPLEMENTED` string: the payload is a real
/// (empty) shell status, which is what lets the host unit tests assert the exact
/// wire shape (`cargo test -p app-web`). A native build of this crate can never
/// have a session ([`crate::NOT_IMPLEMENTED`] is what the session entry points
/// return), so this is the whole truth on that target.
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_status() -> String {
    status_json(&SessionFacts::default())
}

/// `[]` on every target: the command vocabulary is a property of the carrier, not
/// of the build (see [`commands_json`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_commands() -> String {
    commands_json()
}

/// `[]` on every target (see [`aliases_json`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_aliases() -> String {
    aliases_json()
}

/// The same stable refusal as the wasm build: no target of this crate has a
/// command router, so this is a capability statement, not a platform limitation.
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_command(command: &str) -> Result<String, String> {
    command_json(command)
}

// ---------------------------------------------------------------------------
// Tests (host target: they prove the wire contract, not the browser)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys the chrome's `parseCadStatus` requires on **every** payload
    /// (`apps/desktop-ui/src/cad/cad-status.ts`). A missing one is a hard
    /// `CAD_STATUS_INVALID`, so this list is the actual contract, not a sample.
    const REQUIRED_KEYS: [&str; 21] = [
        "documentLabel",
        "entityCount",
        "loading",
        "importProgress",
        "error",
        "canNew",
        "canOpen",
        "canSave",
        "canPlot",
        "canFit",
        "backend",
        "adapter",
        "camera",
        "selection",
        "gridEnabled",
        "snapGridEnabled",
        "snapSpacing",
        "orthoEnabled",
        "cursorWorld",
        "importCount",
        "frameCount",
    ];

    /// A live session built from values a real one could report.
    fn live() -> LiveSession {
        LiveSession {
            label: "synthetic-four-lines.dwg".to_string(),
            batches: 4,
            draws: 4,
            presented: 7,
            adapter: format!("{BACKEND_WEBGL2}: example adapter (wasm32 session)"),
            camera: Some(CadCamera {
                center: [10.0, -5.0],
                scale: 0.25,
            }),
            error: None,
        }
    }

    fn parse(wire: &str) -> Value {
        serde_json::from_str(wire).expect("the shell status must be valid JSON")
    }

    #[test]
    fn status_is_valid_json_with_the_frozen_keys_and_the_carrier() {
        for facts in [
            SessionFacts::default(),
            SessionFacts {
                session: Some(live()),
                loading: false,
                import_count: 1,
                error: None,
            },
        ] {
            let wire = parse(&status_json(&facts));
            let object = wire.as_object().expect("the status is a JSON object");
            for key in REQUIRED_KEYS {
                assert!(
                    object.contains_key(key),
                    "missing required key {key}: {wire}"
                );
            }
            assert_eq!(object["surfaceMode"], json!(SURFACE_MODE_DOM_CANVAS));
            assert_eq!(object["palettesAvailable"], json!(false));
            // The frozen wire shape requires a real, usable spacing.
            assert!(
                object["snapSpacing"]
                    .as_f64()
                    .is_some_and(|value| value > 0.0),
                "snapSpacing must stay a positive number: {wire}"
            );
            // No cursor is ever synthesized.
            assert_eq!(object["cursorWorld"], json!(null));
        }
    }

    #[test]
    fn status_reports_no_document_capability() {
        for facts in [
            SessionFacts::default(),
            SessionFacts {
                session: Some(live()),
                loading: false,
                import_count: 1,
                error: None,
            },
        ] {
            let wire = parse(&status_json(&facts));
            for key in ["canNew", "canOpen", "canSave", "canPlot", "canFit"] {
                assert_eq!(wire[key], json!(false), "{key} must be false: {wire}");
            }
            // The database is released after the open: no entity count exists.
            assert_eq!(wire["entityCount"], json!(null));
            // No picking/tool/popup/grip state exists on this carrier.
            assert_eq!(wire["tool"], json!(null));
            assert_eq!(wire["popup"], json!(null));
            assert_eq!(wire["commandLog"], json!([]));
            assert_eq!(wire["gripDrag"], json!(false));
            assert_eq!(wire["selection"]["count"], json!(0));
        }
    }

    #[test]
    fn no_session_reports_nothing_invented() {
        let wire = parse(&status_json(&SessionFacts::default()));
        assert_eq!(wire["documentLabel"], json!(null));
        assert_eq!(wire["backend"], json!(null));
        assert_eq!(wire["adapter"], json!(null));
        assert_eq!(wire["camera"], json!(null));
        assert_eq!(wire["frameCount"], json!(0));
        assert_eq!(wire["importCount"], json!(0));
        // Absent, not null: "no session" must not look like "a session without a
        // label".
        assert!(wire.get("canvasSession").is_none(), "{wire}");
    }

    #[test]
    fn live_session_overlays_only_its_own_facts() {
        let facts = SessionFacts {
            session: Some(live()),
            loading: false,
            import_count: 1,
            error: None,
        };
        let wire = parse(&status_json(&facts));
        assert_eq!(wire["documentLabel"], json!("synthetic-four-lines.dwg"));
        assert_eq!(wire["backend"], json!(BACKEND_WEBGL2));
        assert!(wire["adapter"].as_str().unwrap().contains("wasm32"));
        assert_eq!(wire["frameCount"], json!(7));
        assert_eq!(wire["importCount"], json!(1));
        assert_eq!(wire["camera"]["center"], json!([10.0, -5.0]));
        assert_eq!(wire["camera"]["scale"], json!(0.25));
        assert_eq!(
            wire["canvasSession"],
            json!({"label": "synthetic-four-lines.dwg", "batches": 4, "draws": 4})
        );
        // A live session still cannot save, plot, refit or count entities.
        assert_eq!(wire["canSave"], json!(false));
        assert_eq!(wire["canFit"], json!(false));
        assert_eq!(wire["entityCount"], json!(null));
    }

    #[test]
    fn a_failure_is_reported_instead_of_looking_healthy() {
        let wire = parse(&status_json(&SessionFacts {
            session: None,
            loading: false,
            import_count: 0,
            error: Some("webgl2: no context".to_string()),
        }));
        assert_eq!(wire["error"], json!("webgl2: no context"));
        // An in-flight open is real state, not a guess.
        let wire = parse(&status_json(&SessionFacts {
            loading: true,
            ..SessionFacts::default()
        }));
        assert_eq!(wire["loading"], json!(true));
    }

    #[test]
    fn the_command_vocabulary_is_empty() {
        assert_eq!(commands_json(), "[]");
        assert_eq!(aliases_json(), "[]");
        assert_eq!(shell_commands(), "[]");
        assert_eq!(shell_aliases(), "[]");
        // ...and it parses as an array, not as a string that merely looks like one.
        assert_eq!(parse(&shell_commands()), json!([]));
        assert_eq!(parse(&shell_aliases()), json!([]));
    }

    #[test]
    fn a_command_is_refused_with_the_stable_prefix() {
        for command in ["OPEN", "SAVE", "", "cad_status"] {
            let error = shell_command(command).expect_err("no command is executable");
            assert!(
                error.starts_with(SHELL_COMMAND_REFUSAL),
                "the refusal must be machine-readable: {error}"
            );
            assert!(
                error.contains("unsupported-runtime"),
                "the refusal must carry the stable prefix: {error}"
            );
            assert!(
                error.contains("view/navigate only"),
                "the refusal must say what the carrier can do: {error}"
            );
        }
    }
}
