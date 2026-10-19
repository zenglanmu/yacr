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
/// key** (`id`, `handle`, `type`, …).
///
/// The key is never translated by the host; the UI maps it to a catalog label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadSelectionRow {
    pub key: String,
    pub value: String,
}

/// The `selection` block of [`CadStatus`] (frozen frontend shape).
///
/// The key is **always** present — never omitted and never `null` — and it
/// describes a real selection instead of a fabricated one:
///
/// * `count >= 0` and `empty == (count == 0)`;
/// * `rows` is non-empty **only** for a single selected object. A single ref
///   that no longer resolves still reports its honest `status: "unresolved"`
///   row rather than invented geometry;
/// * `mixedKeys` is non-empty **only** for a multi-object selection.
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
    /// Property rows of a single selection; empty for any other count.
    pub rows: Vec<CadSelectionRow>,
    /// Machine keys whose values differ across a multi-selection; empty for any
    /// other count.
    pub mixed_keys: Vec<String>,
}

impl CadSelection {
    /// The honest empty selection: nothing is selected and nothing is invented.
    pub fn empty() -> Self {
        Self {
            count: 0,
            empty: true,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
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
        let rows = if properties.count == 1 {
            properties
                .rows
                .iter()
                .map(|row| CadSelectionRow {
                    key: row.key.to_string(),
                    value: row.value.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mixed_keys = if properties.count > 1 {
            properties
                .mixed_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            count,
            empty: count == 0,
            rows,
            mixed_keys,
        }
    }
}

/// The typed tool-parameter block of [`CadTool`] (ADR 0015 decision C, extended
/// by ADR 0016 decision A/B/C).
///
/// A frozen frontend shape. Each field is `null` whenever the active tool does not
/// own that parameter (and the whole `tool` is `null` when no tool is active), so a
/// UI control's enablement can be derived from `tool.key` and never from a stale
/// local value. The legacy `sides` field keeps its exact name and meaning; the
/// FILLET/CHAMFER/ARRAY parameters are added alongside it.
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
    /// Typed tool parameters (see [`CadToolParams`]); always present, with `null`
    /// fields for a tool that has no such parameter.
    pub params: CadToolParams,
}

/// The `cad_status` snapshot consumed by the frontend.
///
/// The field set is the frozen frontend contract: every key is always present,
/// an unknown value is `null`, and a count is never fabricated. `error` is a
/// technical, locale-neutral string from the real host state (a command/
/// completion failure, a resource diagnostic or a render diagnostic); it is
/// never a localized UI label. `import_progress` carries the real, live open
/// progress while one is running (see [`CadImportProgress`]) and `selection`
/// the real current selection (see [`CadSelection`]).
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
    /// Count of verified successful authoritative document transitions caused by
    /// an import. It is **not** a parsed-entity count and must not be read as
    /// proof that no reparse happened.
    pub import_count: u64,
    pub frame_count: u64,
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
        import_count: 0,
        frame_count: 0,
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
            },
        }),
        ToolSnapshot::Measuring(preview) => Some(CadTool {
            key: format!("measure.{}", preview.kind.key()),
            points: preview.points.len(),
            ready: preview.can_confirm(),
            // A measurement capture is never open-ended; report the honest
            // `false` rather than a fabricated `true`.
            open_ended: false,
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
        import_count,
        frame_count,
    }
}
