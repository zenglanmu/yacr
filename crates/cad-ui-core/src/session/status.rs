//! The frozen frontend `cad_status` snapshot and its honest projections.
//!
//! Every key is always present, an unknown value is `null`, and a count is
//! never fabricated. The one deliberate exception is [`CadStatus::opened_source`]
//! (ADR-0041 D5): it is an **additive** key that is omitted entirely for any
//! document without a reverse-engineered source, so every other document's wire
//! stays byte-identical. This module carries no host state: it only projects
//! already-derived values.

use serde::Serialize;

use cad_app::ToolSnapshot;

pub use super::layout_tabs::{CadActiveSpace, CadLayoutTab};

use super::command_log::CadCommandLogEntry;
use super::performance::CadPerformance;

pub use super::import_progress::CadImportProgress;
pub use super::open_progress::{CadOpenProgress, CadOpenSummary};
pub use super::reference_status::CadReferenceStatus;

/// Camera block of the frontend status (`{center, scale}`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CadCamera {
    /// World point at the centre of the viewport.
    pub center: [f64; 2],
    /// World units per logical pixel (strictly positive).
    pub scale: f64,
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
    /// The source's own name for a dynamic row (`Pset_WallCommon.IsExternal`),
    /// or `null` for a static machine key.
    ///
    /// Source data, never catalog copy: the UI shows it verbatim when present and
    /// falls back to the key's catalog label otherwise. It is **omitted** for a
    /// static key (like `openedSource`), so a selection without a dynamic row
    /// keeps its previous wire shape byte for byte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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
                label: row.label.clone(),
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

mod classic;
mod tool;
mod tool_params;
mod units;
pub use self::classic::{
    CadBlockStatus, CadGroupMember, CadGroupStatus, CadImageClipStatus, CadImageStatus,
};
pub(in crate::session) use self::tool::cad_tool;
pub use self::tool_params::CadToolParams;
pub use self::units::CadUnits;

/// The active interactive tool, projected from `cad_app`'s real capture state.
///
/// Present only while a draw or measurement capture is active: an idle,
/// selecting or panning session carries `tool: null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadTool {
    /// Stable machine key: `"draw.line"`..`"draw.offset"`, `"measure.distance"`,
    /// `"measure.polyline"`, `"measure.angle"`, `"measure.area"`, `"measure.id"`.
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
    /// contract reads (`apps/desktop-ui/src/text.mjs` / `cad.mjs` validate and
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
/// Wire shape: `{ kind, label, modal }`. It is the host's real popup state, not
/// a UI guess: `kind` is the allowlisted popup kind key, `label` the frozen
/// legacy wire label, and `modal` whether the host holds its input gate for it.
/// The block is `null` while no popup exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadPopup {
    /// Stable popup kind key (`"about"`, `"textEdit"` or `"options"`).
    pub kind: String,
    /// Frozen legacy wire label (`"popup-1"`).
    ///
    /// The popup is an in-DOM overlay now, so there is no window to address. The
    /// field stays in the frozen `cad_status` shape and is **never** written from
    /// a second source of truth (no dual-write): every producer uses
    /// [`crate::popup::POPUP_LABEL`].
    #[deprecated(
        note = "the popup is an in-DOM overlay; this label is frozen wire state and addresses no window"
    )]
    pub label: String,
    /// Whether the host holds the modal input gate for this popup.
    pub modal: bool,
}

/// Which specification a connector's format was implemented from (ADR-0041
/// D5), as the wire names it.
///
/// Mirrors `cad_connector_core::SpecVisibility` (which deliberately carries no
/// serde) so the `openedSource.spec` value is `"public"` /
/// `"reverse_engineered"` and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CadSpecVisibility {
    /// Published, freely implementable specification.
    Public,
    /// Not published; the reader is an independent reconstruction.
    ReverseEngineered,
}

/// Provenance of the connector that opened the current document (ADR-0041 D5).
///
/// Wire shape: `{ connector, spec, canWrite, experimental }`. It exists **only**
/// when the opened document's source connector has a
/// [`CadSpecVisibility::ReverseEngineered`] spec: a public-spec format produces
/// no object at all, so the key is absent (see
/// [`CadStatus::opened_source`]) and nothing about a DXF/STEP/IFC/glTF/mesh
/// document changes on the wire. Every value is the registry descriptor's own —
/// never an extension sniff or a UI guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSource {
    /// Stable registry connector id (`"dwg"`, `"dgn"`, `"skp"`, `"rvt"`).
    pub connector: String,
    /// The format's spec visibility; always
    /// [`CadSpecVisibility::ReverseEngineered`] on the wire today.
    pub spec: CadSpecVisibility,
    /// Whether the connector declares an export direction
    /// (`descriptor.export.is_some()`).
    pub can_write: bool,
    /// Whether that writer is unverified/experimental; `false` without a writer.
    pub experimental: bool,
}

impl OpenedSource {
    /// Project the application's connector provenance onto the wire shape.
    pub(super) fn from_app(source: cad_app::OpenedSource) -> Self {
        Self {
            connector: source.connector.to_string(),
            spec: if source.spec_is_public() {
                CadSpecVisibility::Public
            } else {
                CadSpecVisibility::ReverseEngineered
            },
            can_write: source.can_write,
            experimental: source.experimental,
        }
    }
}

pub use super::status_view::{CadNavigation, CadViewOrientation};

/// The `cad_status` snapshot consumed by the frontend.
///
/// The field set is the frozen frontend contract: every key is always present,
/// an unknown value is `null`, and a count is never fabricated. The **one**
/// exception is the additive [`Self::opened_source`] key (ADR-0041 D5), which is
/// serialized only for a reverse-engineered source so every other document's
/// wire is unchanged. `error` is a technical, locale-neutral string from the
/// real host state (a command/completion failure, a resource diagnostic or a
/// render diagnostic); it is never a localized UI label. `import_progress`
/// carries the real, live open progress while one is running (see
/// [`CadImportProgress`]), `selection` the real current selection (see
/// [`CadSelection`]) and `popup` the host's open modal popup (see
/// [`CadPopup`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadStatus {
    pub document_label: Option<String>,
    pub entity_count: Option<u64>,
    pub loading: bool,
    /// Live import progress, or `null` when no import is running.
    pub import_progress: Option<CadImportProgress>,
    /// Live open-pipeline progress (ADR-0030 D9), or `null` when no open is
    /// running.
    ///
    /// The carrier overlays its live stage (import → vendor → representations)
    /// onto this field; [`super::CadSession::snapshot`] leaves it `None` and never
    /// fabricates a stage. Once ready it is `null` again and
    /// [`Self::open_summary`] keeps the terminal outcome.
    pub open_progress: Option<CadOpenProgress>,
    /// Whether the **current authoritative document** finished its open pipeline
    /// (vendor settled, derived snapshot installed).
    ///
    /// Distinct from [`Self::loading`] (picker/import only): a vendor pass keeps
    /// this `false` while NEW/OPEN stay available. It is `false` without a
    /// document, and a failed/cancelled OPEN never flips it for the document that
    /// was already open.
    pub document_ready: bool,
    /// Terminal summary of the last successful open (label, entities, vendor
    /// outcome and bounded diagnostics), or `null`.
    ///
    /// Retained after readiness so a late-joining consumer reads it from the
    /// status instead of relying on event replay ([`Self::document_ready`] is the
    /// gate, this is the explanation).
    pub open_summary: Option<CadOpenSummary>,
    /// Provenance of the connector that opened the current document (ADR-0041
    /// D5), or **absent**.
    ///
    /// Present only when the document's source connector has a
    /// reverse-engineered spec (`dwg`, `dgn`, `skp`, `rvt`); `canWrite` reports
    /// whether it also declares an export direction (see [`OpenedSource`]).
    /// Absent for every other document: no document, a native `.ymodel`, a
    /// public-spec connector (DXF/STEP/IFC/glTF/STL/OBJ/PLY/OFF), and after
    /// New/Close. The `skip_serializing_if` keeps the wire byte-identical for
    /// those documents, so this is a purely additive key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_source: Option<OpenedSource>,
    /// The application domain the chrome renders for the current document
    /// (ADR-0053 D2), or `null` without a document.
    ///
    /// Host truth: the document's named value, or the conservative derivation
    /// (BIM data → `bim`, everything else → `cad2d`). It is an interface token,
    /// not a claim about the import format; `openedSource` stays the provenance
    /// key.
    pub application_domain: Option<String>,
    pub error: Option<String>,
    /// Whether the host can start a **blank** document right now: the host
    /// advertises the NEW capability and no open/import is in flight
    /// ([`cad_host::HostCore::loading`]).
    ///
    /// A new drawing needs no source file, so this is deliberately independent
    /// of [`Self::document_label`]: `true` in an empty session, unlike
    /// [`Self::can_save`]/[`Self::can_plot`], which need an authoritative
    /// document. It is a capability+state projection, never a guess.
    pub can_new: bool,
    pub can_open: bool,
    /// Whether the host can save the **current** document right now: the host
    /// advertises the SAVE capability, an authoritative document exists, no
    /// open/import is in flight and no save/plot target chooser is pending
    /// ([`cad_host::HostCore::save_pending`]).
    ///
    /// A capability+state projection: a host without a save dialog reports
    /// `false` even with a document open, and the flag drops while a save or
    /// plot gesture is still awaiting its target, so the command is never
    /// advertised as available twice concurrently.
    pub can_save: bool,
    /// Whether the host can plot the **current** document right now: the host
    /// advertises the PLOT capability with the same document/loading/pending
    /// rules as [`Self::can_save`] (the host serialises both through one
    /// pending-save slot).
    ///
    /// A capability+state projection, never derived from the file extension or
    /// from a UI guess about the format.
    pub can_plot: bool,
    pub can_fit: bool,
    pub backend: Option<String>,
    pub adapter: Option<String>,
    pub camera: Option<CadCamera>,
    /// The current selection; always present, never `null`.
    pub selection: CadSelection,
    /// One tab per **paper-space** layout, in the authoritative descriptor
    /// order; always present, never `null`. The synthetic `LayoutId(0)` "Model"
    /// descriptor is excluded because model space is its own tab.
    pub layouts: Vec<CadLayoutTab>,
    /// The session's real active drawing space (model or one paper layout).
    pub active_space: CadActiveSpace,
    pub pending_prompt: Option<super::api_prompt::CadPendingPrompt>,
    /// One row per authoritative external reference (XREF), in the drawing's
    /// stable reference-name (`BTreeMap`) order; always present, never `null`.
    /// Empty when there is no document or the drawing declares no xrefs. A
    /// reference that could not be loaded stays visible with its real status
    /// and reason instead of being dropped.
    pub references: Vec<CadReferenceStatus>,
    /// One row per authoritative block definition; always present, never `null`.
    pub blocks: Vec<CadBlockStatus>,
    /// One row per **session-only** entity group; always present, never `null`.
    pub groups: Vec<CadGroupStatus>,
    /// One row per raster `IMAGE` entity in any space; always present, never `null`.
    pub images: Vec<CadImageStatus>,
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
    /// Whether the display grid overlay is enabled for this session (`GRID`,
    /// F7).
    ///
    /// Session/display state only; defaults to `true` and can be flipped with the
    /// `GRID` selector (F7). The grid itself is drawn only when this switch and
    /// the persisted `view.overlays.grid` preference are both on, so this flag
    /// alone does not mean the grid is visible. This is the **display** grid
    /// only; grid snap is [`Self::snap_grid_enabled`].
    pub grid_enabled: bool,
    /// Whether **grid snap** is enabled for this session (`SNAP`, F9).
    ///
    /// Session/display state only; defaults to `false` (AutoCAD's initial `SNAP`
    /// off) and can be flipped with the `SNAP` selector (F9). While on, a captured
    /// point is quantised to the nearest multiple of [`Self::snap_spacing`] in X
    /// and Y (never Z), before the `ORTHO` constraint. It never changes the drawing
    /// database, history or a persisted preference, and it is not object snap.
    pub snap_grid_enabled: bool,
    /// The session grid-snap spacing in drawing units (`SNAPUNIT`).
    ///
    /// Always a finite, strictly positive number: the session setter refuses
    /// anything else explicitly, so this field never carries `0`/`NaN`. The
    /// default is `cad_app::DEFAULT_SNAP_SPACING` (10.0 drawing units — this
    /// build's declared choice, since AutoCAD's initial value comes from the
    /// template). Session sysvar state only, set with `SNAPUNIT <value>`.
    pub snap_spacing: f64,
    /// Whether the `ORTHO` drafting constraint is enabled for this session
    /// (`ORTHO`, F8).
    ///
    /// Session/display state only; defaults to `false` (AutoCAD's initial `ORTHO`
    /// off) and can be flipped with the `ORTHO` selector (F8). While on, the next
    /// point entry of a drawing or measurement capture is constrained to a
    /// horizontal or vertical direction from the tool's previous point. It never
    /// changes the drawing database, history or a persisted preference, and it is
    /// **not** grid snap (`SNAP`/F9 is the separate [`Self::snap_grid_enabled`]).
    pub ortho_enabled: bool,
    /// Whether the session text screen is open (`TEXTSCR` / `GRAPHSCR`).
    ///
    /// Session/display state only, projected from the session's own
    /// `drafting_extras` token. `true` means a carrier whose chrome hosts the
    /// text-screen surface shows this session's command history over the
    /// drawing; `false` (the default) means the drawing is shown. It never
    /// changes the document, history or a persisted preference.
    #[serde(rename = "textScreen")]
    pub text_screen: bool,
    /// Whether the status-bar tray is shown (`TRAYSETTINGS`, default `true`).
    ///
    /// Session/display state only, projected from the session's own
    /// `drafting_extras` token. A carrier's chrome hides its status-bar tray
    /// (the icon/notification cluster) while this is `false`, and the Tray
    /// Settings surface toggles the very same flag. It never changes the
    /// document, history or a persisted preference.
    #[serde(rename = "traySettings")]
    pub tray_settings: bool,
    /// Whether the session's PBR environment lighting is **image-based** (`IBL`).
    ///
    /// Session/display state only; the procedural-IBL render lane reads the same
    /// token (`cad_app::PbrEnvironment::is_image_based`). Defaults to `true`
    /// (image-based lighting is the shader's normal path); `IBL OFF` (or the
    /// `LEGACY`/`0` spelling) reports `false` and falls back to the single
    /// dominant light. It never changes the drawing, history or a preference.
    #[serde(rename = "iblEnabled")]
    pub ibl_enabled: bool,
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
    /// all). Derived from the host's single cursor source
    /// (`HostCore::cursor_world`), the same derivation fed to the renderer
    /// transient, never from a guess about the view mode.
    pub crosshair: bool,
    /// The cursor's position in **world (drawing) coordinates**, or `null`.
    ///
    /// Wire shape: `cursorWorld: [x, y, z] | null`, always present. It is the
    /// very same [`cad_app::Point3`] the renderer draws the crosshair from —
    /// `HostCore::cursor_world`, the single host cursor source that is also fed to
    /// `RenderSessionHandle::set_cursor_transient` and is only non-`None` for a
    /// valid 2D view with a pointer on the surface — never a re-derived pointer
    /// guess, so the readout can never disagree with the drawn crosshair.
    ///
    /// `null` means "no reliable world coordinate" (a 3D view, a pointer outside
    /// the surface, or no session at all). It is **not** the origin: a session
    /// without a cursor must render an em dash, never `0, 0, 0`.
    pub cursor_world: Option<[f64; 3]>,
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
    /// Whether a **grip drag** is armed right now
    /// ([`cad_host::HostCore::grip_drag_active`]).
    ///
    /// Additive wire field (never `null`, always a real boolean): the host is the
    /// only authority for the gesture, and the chrome needs it because the drag
    /// writes nothing until the release — Escape must reach the host as `CANCEL`
    /// to drop it. Without this flag the UI's Escape gate (which only knew about
    /// an active **tool**) left the drag un-cancellable from the keyboard.
    ///
    /// It is **not** an active tool: [`Self::tool`] stays `null` during a drag,
    /// so every tool-only affordance (Enter/`CONFIRM`, the tool prompt) keeps
    /// behaving exactly as before. `dom-canvas` carriers never arm a grip drag
    /// and therefore always report `false` (ADR 0024: no grip path there).
    pub grip_drag: bool,
    /// The host's performance metrics (see [`CadPerformance`]), or `null`.
    ///
    /// Additive optional block: every other key is unchanged. `None` means the
    /// host published no metrics — an older host, or a carrier without a
    /// sampler — and is **never** a fabricated zero. The consumer renders an em
    /// dash for each absent metric instead of inventing a measurement.
    pub performance: Option<CadPerformance>,
    /// The host's UI theme (`"dark"` | `"light"`), always present.
    ///
    /// This is the persisted host preference
    /// ([`cad_host::CadSettingKey::Theme`]) projected verbatim, never a UI-side
    /// default: applying a theme also sets the model/paper canvas backgrounds,
    /// so the chrome and the canvas cannot disagree.
    pub theme: String,
    /// The current view orientation (see [`CadViewOrientation`]), or `null`.
    ///
    /// Host truth from the authoritative viewport camera. `null` means "no
    /// authoritative camera or a non-finite basis" (no document, or a camera the
    /// host would not render) — never a fabricated orientation.
    pub view: Option<CadViewOrientation>,
    /// The document's stored `UNITS` readout settings, or **absent**.
    ///
    /// Additive optional block: no document and older hosts omit it, and the
    /// readouts then keep their legacy fixed format. When present it is the
    /// document header's own values (`length_format`/`length_precision`,
    /// angle format/precision/base/direction), never a UI-side default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<CadUnits>,
    /// The host's real navigation/ViewCube switches (see [`CadNavigation`]);
    /// always present, never `null`.
    pub navigation: CadNavigation,
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
        // No session runs no open pipeline: no progress, nothing ready and no
        // terminal summary are invented.
        open_progress: None,
        document_ready: false,
        open_summary: None,
        // No session means no opened connector: absent, never a fabricated object.
        opened_source: None,
        // No session means no document, hence no interface domain.
        application_domain: None,
        error,
        // No session means no document/command capability at all.
        can_new: false,
        can_open: false,
        can_save: false,
        can_plot: false,
        can_fit: false,
        backend: None,
        adapter: None,
        camera: None,
        selection: CadSelection::empty(),
        // No authoritative document means no paper layouts at all, and the only
        // honest space is model space (never an invented "current" tab).
        layouts: Vec::new(),
        active_space: CadActiveSpace::Model,
        pending_prompt: None,
        // A session without a document declares no xrefs: the honest empty
        // block, never an omitted key.
        references: Vec::new(),
        blocks: Vec::new(),
        groups: Vec::new(),
        images: Vec::new(),
        tool: None,
        hover_preview: false,
        snap_enabled: false,
        grid_enabled: false,
        // No session means no installed grid-snap switch to advertise…
        snap_grid_enabled: false,
        // …but the spacing field must still be a real, usable number for the
        // frozen wire shape (the frontend requires a finite value > 0). The
        // documented session default is reported instead of a meaningless 0/NaN;
        // `snap_grid_enabled: false` keeps it clear that nothing is snapping.
        snap_spacing: cad_app::DEFAULT_SNAP_SPACING,
        ortho_enabled: false,
        // No session means no text screen and no status-bar tray to hide: the
        // documented session defaults (text screen closed, tray shown) are
        // reported rather than a fabricated open window.
        text_screen: false,
        tray_settings: true,
        // No session means no render switch is installed, so it cannot be
        // reported as image-based. The session default (on) is `assemble_status`'s
        // business, not the probe snapshot's.
        ibl_enabled: false,
        snap_kind: None,
        // No CAD session draws no crosshair, so the system cursor is never
        // hidden: nothing is left pointerless.
        crosshair: false,
        // …and no session has a cursor, so there is no world coordinate to show.
        cursor_world: None,
        import_count: 0,
        frame_count: 0,
        // No host log exists for an unsupported/probe snapshot.
        command_log: Vec::new(),
        // No host builds a popup for an unsupported/probe snapshot; the host
        // projection fills the real value when one is open.
        popup: None,
        // No session means no pointer and therefore no grip drag to cancel.
        grip_drag: false,
        // No host sampler produced metrics for a probe/unsupported snapshot:
        // explicit `None`, never an invented zero.
        performance: None,
        // No host preference was read for a probe/unsupported snapshot: the
        // documented theme default is reported rather than an empty string.
        theme: cad_host::THEME_VALUES[0].to_string(),
        // …and no session camera exists, so there is no orientation to report.
        view: None,
        units: None,
        navigation: CadNavigation::default(),
    }
}

/// The document-dependent capability projection of [`CadStatus`]
/// (`canNew`/`canSave`/`canPlot`).
///
/// Split from [`assemble_status`] so the rules are unit-testable without a
/// live host: they are a pure function of the advertised host capabilities and
/// the session's real open/loading/save-pending state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DocumentCapabilities {
    /// See [`CadStatus::can_new`].
    pub can_new: bool,
    /// See [`CadStatus::can_save`].
    pub can_save: bool,
    /// See [`CadStatus::can_plot`].
    pub can_plot: bool,
}

/// Project the host capability flags × the live session state onto the three
/// document-dependent capability booleans.
///
/// The rules are the honest minimum, each clause backed by real host state:
///
/// * NEW needs the capability and an **idle** session (no open/import running);
///   it deliberately does **not** need a document — a blank drawing needs no
///   source file — and it stays available while the open pipeline is not ready,
///   so the user can cancel or switch files (ADR-0030 D9).
/// * SAVE/PLOT need the capability, an authoritative document, an **idle**
///   session (no open/import running and no save/plot target chooser or export
///   still pending) **and** a ready document: `document_ready` is `false` while
///   the vendor pass of a newly opened document is still applying, and writing
///   must not race it (ADR-0030 D9). The host serialises save and plot through
///   the same pending slot, so neither is advertised while the other is in
///   flight.
pub(super) fn document_capabilities(
    capabilities: cad_platform::HostCapabilities,
    has_document: bool,
    loading: bool,
    save_pending: bool,
    document_ready: bool,
) -> DocumentCapabilities {
    let document_idle = has_document && !loading && !save_pending;
    DocumentCapabilities {
        can_new: capabilities.new && !loading,
        can_save: capabilities.save && document_idle && document_ready,
        can_plot: capabilities.plot && document_idle && document_ready,
    }
}

/// Assemble the frozen frontend status from already-derived inputs.
///
/// Pure so the empty-document capability rules are directly testable: without a
/// document there is no label, no entity count, no camera and no `FIT`. The
/// `selection` block is always carried, even when nothing is selected.
///
/// `capabilities` is the already-projected document capability block (see
/// [`document_capabilities`]); it is carried verbatim, never re-derived here.
/// `grip_drag` is likewise the host's own [`cad_host::HostCore::grip_drag_active`]
/// answer, passed in by the session that owns the host.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble_status(
    has_document: bool,
    label: String,
    entity_count: Option<u64>,
    loading: bool,
    can_open: bool,
    capabilities: DocumentCapabilities,
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
    grid_enabled: bool,
    snap_grid_enabled: bool,
    snap_spacing: f64,
    ortho_enabled: bool,
    ibl_enabled: bool,
    snap_kind: Option<String>,
    crosshair: bool,
    cursor_world: Option<[f64; 3]>,
    grip_drag: bool,
    layouts: Vec<CadLayoutTab>,
    active_space: CadActiveSpace,
    theme: String,
    view: Option<CadViewOrientation>,
    navigation: CadNavigation,
    pending_prompt: Option<super::api_prompt::CadPendingPrompt>,
    document_ready: bool,
    open_summary: Option<CadOpenSummary>,
) -> CadStatus {
    CadStatus {
        document_label: has_document.then_some(label),
        entity_count,
        loading,
        import_progress: None,
        // The carrier overlays the live open stage; the assembler never invents
        // one, exactly like `import_progress`.
        open_progress: None,
        document_ready,
        open_summary,
        // The session owns the document: `CadSession::snapshot` projects the
        // opened source's provenance and the interface domain onto these fields
        // (never inferred here).
        opened_source: None,
        application_domain: None,
        error,
        can_new: capabilities.can_new,
        can_open,
        can_save: capabilities.can_save,
        can_plot: capabilities.can_plot,
        can_fit: has_document && !loading,
        backend,
        adapter,
        camera: if has_document { camera } else { None },
        selection,
        layouts,
        active_space,
        // The session owns the drawing: `CadSession::snapshot` projects the
        // authoritative references onto this field (never inferred here).
        references: Vec::new(),
        blocks: Vec::new(),
        groups: Vec::new(),
        images: Vec::new(),
        tool,
        hover_preview,
        snap_enabled,
        grid_enabled,
        snap_grid_enabled,
        snap_spacing,
        ortho_enabled,
        // The session owns the two chrome-facing session tokens: `CadSession`
        // projects their real values onto the snapshot it returns. These are the
        // documented session defaults for a status assembled without them.
        text_screen: false,
        tray_settings: true,
        ibl_enabled,
        snap_kind,
        crosshair,
        cursor_world,
        import_count,
        frame_count,
        // The session owns no command log: `CadSession::snapshot` projects the
        // host's real bounded log onto this field (never inferred here).
        command_log: Vec::new(),
        // The session owns no window: the host projects its live popup state
        // onto the snapshot it returns (never inferred here).
        popup: None,
        // Carried verbatim from the host's own gesture state; this assembler
        // never derives a drag from the tool/pointer fields.
        grip_drag,
        // The session owns no sampler: the host projects its live metrics onto
        // this field on the snapshot it returns (never inferred here).
        performance: None,
        // The host's persisted theme and navigation switches, carried verbatim.
        theme,
        // The authoritative viewport orientation, or `null` when there is no
        // trustworthy camera basis.
        view,
        units: None,
        navigation,
        pending_prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(rows: Vec<cad_app::PropertyRow>) -> cad_app::SelectionProperties {
        cad_app::SelectionProperties {
            count: 1,
            rows,
            mixed_keys: Vec::new(),
            text: None,
            text_multiline: false,
            empty: false,
        }
    }

    #[test]
    fn a_dynamic_row_carries_its_source_label_and_a_static_row_omits_the_key() {
        let mut dynamic = cad_app::PropertyRow::new("bim.pset.0.0", "3.5 m");
        dynamic.label = Some("Pset_WallCommon.Width".to_string());
        let selection = CadSelection::from_properties(&single(vec![
            cad_app::PropertyRow::new("id", "1"),
            dynamic,
        ]));

        let value = serde_json::to_value(&selection).expect("the selection serializes");
        let rows = value["rows"].as_array().expect("rows are an array");
        assert!(
            rows[0].get("label").is_none(),
            "a static key omits `label`, keeping the previous wire shape"
        );
        assert_eq!(rows[1]["label"], "Pset_WallCommon.Width");
        assert_eq!(rows[1]["key"], "bim.pset.0.0");
    }
}
