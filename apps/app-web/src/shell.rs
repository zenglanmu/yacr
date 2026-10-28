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
//! It is deliberately *not* a second session or a second document (ADR 0024 D2 /
//! iron rule 8): it only **projects** facts the session already owns, and it
//! **executes** exactly the selectors that session really implements — refusing
//! everything else by name instead of pretending to route it.
//!
//! # Exports (all return JSON strings)
//!
//! ```text
//! shell_status()            -> JSON `cad_status` snapshot
//! shell_commands()          -> JSON array of executable command names
//!                              (["FIT", "ZOOMIN", "ZOOMOUT"])
//! shell_aliases()           -> JSON array of `{alias, command}` rows whose target
//!                              is executable here ([{"alias":"ZOOM EXTENTS",
//!                              "command":"FIT"}])
//! shell_command(command)    -> `{"accepted":true}`, or Err("unsupported-runtime:
//!                              cad_command: …")
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
//! | `canOpen` | `false` | opening is the page's `open_bytes(bytes)`, not a command served from here; the **transport** (`apps/desktop-ui/src/host/web-adapter.ts`) owns the file picker and reports `canOpen: true` |
//! | `canSave`/`canPlot` | `false` | no save/plot path, and the database is released (no second document to write) |
//! | `canFit` | `true` for a live session **with retained bounds** | `FIT` re-runs the open's own fit rule from the two retained world points ([`crate::view::DrawingBounds`]); `false` without a session or without bounds, because then there is nothing to frame |
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
//! Four fields are added on top of the frozen `CadStatus` shape (the same way
//! the Linux `dom-canvas` host adds them):
//!
//! * `surfaceMode: "dom-canvas"` — the carrier (`apps/desktop-ui/src/cad/surface.ts`
//!   selects the in-DOM canvas path on exactly this string);
//! * `palettesAvailable: false` — this carrier holds **no host-side document
//!   state** for the layers/properties/diagnostics palettes to project (iron rule
//!   8). Note for the page shell: `palettesOffered()` reads that flag from the
//!   **shell** object, so a wrapper that builds `{ status }` must lift this value
//!   out of the status instead of nesting it;
//! * `standardViews: []` — the standard-view capability list, **empty**: there is
//!   no `cad_standard_view` mapping here (see the `#[doc]` on every other carrier
//!   field below). The chrome reads a present list as authoritative and disables
//!   the seven view buttons with their own reason; an absent list would mean "the
//!   host declared nothing" and keep them enabled;
//! * `canvasSession: {label, batches, draws}` — the session's own descriptor,
//!   present only while a session is live (the same shape `status()` publishes and
//!   the Linux host relays), never a mirrored document.
//!
//! # `shell_commands`/`shell_aliases`: the vocabulary this carrier executes
//!
//! `cad_ui_core::session::admissible_names()` is the *desktop* command
//! vocabulary: most of those names are routed by `cad_host`/`CadSession`, which
//! this crate does not have. Claiming them would make the chrome (ribbon, command
//! line) offer buttons that nothing can execute, and the first click would be a
//! silent nothing.
//!
//! So the list is exactly the selectors this crate implements **end to end**, each
//! added at the same time as its implementation and its contract test — never
//! ahead of it:
//!
//! | name | what the session does |
//! |---|---|
//! | `FIT` | refits the camera with the open's own fit rule, from the retained bounds ([`EXECUTABLE_COMMANDS`]) |
//! | `ZOOMIN` | one [`cad_ui_core::session::ZOOM_IN_FACTOR`] step about the viewport centre |
//! | `ZOOMOUT` | one [`cad_ui_core::session::ZOOM_OUT_FACTOR`] step about the viewport centre |
//!
//! **This list is the production viewer's**, not the desktop router's. The two
//! coexist on purpose until the production flip: [`crate::direct`] serves
//! `cad_commands`/`cad_command_aliases` from the **shared** tables
//! (`cad_ui_core::session::wire`), i.e. the full desktop selector set, for a host
//! that runs the real `CadSession`; [`commands_json`] keeps advertising only what
//! the *viewer* session implements, so nothing the current page can render changes.
//! Flipping the page to the `direct` transport replaces the short list with the full
//! one in the same step — that is the production switch, not this module's job.
//!
//! `OPEN` is deliberately **not** in this list: opening a file here is
//! [`open_bytes`](crate::open_bytes) — the canvas transport owns the bytes, the
//! file picker and the canvas — so it is implemented by the page shell
//! (`apps/desktop-ui/src/host/web-adapter.ts`), not by this module. Everything else
//! (layers, settings, measurements, drawing tools, `PAN`, …) is refused by name.
//!
//! `shell_aliases` publishes only the rows of the desktop table
//! (`cad_ui_core::session::alias_pairs()`) whose **target is executable here** —
//! today `ZOOM EXTENTS → FIT`. No alias is invented, and a row pointing at a
//! selector this carrier cannot run would be a suggestion that always fails, so it
//! is filtered out. [`resolve_command`] honours every published row.
//!
//! # `shell_command`: execute, or a stable refusal — never an empty success
//!
//! A published name is executed against the one live session and answers
//! `{"accepted":true}` (the reply `cad_command` requires); a name this carrier
//! cannot run is refused with the stable machine prefix
//! [`SHELL_COMMAND_REFUSAL`] plus one sentence naming the selector, so a caller can
//! key on the prefix and a human reads why. Returning `Ok("")` would look like a
//! success to the chrome and to any future caller.
//!
//! # Native builds
//!
//! The JSON contract is target-independent (it describes the carrier this crate
//! implements, and degrades to the honest no-session snapshot where no session
//! exists), which is what lets `cargo test -p app-web` assert the exact payload on
//! the host instead of only in a browser. [`resolve_command`] — the whole
//! vocabulary — is target-independent too, so the accepted path of every published
//! name is unit-tested on the host through [`dispatch_json`] with a recording
//! [`Runner`]. What a native build cannot do is *execute*: it has no session
//! ([`crate::NOT_IMPLEMENTED`]), so [`shell_command`] resolves the name and then
//! reports that honest platform failure instead of a fabricated `accepted`.

use serde_json::{json, Value};

use cad_ui_core::session::{
    alias_pairs, unsupported_status, CadCamera, CadStatus, ZOOM_IN_FACTOR, ZOOM_OUT_FACTOR,
};

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
/// channel this carrier answers by name.
pub const SHELL_COMMAND_REFUSAL: &str = "unsupported-runtime: cad_command";

/// The selectors this carrier executes **end to end**, in the order the chrome
/// advertises them.
///
/// Every name here has a real implementation in the wasm session (`crate::session`,
/// wasm-only) and a contract test; nothing is listed ahead of its implementation, and
/// `OPEN` is not listed because the page shell implements it (see the module docs).
pub const EXECUTABLE_COMMANDS: [&str; 3] = ["FIT", "ZOOMIN", "ZOOMOUT"];

/// One resolved command: what the live session has to do.
///
/// Kept target-independent (no session handle) so the vocabulary and its dispatch
/// are unit-testable on the host, not only in a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Execution {
    /// Reframe the camera with the open's own fit rule, from the retained bounds.
    Fit,
    /// One [`ZOOM_IN_FACTOR`] step about the viewport centre.
    ZoomIn,
    /// One [`ZOOM_OUT_FACTOR`] step about the viewport centre.
    ZoomOut,
}

impl Execution {
    /// The selector this step was resolved from (the name `cad_command` sent).
    pub fn selector(self) -> &'static str {
        match self {
            Execution::Fit => "FIT",
            Execution::ZoomIn => "ZOOMIN",
            Execution::ZoomOut => "ZOOMOUT",
        }
    }

    /// The zoom factor of a fixed step, or `None` for [`Execution::Fit`].
    ///
    /// The factors are `cad-ui-core`'s own constants, so the browser step and the
    /// desktop `ZOOMIN`/`ZOOMOUT` cannot drift apart.
    pub fn zoom_factor(self) -> Option<f64> {
        match self {
            Execution::Fit => None,
            Execution::ZoomIn => Some(ZOOM_IN_FACTOR),
            Execution::ZoomOut => Some(ZOOM_OUT_FACTOR),
        }
    }
}

/// Something that can carry out one [`Execution`] against a live session.
///
/// The wasm build implements this over the process-global session
/// (`SessionRunner`, wasm-only); host tests implement it as a recorder, which is how
/// the accepted path of every published selector is asserted without a browser.
pub trait Runner {
    /// Run `step`; the error text is the session's own diagnostic.
    fn run(&self, step: Execution) -> Result<(), String>;
}

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
    /// Whether the import retained the drawing bounds this session's fit used.
    ///
    /// `true` is what makes `FIT` executable (`cad_status.canFit`); `false` means
    /// the imported database reported no bounds, so there is nothing to reframe and
    /// a `true` would advertise a command that cannot run.
    pub has_bounds: bool,
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
        // `FIT` is executable iff the import retained the bounds it fitted to.
        // That is the session's own observation, so the chrome can enable the
        // button exactly when the command will run.
        status.can_fit = live.has_bounds;
    }
    // A live session's own last failure wins; otherwise report the failure that
    // ended a start attempt, so a failed carrier never looks healthy.
    status.error = facts
        .session
        .as_ref()
        .and_then(|live| live.error.clone())
        .or_else(|| facts.error.clone());

    // `entityCount`, `canNew`, `canOpen`, `canSave`, `canPlot`, `importProgress`,
    // `tool`, `popup`, `commandLog`, `gripDrag`, `cursorWorld`/`crosshair`,
    // `selection`, `snap*`/`ortho`/`grid` and the snap spacing stay exactly as the
    // baseline set them: `null`/`false`/empty, or the documented
    // `DEFAULT_SNAP_SPACING` for the one field the frozen wire shape requires to be
    // a usable number. None of them has an implementation on this carrier, so none
    // of them may claim one. (`canOpen` is not set here on purpose: the file picker
    // is the **transport's** capability, so `apps/desktop-ui/src/host/web-adapter.ts`
    // lifts it, not this module.)

    let mut wire = to_json_value(&status);
    if let Some(object) = wire.as_object_mut() {
        // The carrier flag the chrome dispatches on. It describes the carrier this
        // module implements, not a per-call measurement.
        object.insert("surfaceMode".to_string(), json!(SURFACE_MODE_DOM_CANVAS));
        // No host-side document state exists on this carrier, so the palettes have
        // nothing to project (iron rule 8). An affirmative `false`, never a
        // silently missing key.
        object.insert("palettesAvailable".to_string(), json!(false));
        // The standard-view capability list: **empty, on purpose**. This carrier
        // has no `cad_standard_view` mapping at all. The wasm session is a 2D
        // presenter — it imports, draws and presets a `camera2d` view, and has no
        // 3D camera, no `StandardView` state and no transport channel behind the
        // seven view names. Reporting `[]` (not omitting the field) is the honest
        // answer: absent would mean "this host declared nothing", which the chrome
        // keeps enabling as before, and tapping a view then throws
        // `unsupported-runtime: cad_standard_view` from a button that looked live.
        // `[]` makes `standardViewAvailable` refuse every view **before** IPC.
        object.insert("standardViews".to_string(), json!([]));
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

/// The command names this carrier executes: [`EXECUTABLE_COMMANDS`].
///
/// Not the desktop router's vocabulary — only the selectors the wasm session
/// (`crate::session`, wasm-only) really implements (see the module docs). `OPEN` is
/// served by the page shell.
pub fn commands_json() -> String {
    json!(EXECUTABLE_COMMANDS).to_string()
}

/// The alias rows this carrier resolves: the desktop table filtered to targets it
/// can execute.
///
/// `cad_ui_core::session::alias_pairs()` is the single alias source; a row whose
/// target is not in [`EXECUTABLE_COMMANDS`] is dropped, because a suggestion that
/// this carrier would refuse is worse than no suggestion. The wire shape is the
/// desktop one (`{alias, command}`), which is what `setCommandAliases` in
/// `apps/desktop-ui/src/commands/commands.ts` reads.
pub fn aliases() -> Vec<(&'static str, &'static str)> {
    alias_pairs()
        .into_iter()
        .filter(|(_, target)| EXECUTABLE_COMMANDS.contains(target))
        .collect()
}

/// The JSON alias table (see [`aliases`]).
pub fn aliases_json() -> String {
    json!(aliases()
        .into_iter()
        .map(|(alias, command)| json!({ "alias": alias, "command": command }))
        .collect::<Vec<_>>())
    .to_string()
}

/// The stable refusal for one name this carrier cannot execute.
///
/// The requested text is echoed with `{:?}` so a caller sees exactly what it sent
/// (including a newline or quote) without the refusal prose being spoofable.
pub fn refusal(command: &str) -> String {
    format!(
        "{SHELL_COMMAND_REFUSAL}: {command:?} is not executable — this carrier executes \
         {EXECUTABLE_COMMANDS:?} only (open is the page's file picker, pan/scroll are the \
         pointer transport; there is no command router)"
    )
}

/// Normalise one command line the way the router does: trim, and upper-case the
/// **selector** while keeping the rest of the line as typed.
///
/// The three published selectors take no argument, but the normalisation is the
/// documented one (not a special case per name), so an unexpected argument is
/// preserved verbatim in the refusal instead of being silently folded.
pub fn normalize_line(line: &str) -> String {
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let selector = parts.next().unwrap_or("").to_ascii_uppercase();
    match parts.next().map(str::trim) {
        Some(rest) if !rest.is_empty() => format!("{selector} {rest}"),
        _ => selector,
    }
}

/// Resolve one typed line to the step it asks for, or the stable refusal.
///
/// This is the whole vocabulary: the three canonical selectors plus every
/// **published** alias (`ZOOM EXTENTS → FIT`). Matching is case-insensitive on the
/// normalised line, exactly like `cad_ui_core::session::canonical_command_name`;
/// anything else is [`refusal`]. Nothing is executed here, which is what makes the
/// accepted path assertable on the host.
pub fn resolve_command(command: &str) -> Result<Execution, String> {
    let key = normalize_line(command);
    let step = match key.as_str() {
        "FIT" => Some(Execution::Fit),
        "ZOOMIN" => Some(Execution::ZoomIn),
        "ZOOMOUT" => Some(Execution::ZoomOut),
        _ => aliases()
            .into_iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(&key))
            .and_then(|(_, target)| match target {
                "FIT" => Some(Execution::Fit),
                "ZOOMIN" => Some(Execution::ZoomIn),
                "ZOOMOUT" => Some(Execution::ZoomOut),
                _ => None,
            }),
    };
    step.ok_or_else(|| refusal(command))
}

/// The wire reply a completed command answers with.
///
/// `queueCadCommand` (`apps/desktop-ui/src/cad/cad-command.ts`) requires exactly
/// `accepted: true`; anything else counts as a failure, so this is the only success
/// shape emitted.
pub fn accepted_json() -> String {
    json!({ "accepted": true }).to_string()
}

/// Execute one line through `runner` and answer its wire reply.
///
/// A refusal never reaches the runner (an unimplemented name has nothing to run),
/// and a runner error is the session's own diagnostic, handed on unchanged instead
/// of being turned into a success.
pub fn dispatch_json(runner: &impl Runner, command: &str) -> Result<String, String> {
    runner.run(resolve_command(command)?)?;
    Ok(accepted_json())
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
/// `["FIT","ZOOMIN","ZOOMOUT"]`: the selectors the wasm session executes (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_commands() -> String {
    commands_json()
}

/// The desktop alias rows whose target this carrier executes (wasm).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_aliases() -> String {
    aliases_json()
}

/// Execute one published selector against the live session, or refuse it.
///
/// A published name (`FIT`/`ZOOMIN`/`ZOOMOUT`, or a published alias of one) runs on
/// the one wasm session through [`SessionRunner`] and answers
/// `{"accepted":true}`; everything else is the stable
/// [`SHELL_COMMAND_REFUSAL`]. A session failure (no live session, a navigation
/// error) is the session's own message, never a rewritten success.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_command(command: &str) -> Result<String, JsValue> {
    dispatch_json(&SessionRunner, command).map_err(|message| JsValue::from_str(&message))
}

/// The wasm [`Runner`]: every step is one call on the process-global session.
#[cfg(target_arch = "wasm32")]
struct SessionRunner;

#[cfg(target_arch = "wasm32")]
impl Runner for SessionRunner {
    fn run(&self, step: Execution) -> Result<(), String> {
        match step.zoom_factor() {
            // `FIT` reframes from the bounds the open retained.
            None => crate::session::fit_view_impl()
                .map(|_status| ())
                .map_err(describe_session_error),
            // `ZOOMIN`/`ZOOMOUT` are fixed steps about the viewport centre.
            Some(factor) => crate::session::zoom_step_impl(factor)
                .map(|_status| ())
                .map_err(describe_session_error),
        }
    }
}

/// The text of a session `Err(JsValue)` (the session always rejects with a string;
/// a non-string is described rather than swallowed).
#[cfg(target_arch = "wasm32")]
fn describe_session_error(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
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

/// The same vocabulary on every target: it is a property of the carrier, not of the
/// build (see [`commands_json`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_commands() -> String {
    commands_json()
}

/// The same filtered alias table on every target (see [`aliases_json`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_aliases() -> String {
    aliases_json()
}

/// Resolve the name, then report the platform failure: a native build has no wasm
/// session to run it on, so it must never answer `accepted`.
///
/// An unlisted name keeps the stable [`SHELL_COMMAND_REFUSAL`]; a listed one is
/// [`crate::NOT_IMPLEMENTED`], exactly like the other native canvas entry points.
#[cfg(not(target_arch = "wasm32"))]
pub fn shell_command(command: &str) -> Result<String, String> {
    resolve_command(command)?;
    Err(crate::NOT_IMPLEMENTED.to_string())
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
        live_with_bounds(true)
    }

    /// A live session with or without retained drawing bounds (`FIT` viability).
    fn live_with_bounds(has_bounds: bool) -> LiveSession {
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
            has_bounds,
            error: None,
        }
    }

    /// A [`Runner`] that records what it was asked to do (the host-side accepted
    /// path, without a browser).
    #[derive(Default)]
    struct RecordingRunner {
        seen: std::cell::RefCell<Vec<Execution>>,
        fail: Option<String>,
    }

    impl RecordingRunner {
        fn failing(message: &str) -> Self {
            Self {
                seen: std::cell::RefCell::new(Vec::new()),
                fail: Some(message.to_string()),
            }
        }

        fn seen(&self) -> Vec<Execution> {
            self.seen.borrow().clone()
        }
    }

    impl Runner for RecordingRunner {
        fn run(&self, step: Execution) -> Result<(), String> {
            self.seen.borrow_mut().push(step);
            match &self.fail {
                Some(message) => Err(message.clone()),
                None => Ok(()),
            }
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

    /// The standard-view capability list is present and **empty**: this carrier
    /// executes no standard view, and `[]` is the affirmative answer that makes the
    /// chrome disable the seven buttons with their own reason. Omitting the field
    /// would read as "host declared nothing" and leave them enabled.
    #[test]
    fn standard_views_are_affirmatively_unavailable() {
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
            let views = wire
                .get("standardViews")
                .unwrap_or_else(|| panic!("standardViews must be present: {wire}"));
            assert_eq!(views, &json!([]), "standardViews must be empty: {wire}");
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
            // `canFit` is the one capability this carrier has (a live session with
            // retained bounds), and it is asserted separately below.
            for key in ["canNew", "canOpen", "canSave", "canPlot"] {
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

    /// `canFit` is exactly "a live session whose import retained the bounds".
    #[test]
    fn can_fit_needs_a_live_session_with_retained_bounds() {
        let facts = |session: Option<LiveSession>| SessionFacts {
            session,
            loading: false,
            import_count: 1,
            error: None,
        };
        // No session: nothing to refit, so nothing is advertised.
        assert_eq!(parse(&status_json(&facts(None)))["canFit"], json!(false));
        // A session that retained no bounds: the fit had nothing to frame.
        assert_eq!(
            parse(&status_json(&facts(Some(live_with_bounds(false)))))["canFit"],
            json!(false)
        );
        // A session with the import's bounds: `FIT` really can run.
        assert_eq!(
            parse(&status_json(&facts(Some(live_with_bounds(true)))))["canFit"],
            json!(true)
        );
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
        // A live session still cannot save, plot or count entities — but it can
        // refit, because the import retained the bounds (`has_bounds`).
        assert_eq!(wire["canSave"], json!(false));
        assert_eq!(wire["canFit"], json!(true));
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
    fn the_command_vocabulary_is_exactly_what_the_session_executes() {
        // The exact list, in order — the chrome advertises this and nothing else.
        assert_eq!(
            parse(&shell_commands()),
            json!(["FIT", "ZOOMIN", "ZOOMOUT"])
        );
        assert_eq!(parse(&commands_json()), json!(["FIT", "ZOOMIN", "ZOOMOUT"]));
        // Every published name is a real admitted selector of the shared table, so
        // no name is invented here.
        let admitted = cad_ui_core::session::admissible_names();
        for name in EXECUTABLE_COMMANDS {
            assert!(
                admitted.contains(&name),
                "{name} is not an admitted selector"
            );
        }
        // `OPEN` is the page shell's command, never this carrier's.
        assert!(!EXECUTABLE_COMMANDS.contains(&"OPEN"));
        assert_eq!(shell_commands(), commands_json());
        assert_eq!(shell_aliases(), aliases_json());
    }

    #[test]
    fn the_alias_table_only_points_at_executable_targets() {
        assert_eq!(
            parse(&aliases_json()),
            json!([{ "alias": "ZOOM EXTENTS", "command": "FIT" }])
        );
        // Every row is a real row of the desktop table, filtered — never invented,
        // and never pointing at a name this carrier would refuse.
        let desktop = cad_ui_core::session::alias_pairs();
        for (alias, target) in aliases() {
            assert!(desktop.contains(&(alias, target)), "{alias} -> {target}");
            assert!(EXECUTABLE_COMMANDS.contains(&target), "{alias} -> {target}");
        }
        // The alias resolves to the same step as its target, so a published alias
        // can never be a suggestion that always fails.
        assert_eq!(resolve_command("ZOOM EXTENTS"), resolve_command("FIT"));
        assert_eq!(resolve_command("zoom extents"), Ok(Execution::Fit));
    }

    #[test]
    fn each_published_command_resolves_and_is_dispatched() {
        let runner = RecordingRunner::default();
        for (line, expected) in [
            ("FIT", Execution::Fit),
            ("ZOOMIN", Execution::ZoomIn),
            ("ZOOMOUT", Execution::ZoomOut),
            // Trim + selector case are normalised like the router does.
            ("  zoomin ", Execution::ZoomIn),
        ] {
            assert_eq!(resolve_command(line), Ok(expected), "{line}");
            assert_eq!(dispatch_json(&runner, line), Ok(accepted_json()), "{line}");
        }
        assert_eq!(
            runner.seen(),
            vec![
                Execution::Fit,
                Execution::ZoomIn,
                Execution::ZoomOut,
                Execution::ZoomIn
            ]
        );
        // The reply is the only shape `queueCadCommand` treats as accepted, and the
        // steps are the desktop factors.
        assert_eq!(parse(&accepted_json()), json!({"accepted": true}));
        assert_eq!(Execution::ZoomIn.zoom_factor(), Some(ZOOM_IN_FACTOR));
        assert_eq!(Execution::ZoomOut.zoom_factor(), Some(ZOOM_OUT_FACTOR));
        assert_eq!(Execution::Fit.zoom_factor(), None);
    }

    #[test]
    fn a_session_failure_is_never_turned_into_an_accepted_reply() {
        let runner = RecordingRunner::failing("navigation: no drawing bounds");
        let error = dispatch_json(&runner, "FIT").expect_err("the session refused");
        assert_eq!(error, "navigation: no drawing bounds");
    }

    #[test]
    fn a_command_is_refused_with_the_stable_prefix() {
        for command in ["OPEN", "PAN", "SAVE", "ZOOM", "GARBAGE", "", "cad_status"] {
            let error = resolve_command(command).expect_err("not an executable command");
            assert!(
                error.starts_with(SHELL_COMMAND_REFUSAL),
                "the refusal must be machine-readable: {error}"
            );
            assert!(
                error.contains("unsupported-runtime"),
                "the refusal must carry the stable prefix: {error}"
            );
            assert!(
                error.contains(&format!("{command:?}")),
                "the refusal must name the selector it refused: {error}"
            );
        }
    }

    /// A native build resolves a published name but cannot execute it: no session
    /// exists on this target, so it says so instead of answering `accepted`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_native_build_reports_the_platform_failure_not_a_success() {
        for command in EXECUTABLE_COMMANDS {
            let error = shell_command(command).expect_err("no session exists natively");
            assert_eq!(error, crate::NOT_IMPLEMENTED.to_string());
        }
        // …and an unlisted name keeps the stable refusal.
        let error = shell_command("OPEN").expect_err("OPEN is the page's command");
        assert!(error.starts_with(SHELL_COMMAND_REFUSAL), "{error}");
    }
}
