//! The frozen frontend `cad_status` snapshot and its honest projections.
//!
//! Every key is always present, an unknown value is `null`, and a count is
//! never fabricated. This module carries no host state: it only projects
//! already-derived values.

use serde::Serialize;

use cad_app::ToolSnapshot;

/// Camera block of the frontend status (`{center, scale}`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CadCamera {
    /// World point at the centre of the viewport.
    pub center: [f64; 2],
    /// World units per logical pixel (strictly positive).
    pub scale: f64,
}

/// Live import progress for the frontend progress affordance.
///
/// Present **only while an import is genuinely running**: a terminal snapshot
/// (opened/cancelled/failed) is never projected here, so an idle or finished
/// document carries `importProgress: null` and the chrome shows no bar. Every
/// field is a real observed value — `entities_total`/`bytes` stay `null` when
/// the importer did not expose them, which the UI renders as an indeterminate
/// bar without a byte count; nothing is extrapolated to 100%.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadImportProgress {
    /// Stable machine key of the latest phase (e.g. `"entities"`); `null` before
    /// the first progress tick. The UI maps it to a catalog label.
    pub phase: Option<String>,
    /// Real entities normalised/inserted so far (`0` before the first tick).
    pub entities_done: u64,
    /// Total entities expected, or `null` when the format does not expose it.
    pub entities_total: Option<u64>,
    /// Source bytes measured so far, only when actually known.
    pub bytes: Option<u64>,
    /// Whether a cancel request can still be issued for this import.
    pub cancellable: bool,
}

impl CadImportProgress {
    /// Project a running [`cad_app::ImportProgressSnapshot`]; a terminal snapshot
    /// maps to `None`, so the affordance is only ever driven by a live import.
    pub(super) fn from_snapshot(snapshot: &cad_app::ImportProgressSnapshot) -> Option<Self> {
        if !snapshot.running {
            return None;
        }
        Some(CadImportProgress {
            phase: snapshot.phase_key().map(str::to_string),
            entities_done: snapshot.entities_done as u64,
            entities_total: snapshot.entities_total.map(|total| total as u64),
            bytes: snapshot.bytes,
            cancellable: snapshot.cancellable,
        })
    }
}

/// One row of the [`CadSelection`] block: a value plus its **stable machine
/// key** (`id`, `handle`, `type`, …), its panel group and its read-side editing
/// metadata.
///
/// The key, group and kind are never translated by the host; the UI maps each
/// to a catalog label. `value` is `null` **iff** `mixed` is `true` (a
/// multi-selection whose values differ); it is never an empty-string stand-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadSelectionRow {
    pub key: String,
    /// Shared value, or `null` when the values differ across the selection.
    pub value: Option<String>,
    /// Stable panel group machine key (`entity`, `geometry`, `text`, `display`).
    pub group: String,
    /// Whether the values differ across the selection (`value` is `null`).
    pub mixed: bool,
    /// Whether a future write path can change the value for every selected ref.
    pub editable: bool,
    /// Stable editing-control machine key (`text`, `number`, `enum`, `bool`).
    pub kind: String,
    /// Real named-table values for an `enum` row (sorted, capped by the app).
    pub options: Vec<String>,
}

/// The `selection` block of [`CadStatus`] (frozen frontend shape).
///
/// The key is **always** present — never omitted and never `null` — and it
/// describes a real selection instead of a fabricated one:
///
/// * `count >= 0` and `empty == (count == 0)`;
/// * `rows` carries property rows for any `count >= 1`. A single object gets its
///   full row set; a multi-selection gets the intersection of the refs' row
///   keys, where a differing value is `value: null` with `mixed: true`. A single
///   ref that no longer resolves still reports its honest `status: "unresolved"`
///   row rather than invented geometry;
/// * `mixedKeys` is exactly the keys of the rows with `mixed: true` (empty for a
///   single selection).
///
/// A session with no document (or no selection) carries the honest empty block
/// ([`CadSelection::empty`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadSelection {
    /// Number of selected objects.
    pub count: u64,
    /// Whether nothing is selected (`count == 0`).
    pub empty: bool,
    /// Property rows of the selection; empty only for an empty selection.
    pub rows: Vec<CadSelectionRow>,
    /// Machine keys of the mixed rows; empty for any single selection.
    pub mixed_keys: Vec<String>,
    /// The **round-trippable** current text of a single selected `Text` entity
    /// (ADR 0020 amendment B), or `null`.
    ///
    /// Distinct from [`Self::rows`]: those values are display-formatted, while
    /// this is the exact string the edit channel
    /// ([`cad_app::interaction::InteractionRequest::SetSelectedText`]) replaces.
    /// Empty for every other selection — nothing selected, a multi-selection, a
    /// non-text entity, and a block-instance member or sub-element (text editing
    /// never reaches into block internals).
    pub text: Option<String>,
    /// Whether [`Self::text`] is a **multi-line** text target: the editable
    /// string comes from `AcDbMText` or from a `MultiLeader` with text content.
    ///
    /// Always present, never `null`; `false` for an `AcDbText` and **always
    /// `false` whenever [`Self::text`] is `None`** (nothing editable). It is a
    /// **capability of the current text target**, not a display property: the
    /// docked panel and the popup apply one rule from this field instead of
    /// reading a display row such as `type`.
    ///
    /// Wire name: **`text_multiline`**. Like [`CadTool::awaiting_text`] it
    /// deliberately overrides the block's camelCase convention because that is
    /// the name the UI lane reads; emitting `textMultiline` would silently make
    /// the panel and the popup disagree.
    #[serde(rename = "text_multiline")]
    pub text_multiline: bool,
}

impl CadSelection {
    /// The honest empty selection: nothing is selected and nothing is invented.
    pub fn empty() -> Self {
        Self {
            count: 0,
            empty: true,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
            text: None,
            // Nothing editable: no text capability.
            text_multiline: false,
        }
    }

    /// A selection block with no property source: there is no authoritative
    /// document to read rows from, so only the **real** ref count is reported.
    ///
    /// No property row is fabricated, and a non-empty session selection is never
    /// hidden behind a `0` (a document replacement resets the session, so this
    /// normally *is* the empty block).
    pub(super) fn refs_only(count: usize) -> Self {
        Self {
            count: count as u64,
            empty: count == 0,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
            // No authoritative document means no editable string, never a guess.
            text: None,
            text_multiline: false,
        }
    }

    /// Project the application's read-only selection properties onto the frozen
    /// wire shape.
    ///
    /// `empty` is derived from `count` so the frozen invariant
    /// `empty == (count == 0)` holds by construction; the application derives
    /// both from the same selection set, so the two cannot disagree.
    pub(super) fn from_properties(properties: &cad_app::SelectionProperties) -> Self {
        let count = properties.count as u64;
        let rows: Vec<CadSelectionRow> = properties
            .rows
            .iter()
            .map(|row| CadSelectionRow {
                key: row.key.to_string(),
                value: row.value.clone(),
                group: row.group.key().to_string(),
                mixed: row.mixed,
                editable: row.editable,
                kind: row.kind.key().to_string(),
                options: row.options.clone(),
            })
            .collect();
        // One source of truth: the wire's `mixedKeys` is derived from the rows
        // that are actually marked mixed, so the two fields cannot drift.
        let mixed_keys = if properties.count > 1 {
            rows.iter()
                .filter(|row| row.mixed)
                .map(|row| row.key.clone())
                .collect()
        } else {
            Vec::new()
        };
        // Only a single resolved text-bearing entity (TEXT / MTEXT /
        // MULTILEADER text) carries a write-back string; the application
        // leaves it `None` for every other selection.
        let text = if count == 1 {
            properties.text.clone()
        } else {
            None
        };
        // The multiline capability is meaningless without an editable string, so
        // it is forced `false` whenever `text` is absent — even if a caller
        // handed a stale flag in.
        let text_multiline = text.is_some() && properties.text_multiline;
        Self {
            count,
            empty: count == 0,
            rows,
            mixed_keys,
            text,
            text_multiline,
        }
    }
}

/// The typed tool-parameter block of [`CadTool`] (ADR 0015 decision C, extended
/// by ADR 0016 decisions A/B/C/D/F).
///
/// A frozen frontend shape. Each field is `null` whenever the active tool does not
/// own that parameter (and the whole `tool` is `null` when no tool is active), so a
/// UI control's enablement can be derived from `tool.key` and never from a stale
/// local value. The legacy `sides` field keeps its exact name and meaning; the
/// FILLET/CHAMFER/ARRAY/ALIGN parameters are added alongside it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadToolParams {
    /// The POLYGON side count, or `null` for every other tool.
    pub sides: Option<u32>,
    /// The FILLET radius, or `null` for every other tool.
    pub fillet_radius: Option<f64>,
    /// The CHAMFER first distance, or `null` for every other tool.
    pub chamfer_distance_a: Option<f64>,
    /// The CHAMFER second distance, or `null` for every other tool.
    pub chamfer_distance_b: Option<f64>,
    /// The ARRAYRECT row count, or `null` for every other tool.
    pub array_rows: Option<f64>,
    /// The ARRAYRECT column count, or `null` for every other tool.
    pub array_columns: Option<f64>,
    /// The ARRAYRECT row spacing, or `null` for every other tool.
    pub array_row_spacing: Option<f64>,
    /// The ARRAYRECT column spacing, or `null` for every other tool.
    pub array_column_spacing: Option<f64>,
    /// The ARRAYPOLAR item count, or `null` for every other tool.
    pub array_items: Option<f64>,
    /// The ARRAYPOLAR fill angle (degrees), or `null` for every other tool.
    pub array_fill_angle: Option<f64>,
    /// The ARRAYPOLAR rotate-items flag, or `null` for every other tool.
    pub array_rotate_items: Option<bool>,
    /// The ALIGN scale flag, or `null` for every other tool.
    pub align_scale: Option<bool>,
    /// The HATCH pattern type (`0` solid, `1` user-defined), or `null` for every
    /// other tool (ADR 0018 decision D, P6b).
    pub hatch_pattern_type: Option<f64>,
    /// The HATCH user-defined pattern spacing, or `null` for every other tool.
    pub hatch_spacing: Option<f64>,
    /// The HATCH pattern angle in degrees, or `null` for every other tool.
    pub hatch_angle: Option<f64>,
    /// The HATCH `HPDOUBLE` flag, or `null` for every other tool.
    pub hatch_double: Option<bool>,
    /// The HATCH style (`0` Normal, `1` Outer, `2` Ignore), or `null` for every
    /// other tool.
    pub hatch_style: Option<f64>,
    /// The TEXT glyph height, or `null` for every other tool (ADR 0019 F.1).
    pub text_height: Option<f64>,
}

/// The active interactive tool, projected from `cad_app`'s real capture state.
///
/// Present only while a draw or measurement capture is active: an idle,
/// selecting or panning session carries `tool: null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadTool {
    /// Stable machine key: `"draw.line"`..`"draw.offset"`, `"measure.distance"`,
    /// `"measure.polyline"`, `"measure.angle"`, `"measure.area"`.
    pub key: String,
    /// Points captured so far (the real count from the preview).
    pub points: usize,
    /// Whether the tool can be confirmed right now, from `cad-app`'s own
    /// readiness ([`cad_app::DrawPreview::can_confirm`] /
    /// [`cad_app::MeasurementPreview::can_confirm`]).
    pub ready: bool,
    /// Whether the capture accepts points **past** its minimum (ADR 0015
    /// decision B), derived from `cad-app`'s own arity model
    /// ([`cad_app::DrawPreview::is_open_ended`]).
    ///
    /// `true` only for the open-ended draw kinds (PLINE, SPLINE) while their
    /// capture is active: they accept more points and finish with Enter, so the
    /// UI must not report a fixed "remaining" count for them. Every fixed-arity
    /// draw kind and **every** measurement tool reports `false` — never a
    /// fabricated open-ness.
    pub open_ended: bool,
    /// Whether the active tool is a text tool whose point complement is complete
    /// and whose text has not been submitted yet (ADR 0019 decision A.2).
    ///
    /// This is the **only** honest source for the text prompt phase; a UI must
    /// never infer it from `key` + `points`/`ready`, because `ready` alone cannot
    /// tell "missing points" from "missing text" and such an inference would
    /// drift from `cad_app`'s `requires_text()`. While it is `true` the UI enters
    /// its text-entry mode and `Confirm` is refused (the terminal step is
    /// `cad_set_tool_text`).
    ///
    /// Always `false` for a measurement capture and for `tool: null`.
    ///
    /// Wire name: **`awaiting_text`**. This field deliberately overrides the
    /// block's camelCase convention because it is the name the committed React
    /// contract reads (`apps/app-linux/ui/src/text.mjs` / `cad.mjs` validate and
    /// consume `tool.awaiting_text`); emitting `awaitingText` would silently
    /// never open the text prompt.
    #[serde(rename = "awaiting_text")]
    pub awaiting_text: bool,
    /// The current string of the entity a `TEXTEDIT` capture will replace
    /// (ADR 0020 amendment A), or `null`.
    ///
    /// Present **only** while a text tool is active and only once `TEXTEDIT`'s
    /// entity pick resolved to a single `Text`: it is that entity's stored
    /// string, so the UI can show it for editing instead of making the user
    /// retype it. A fresh `TEXT` never carries one, nor does any non-text tool
    /// or a measurement capture. It is not a display row and is never
    /// fabricated.
    ///
    /// Wire name: **`text_prefill`** (like [`Self::awaiting_text`], it keeps the
    /// snake_case name the React contract reads: `tool.text_prefill`).
    #[serde(rename = "text_prefill")]
    pub text_prefill: Option<String>,
    /// Typed tool parameters (see [`CadToolParams`]); always present, with `null`
    /// fields for a tool that has no such parameter.
    pub params: CadToolParams,
}

/// The open modal popup of the host (ADR-0021 D1/D2).
///
/// Wire shape: `{ kind, label, modal }`. It is the host's real window state, not
/// a UI guess: `kind` is the allowlisted popup kind key, `label` the native
/// window label the host built, and `modal` whether the host applied its input
/// gate for it. The block is `null` while no popup exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadPopup {
    /// Stable popup kind key (currently only `"about"`).
    pub kind: String,
    /// Native window label the host created for this popup.
    pub label: String,
    /// Whether the host holds the modal input gate for this popup.
    pub modal: bool,
}

/// Which kind of echo a [`CadCommandLogEntry`] is.
///
/// Serialized as a lowercase string (`"command"`, `"result"`, `"error"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CadCommandLogKind {
    /// A named selector was admitted by the router.
    Command,
    /// A command produced a typed measurement result.
    Result,
    /// A command was refused.
    Error,
}

/// Typed measurement echo carried by a `Result` entry.
///
/// Every field is a stable machine key or a real measured number: `algorithm`
/// is one of `distance`/`polyline`/`angle`/`area`, `unit` is a known display
/// symbol (`mm`/`m`/`in`/`ft`) or `null` for drawing units, and `precision` is
/// the modelled precision token. There is deliberately **no** text field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadMeasurementEcho {
    /// Stable machine algorithm key (never translated).
    pub algorithm: String,
    /// The measured value in the record's own unit context.
    pub value: f64,
    /// A known display unit symbol, or `null` for drawing units.
    pub unit: Option<String>,
    /// The modelled precision token (`analytic`/`approximate`/`unknown`).
    pub precision: Option<String>,
    /// The picked points, in capture order, so the UI can format deltas.
    pub points: Vec<[f64; 3]>,
}

/// One entry of the command-window echo (AutoCAD-style command history).
///
/// The type has **no** message/text field by construction: the host only ever
/// supplies stable codes/keys, and `Diagnostic::message` (partly Chinese today)
/// can never reach the wire. `command` is present only on `Command` entries and
/// `measurement` only on `Result` entries; an `Error` carries only `code`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadCommandLogEntry {
    /// Monotonic sequence, unique for the host log's lifetime.
    pub sequence: u64,
    pub kind: CadCommandLogKind,
    /// Stable selector name (`DIST`, `LINE`, …) for a `Command` entry.
    pub command: Option<String>,
    /// Stable diagnostic code for an `Error` entry.
    pub code: Option<String>,
    /// Typed measurement echo for a `Result` entry.
    pub measurement: Option<CadMeasurementEcho>,
}

impl CadCommandLogEntry {
    /// Project one host log entry onto the wire shape.
    pub(super) fn from_host(entry: &cad_host::CommandLogEntry) -> Self {
        Self {
            sequence: entry.sequence,
            kind: match entry.kind {
                cad_host::CommandLogKind::Command => CadCommandLogKind::Command,
                cad_host::CommandLogKind::Result => CadCommandLogKind::Result,
                cad_host::CommandLogKind::Error => CadCommandLogKind::Error,
            },
            command: entry.command.clone(),
            code: entry.code.clone(),
            measurement: entry
                .measurement
                .as_ref()
                .map(|measurement| CadMeasurementEcho {
                    algorithm: measurement.algorithm.clone(),
                    value: measurement.value,
                    unit: measurement.unit.clone(),
                    precision: measurement.precision.clone(),
                    points: measurement.points.clone(),
                }),
        }
    }
}

/// The `cad_status` snapshot consumed by the frontend.
///
/// The field set is the frozen frontend contract: every key is always present,
/// an unknown value is `null`, and a count is never fabricated. `error` is a
/// technical, locale-neutral string from the real host state (a command/
/// completion failure, a resource diagnostic or a render diagnostic); it is
/// never a localized UI label. `import_progress` carries the real, live open
/// progress while one is running (see [`CadImportProgress`]), `selection`
/// the real current selection (see [`CadSelection`]) and `popup` the host's open
/// modal popup (see [`CadPopup`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadStatus {
    pub document_label: Option<String>,
    pub entity_count: Option<u64>,
    pub loading: bool,
    /// Live import progress, or `null` when no import is running.
    pub import_progress: Option<CadImportProgress>,
    pub error: Option<String>,
    pub can_open: bool,
    pub can_fit: bool,
    pub backend: Option<String>,
    pub adapter: Option<String>,
    pub camera: Option<CadCamera>,
    /// The current selection; always present, never `null`.
    pub selection: CadSelection,
    /// The active draw/measurement tool, or `null` when no capture is active.
    pub tool: Option<CadTool>,
    /// Whether idle pointer movement previews (tints) the entity under the
    /// cursor. Session/display state only; defaults to `true`, matching
    /// AutoCAD's `SELECTIONPREVIEW` initial `3` (on when no command is active),
    /// and can be turned off with `HOVERPREVIEW`.
    pub hover_preview: bool,
    /// Whether object snapping is active in this session (ADR 0014 A.4).
    ///
    /// Session/display state only; defaults to `true` and can be flipped with
    /// the `OSNAP` selector (F3). It is independent of the persisted
    /// `view.overlays.snapHints` preference, which only gates the hint overlay.
    pub snap_enabled: bool,
    /// The best current object-snap candidate's stable kind string (ADR 0014
    /// A.7), or `null`.
    ///
    /// One of the real `cad_measure::SnapKind` keys (`"endpoint"`,
    /// `"midpoint"`, `"center"`, `"quadrant"`, `"perpendicular"`,
    /// `"intersection"`), derived from the same cached hint resolution the snap
    /// overlay uses. Present only while a point prompt is active and a
    /// candidate is in tolerance; `null` otherwise (no candidate, no prompt, or
    /// snapping disabled). A `null` is never a swallowed error: a resolution
    /// failure leaves this `null` and surfaces through [`CadStatus::error`].
    pub snap_kind: Option<String>,
    /// Whether the host is currently drawing its **own pointer crosshair** over
    /// the CAD partition: a 2D view with a pointer position on the surface.
    ///
    /// The frontend hides the system cursor over the CAD partition exactly when
    /// this is `true`, so the system arrow never sits on top of the drawn
    /// crosshair — and, just as important, a **3D view keeps the system arrow**
    /// (the crosshair is 2D-only, so blanking it there would leave no pointer at
    /// all). Derived from the same overlay input the renderer draws from, never
    /// from a guess about the view mode.
    pub crosshair: bool,
    /// Count of verified successful authoritative document transitions caused by
    /// an import. It is **not** a parsed-entity count and must not be read as
    /// proof that no reparse happened.
    pub import_count: u64,
    pub frame_count: u64,
    /// Typed command-window echo, oldest first; always present, never `null`.
    ///
    /// Bounded to [`cad_host::COMMAND_LOG_CAPACITY`] (200) with the oldest entry
    /// dropped. Cleared when the authoritative document is replaced (open or
    /// new), so a log from another drawing can never be shown. Observational
    /// only; no entry feeds an input path, and no entry carries human-readable
    /// message text.
    pub command_log: Vec<CadCommandLogEntry>,
    /// The open modal popup (ADR-0021 D1/D2), or `null` while none is open.
    ///
    /// Additive wire field: every other key is unchanged. The host projects its
    /// own live popup state here (see `cad_ui_core::shell::projected_popup`), so
    /// the main UI can reflect/test the real state instead of inferring it.
    pub popup: Option<CadPopup>,
}

/// A `cad_status` snapshot that advertises no CAD capability (probe mode or a
/// failed session): an explicit error, never a fabricated document/count.
///
/// The selection block is present and honest: no session means nothing is
/// selected (`count: 0`), never an omitted key.
pub fn unsupported_status(error: Option<String>) -> CadStatus {
    CadStatus {
        document_label: None,
        entity_count: None,
        loading: false,
        import_progress: None,
        error,
        can_open: false,
        can_fit: false,
        backend: None,
        adapter: None,
        camera: None,
        selection: CadSelection::empty(),
        tool: None,
        hover_preview: false,
        snap_enabled: false,
        snap_kind: None,
        // No CAD session draws no crosshair, so the system cursor is never
        // hidden: nothing is left pointerless.
        crosshair: false,
        import_count: 0,
        frame_count: 0,
        // No host log exists for an unsupported/probe snapshot.
        command_log: Vec::new(),
        // No host builds a popup for an unsupported/probe snapshot; the host
        // projection fills the real value when one is open.
        popup: None,
    }
}

/// Project the real interactive tool state onto the frozen wire shape.
///
/// Idle/selecting/panning map to `None` (no capture is active). A drawing or
/// measurement capture carries its `cad-app` machine key, the **real** captured
/// point count and `cad-app`'s own readiness; nothing is inferred from the
/// pointer stream or fabricated for a tool the state machine does not hold.
///
/// `session` is the authoritative session whose typed parameter store supplies the
/// projected values (POLYGON's `sides`, FILLET's `filletRadius`, CHAMFER's
/// `chamferDistanceA`/`B`, ARRAYRECT's four grid parameters, ARRAYPOLAR's item
/// count / fill angle / rotate flag); each is present **only** while its own tool is
/// active, so a control's enablement is derived from `tool.key` and never from a
/// stale local value. No value is fabricated for an inactive tool.
pub(super) fn cad_tool(
    snapshot: &ToolSnapshot,
    session: &cad_app::SessionState,
) -> Option<CadTool> {
    use cad_app::{DrawToolKind, ToolParameter};

    let number = |parameter: ToolParameter, active: bool| {
        active
            .then(|| session.tool_parameter(parameter).number())
            .flatten()
    };
    let flag = |parameter: ToolParameter, active: bool| {
        active
            .then(|| session.tool_parameter(parameter).flag())
            .flatten()
    };
    match snapshot {
        ToolSnapshot::Drawing(preview) => Some(CadTool {
            key: format!("draw.{}", preview.kind.key()),
            points: preview.points.len(),
            ready: preview.can_confirm(),
            // The real arity model: `true` only for the open-ended kinds (PLINE,
            // SPLINE) while active, so the UI never reports a fixed "remaining"
            // for a capture that still accepts points. Never inferred from the
            // captured count.
            open_ended: preview.is_open_ended(),
            // The real text prompt phase (ADR 0019 decision A.2), read from
            // `cad-app`'s own capture state — never inferred from `key`/`points`.
            awaiting_text: preview.awaiting_text,
            // The TEXTEDIT prefill, straight from the capture state: `None` for
            // every other kind and before the pick resolves.
            text_prefill: preview.text_prefill.clone(),
            params: CadToolParams {
                // The side count is present **only** for a POLYGON capture; every
                // other draw tool reports `null`, and the session already clears
                // the field for a non-POLYGON kind.
                sides: if preview.kind == DrawToolKind::Polygon {
                    preview.polygon_sides
                } else {
                    None
                },
                fillet_radius: number(
                    ToolParameter::FilletRadius,
                    preview.kind == DrawToolKind::Fillet,
                ),
                chamfer_distance_a: number(
                    ToolParameter::ChamferDistanceA,
                    preview.kind == DrawToolKind::Chamfer,
                ),
                chamfer_distance_b: number(
                    ToolParameter::ChamferDistanceB,
                    preview.kind == DrawToolKind::Chamfer,
                ),
                // The four ARRAYRECT parameters are present **only** while an
                // ARRAYRECT capture is active.
                array_rows: number(
                    ToolParameter::ArrayRows,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_columns: number(
                    ToolParameter::ArrayColumns,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_row_spacing: number(
                    ToolParameter::ArrayRowSpacing,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                array_column_spacing: number(
                    ToolParameter::ArrayColumnSpacing,
                    preview.kind == DrawToolKind::ArrayRect,
                ),
                // The three ARRAYPOLAR parameters are present **only** while an
                // ARRAYPOLAR capture is active.
                array_items: number(
                    ToolParameter::ArrayItems,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                array_fill_angle: number(
                    ToolParameter::ArrayFillAngle,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                array_rotate_items: flag(
                    ToolParameter::ArrayRotateItems,
                    preview.kind == DrawToolKind::ArrayPolar,
                ),
                // The ALIGN scale flag is present **only** while an ALIGN capture
                // is active (ADR 0016 decision F); it defaults to false.
                align_scale: flag(
                    ToolParameter::AlignScale,
                    preview.kind == DrawToolKind::Align,
                ),
                // The five HATCH parameters are present **only** while a HATCH
                // capture is active (ADR 0018 decision D, P6b): the tool captures
                // no points, so these are its whole definition.
                hatch_pattern_type: number(
                    ToolParameter::HatchPatternType,
                    preview.kind == DrawToolKind::Hatch,
                ),
                hatch_spacing: number(
                    ToolParameter::HatchSpacing,
                    preview.kind == DrawToolKind::Hatch,
                ),
                hatch_angle: number(
                    ToolParameter::HatchAngle,
                    preview.kind == DrawToolKind::Hatch,
                ),
                hatch_double: flag(
                    ToolParameter::HatchDouble,
                    preview.kind == DrawToolKind::Hatch,
                ),
                hatch_style: number(
                    ToolParameter::HatchStyle,
                    preview.kind == DrawToolKind::Hatch,
                ),
                // The TEXT height is present **only** while a TEXT capture is
                // active (ADR 0019 decision F.1): it is that tool's whole numeric
                // definition.
                text_height: number(
                    ToolParameter::TextHeight,
                    preview.kind == DrawToolKind::Text,
                ),
            },
        }),
        ToolSnapshot::Measuring(preview) => Some(CadTool {
            key: format!("measure.{}", preview.kind.key()),
            points: preview.points.len(),
            ready: preview.can_confirm(),
            // A measurement capture is never open-ended; report the honest
            // `false` rather than a fabricated `true`.
            open_ended: false,
            // A measurement never awaits text.
            awaiting_text: false,
            // …and never carries a text prefill.
            text_prefill: None,
            params: CadToolParams {
                sides: None,
                fillet_radius: None,
                chamfer_distance_a: None,
                chamfer_distance_b: None,
                array_rows: None,
                array_columns: None,
                array_row_spacing: None,
                array_column_spacing: None,
                array_items: None,
                array_fill_angle: None,
                array_rotate_items: None,
                align_scale: None,
                hatch_pattern_type: None,
                hatch_spacing: None,
                hatch_angle: None,
                hatch_double: None,
                hatch_style: None,
                text_height: None,
            },
        }),
        ToolSnapshot::Idle | ToolSnapshot::Selecting | ToolSnapshot::Panning => None,
    }
}

/// Assemble the frozen frontend status from already-derived inputs.
///
/// Pure so the empty-document capability rules are directly testable: without a
/// document there is no label, no entity count, no camera and no `FIT`. The
/// `selection` block is always carried, even when nothing is selected.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble_status(
    has_document: bool,
    label: String,
    entity_count: Option<u64>,
    loading: bool,
    can_open: bool,
    error: Option<String>,
    camera: Option<CadCamera>,
    backend: Option<String>,
    adapter: Option<String>,
    selection: CadSelection,
    import_count: u64,
    frame_count: u64,
    tool: Option<CadTool>,
    hover_preview: bool,
    snap_enabled: bool,
    snap_kind: Option<String>,
    crosshair: bool,
) -> CadStatus {
    CadStatus {
        document_label: has_document.then_some(label),
        entity_count,
        loading,
        import_progress: None,
        error,
        can_open,
        can_fit: has_document && !loading,
        backend,
        adapter,
        camera: if has_document { camera } else { None },
        selection,
        tool,
        hover_preview,
        snap_enabled,
        snap_kind,
        crosshair,
        import_count,
        frame_count,
        // The session owns no command log: `CadSession::snapshot` projects the
        // host's real bounded log onto this field (never inferred here).
        command_log: Vec::new(),
        // The session owns no window: the host projects its live popup state
        // onto the snapshot it returns (never inferred here).
        popup: None,
    }
}
