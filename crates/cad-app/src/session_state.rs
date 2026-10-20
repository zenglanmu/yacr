//! Per-document CAD session state and its recorded backend preference.
//!
//! Split out of the crate root to keep `lib.rs` inside the code-size limit.
//! Every item here is re-exported at the crate root under its original name, so
//! `cad_app::SessionState`, `cad_app::BackendChoice` and
//! `cad_app::ClipboardEntry` are unchanged for every caller.

use cad_db::MeasurementRecord;
use cad_domain::*;

use crate::draw_tool::{
    DrawIntent, DrawPreview, DrawTool, DrawToolKind, ToolParameter, ToolParameterStore, ToolValue,
    DEFAULT_POLYGON_SIDES,
};
use crate::layers::LayerOverrideSet;
use crate::measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
use crate::selection::SelectionSet;
use crate::tool_state::{DrawCapture, ToolSnapshot, ToolState};
use crate::{AppMode, CommandId};

/// One entity captured into the in-process clipboard.
///
/// The [`DbEntity`] carries the row; [`cad_db::EntityDisplayState`] carries the
/// per-entity render attributes and dynamic-visibility membership the row does
/// **not**, so a paste preserves exactly what a cut's undo would restore.
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    pub entity: cad_db::DbEntity,
    pub display: cad_db::EntityDisplayState,
}

pub struct SessionState {
    pub document: DocumentId,
    mode: AppMode,
    pub active_space: SpaceId,
    pub selection: SelectionSet,
    /// Temporary layer visibility. Never the drawing's layer table (F03).
    pub layer_overrides: LayerOverrideSet,
    pub tool: ToolState,
    /// Generation incremented whenever an active tool lifecycle is replaced.
    pub tool_generation: u64,
    pub generation: u64,
    /// Recorded CAD backend preference; the host rebuilds the render session.
    pub backend: BackendChoice,
    /// The session's active layer for new drawing entities (spec F-EDIT).
    ///
    /// Purely session state: it selects where [`CommandId::CreateLine`] /
    /// [`CommandId::CreateCircle`] place a new entity and never mutates the
    /// drawing's layer table. The default is layer `0`, which every drawing is
    /// required to have; a create naming a layer the database does not contain
    /// is an explicit `InvalidInput` rather than a silent fallback.
    pub active_layer: LayerId,
    /// The session-scoped typed tool parameters (ADR 0016 decision A).
    ///
    /// Session/display state only — **never** a persisted config. The general
    /// channel replaces ADR 0015 decision C's narrow `polygon_sides` sysvar: every
    /// parameter starts at its documented default ([`ToolParameter::default_value`],
    /// e.g. `PolygonSides` = 4, AutoCAD's `POLYSIDES` initial value) and is set
    /// only through the typed `SetToolParameter` request (per-parameter range/kind
    /// validation, out of range is an explicit refusal, never a silent clamp). The
    /// values survive a tool switch and a cancel (sysvar semantics), are **not**
    /// reset by a document edit, and return to the defaults only when the session
    /// is rebuilt (a new blank document or a document replacement).
    tool_parameters: ToolParameterStore,
    /// Whether object snapping is active for this session (ADR 0014).
    ///
    /// Session state only: toggling it writes no database and creates no
    /// history entry. The default is `true`; it is deliberately independent of
    /// the persisted `view.overlays.snapHints` preference, which only controls
    /// whether resolved hints are drawn.
    snap_enabled: bool,
    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// Session/display state only: toggling it writes no database, produces no
    /// history entry and schedules no ChangeSet. The default is **`true`**, which
    /// matches AutoCAD's `SELECTIONPREVIEW` initial value `3` (on while no
    /// command is active and at "Select objects" prompts). It can be turned off
    /// with the `HOVERPREVIEW` selector. AutoCAD has no hover timer, so the
    /// dwell (`hover_dwell_ms`) remains an explicit deviation, not parity. The
    /// preview never affects object-snap markers, which stay gated on an active
    /// point prompt.
    hover_preview: bool,
    /// How long a resolved hover must stay stable before it is tinted, in
    /// milliseconds (default **20**).
    ///
    /// **Explicit deviation from AutoCAD**, not a parity claim: AutoCAD defines
    /// no dwell for the hover highlight (the highlight is immediate and only
    /// distance/pickbox-gated — verified against the sysvar set). The product
    /// owner asked for a deferred highlight so a fast sweep paints nothing; the
    /// owner then lowered the default from 300 ms to 20 ms, which at a 60 Hz
    /// frame cadence is roughly one frame: it filters within-frame jitter and
    /// small sweeps, and no longer suppresses a deliberate fast sweep.
    /// `0` restores the immediate highlight (the documented escape hatch). Read
    /// only when the preview is enabled; object-snap markers are unaffected.
    /// Session/display state only.
    hover_dwell_ms: u64,
    /// The last structured measurement result produced in this session (F06).
    ///
    /// Set when a measurement is evaluated (auto-completed or explicitly
    /// confirmed). Cleared when the session is rebuilt for new content.
    last_measurement: Option<MeasurementRecord>,
    /// Last measurement choice retained even while no measurement is active.
    selected_measurement_kind: MeasurementToolKind,
    /// In-process entity clipboard (Phase 8).
    ///
    /// Holds clones of the selected entities (with their display state); it is
    /// **never** the OS/system clipboard. It is cleared whenever the document
    /// content is replaced (`HostController::reset_for_new_content` rebuilds the
    /// session), so a paste can never cross documents. Ctrl+C/X capture here;
    /// Ctrl+V inserts fresh-id clones at their original coordinates (PASTEORIG
    /// semantics).
    pub(crate) clipboard: Vec<ClipboardEntry>,
}

/// CAD backend preference shared by UI, app state and hosts (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Auto,
    WebGpu,
    WebGl2,
}

impl SessionState {
    pub fn new(document: DocumentId, mode: AppMode) -> Self {
        SessionState {
            document,
            mode,
            active_space: SpaceId::Model,
            selection: SelectionSet::new(),
            layer_overrides: LayerOverrideSet::new(),
            tool: ToolState::Idle,
            tool_generation: 0,
            generation: 0,
            backend: BackendChoice::Auto,
            active_layer: LayerId(0),
            tool_parameters: ToolParameterStore::defaults(),
            snap_enabled: true,
            hover_preview: true,
            hover_dwell_ms: 20,
            last_measurement: None,
            selected_measurement_kind: MeasurementToolKind::Distance,
            clipboard: Vec::new(),
        }
    }

    pub fn mode(&self) -> AppMode {
        self.mode
    }

    /// Whether object snapping is active in this session.
    pub fn snap_enabled(&self) -> bool {
        self.snap_enabled
    }

    /// Enable or disable object snapping for this session.
    ///
    /// Session state only: no database write, no history entry. When disabled,
    /// [`crate::host::HostController::resolve_snap`] returns `Ok(None)` (the raw
    /// point is used) and the snap-hint overlay is empty.
    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.snap_enabled = enabled;
    }

    /// Flip object snapping and return the new state (F3 binding).
    pub fn toggle_snap(&mut self) -> bool {
        self.snap_enabled = !self.snap_enabled;
        self.snap_enabled
    }

    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// See [`SessionState::set_hover_preview`] for the AutoCAD-matching default.
    pub fn hover_preview(&self) -> bool {
        self.hover_preview
    }

    /// The current value of one session tool parameter (ADR 0016 decision A).
    ///
    /// Session/display state only; the store is initialised to each parameter's
    /// documented default.
    pub fn tool_parameter(&self, parameter: ToolParameter) -> ToolValue {
        self.tool_parameters.get(parameter)
    }

    /// The session POLYGON side count (ADR 0015 decision C, now carried by the
    /// general channel as [`ToolParameter::PolygonSides`]).
    ///
    /// Session/display state only; the default is [`DEFAULT_POLYGON_SIDES`] (4,
    /// AutoCAD's `POLYSIDES` initial value).
    pub fn polygon_sides(&self) -> u32 {
        match self.tool_parameter(ToolParameter::PolygonSides) {
            ToolValue::Number(sides) => sides as u32,
            // The store validates on write, so `PolygonSides` is always a number;
            // the flag arm is unreachable and falls back to the documented default.
            ToolValue::Flag(_) => DEFAULT_POLYGON_SIDES,
        }
    }

    /// Set one session tool parameter through the single general channel.
    ///
    /// Authorizes the parameter's own create/edit command (all Work-only, so a
    /// Viewer-mode set is an explicit `PermissionDenied`), then validates the
    /// value against the parameter's kind and range with
    /// [`ToolParameter::validate`] — an out-of-range or wrong-kind value is an
    /// explicit `InvalidInput`, never a silent clamp, and never mutates the store.
    /// The value is session state: a tool switch, a cancel and a document edit
    /// leave it untouched; only a session rebuild resets it to the default.
    pub fn set_tool_parameter(
        &mut self,
        parameter: ToolParameter,
        value: ToolValue,
    ) -> CadResult<()> {
        self.authorize(authorizing_command(parameter))?;
        parameter.validate(value)?;
        self.tool_parameters.set(parameter, value);
        Ok(())
    }

    /// Set the session POLYGON side count, refusing out-of-range input.
    ///
    /// A thin convenience over [`SessionState::set_tool_parameter`] — the general
    /// store is the single authority for validation and storage (this is **not** a
    /// second entrance: the wire request, the IPC and the session channel are all
    /// the general one). The range is
    /// [`crate::draw_tool::MIN_POLYGON_SIDES`]`..=`[`crate::draw_tool::MAX_POLYGON_SIDES`]
    /// (AutoCAD's `POLYSIDES` legal range); an out-of-range value is an explicit
    /// `InvalidInput`, never a silent clamp. Work-only: the create commands (and
    /// therefore this parameter) are refused in Viewer mode.
    pub fn set_polygon_sides(&mut self, sides: u32) -> CadResult<()> {
        self.set_tool_parameter(
            ToolParameter::PolygonSides,
            ToolValue::Number(f64::from(sides)),
        )
    }

    /// Enable or disable the idle hover entity preview for this session.
    ///
    /// Session/display state only: no database write, no history entry and no
    /// ChangeSet. The default is **`true`**, matching AutoCAD's
    /// `SELECTIONPREVIEW` initial value `3`; `HOVERPREVIEW` turns it off. The
    /// dwell remains a documented deviation (AutoCAD has no hover timer).
    /// Object-snap markers are unaffected: they remain gated on an active point
    /// prompt.
    pub fn set_hover_preview(&mut self, enabled: bool) {
        self.hover_preview = enabled;
    }

    /// Flip the idle hover entity preview and return the new state.
    pub fn toggle_hover_preview(&mut self) -> bool {
        self.hover_preview = !self.hover_preview;
        self.hover_preview
    }

    /// How long a resolved hover must stay stable before it is tinted, in
    /// milliseconds.
    ///
    /// **A deliberate deviation from AutoCAD**, which has no hover timer; see
    /// [`SessionState::set_hover_dwell_ms`]. It is a documented extension, never
    /// presented as AutoCAD parity.
    pub fn hover_dwell_ms(&self) -> u64 {
        self.hover_dwell_ms
    }

    /// Set the hover dwell in milliseconds (`0` = immediate highlight).
    ///
    /// Session/display state only: no database write, no history entry, no
    /// ChangeSet. The default is **20 ms** (lowered from 300 ms by the product
    /// owner; at 60 Hz that is roughly one frame), an explicit product extension —
    /// AutoCAD defines no dwell for `SELECTIONPREVIEW` (the highlight is
    /// immediate, distance-gated only). The dwell is honoured only while the
    /// preview is enabled.
    pub fn set_hover_dwell_ms(&mut self, dwell_ms: u64) {
        self.hover_dwell_ms = dwell_ms;
    }

    pub fn authorize(&self, command: CommandId) -> CadResult<()> {
        if self.mode == AppMode::Viewer && command.requires_work_mode() {
            Err(CadError::PermissionDenied)
        } else {
            Ok(())
        }
    }

    /// Switch modes; an unconfirmed tool is cancelled, never silently committed.
    pub fn switch_mode(&mut self, mode: AppMode) -> CadResult<()> {
        if self.mode != mode {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
        self.mode = mode;
        Ok(())
    }

    /// Cancel the current tool and any preview, returning to navigation.
    ///
    /// Cancelling never opens a transaction, so a cancelled measurement always
    /// leaves the drawing database and history untouched.
    pub fn cancel_tool(&mut self) -> CadResult<()> {
        if !matches!(self.tool, ToolState::Idle) {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
        Ok(())
    }

    fn bump_tool_generation(&mut self) {
        self.tool_generation = self.tool_generation.saturating_add(1);
    }

    /// Begin drawing capture and snapshot the current selection identity.
    pub fn begin_draw(&mut self, kind: DrawToolKind) -> CadResult<()> {
        self.authorize(match kind {
            DrawToolKind::Line => CommandId::CreateLine,
            DrawToolKind::Circle => CommandId::CreateCircle,
            DrawToolKind::Arc => CommandId::CreateArc,
            DrawToolKind::Polyline => CommandId::CreatePolyline,
            DrawToolKind::Rectang => CommandId::CreateRectangle,
            DrawToolKind::Ellipse => CommandId::CreateEllipse,
            DrawToolKind::Polygon => CommandId::CreatePolygon,
            DrawToolKind::Point => CommandId::CreatePoint,
            DrawToolKind::Spline => CommandId::CreateSpline,
            DrawToolKind::Move => CommandId::MoveEntities,
            DrawToolKind::Trim => CommandId::TrimEntity,
            DrawToolKind::Copy => CommandId::CopyEntities,
            DrawToolKind::Extend => CommandId::ExtendEntity,
            DrawToolKind::Rotate => CommandId::RotateEntities,
            DrawToolKind::Scale => CommandId::ScaleEntities,
            DrawToolKind::Mirror => CommandId::MirrorEntities,
            DrawToolKind::Offset => CommandId::OffsetEntity,
            DrawToolKind::Chamfer => CommandId::ChamferLines,
            DrawToolKind::Fillet => CommandId::FilletLines,
            DrawToolKind::ArrayRect => CommandId::ArrayRectangular,
            DrawToolKind::ArrayPolar => CommandId::ArrayPolar,
            DrawToolKind::Align => CommandId::AlignEntities,
            DrawToolKind::Stretch => CommandId::StretchEntities,
            DrawToolKind::DimLinear => CommandId::DimLinear,
            DrawToolKind::DimAligned => CommandId::DimAligned,
            DrawToolKind::DimRadius => CommandId::DimRadius,
            DrawToolKind::DimDiameter => CommandId::DimDiameter,
            DrawToolKind::DimCenter => CommandId::DimCenter,
            DrawToolKind::DimAngular => CommandId::DimAngular,
            DrawToolKind::DimOrdinate => CommandId::DimOrdinate,
            DrawToolKind::Hatch => CommandId::Hatch,
            DrawToolKind::Text => CommandId::CreateText,
        })?;
        let selection = self.selection.refs().to_vec();
        let tool = DrawTool::new(kind, selection.len());
        self.tool = ToolState::Drawing(DrawCapture { tool, selection });
        self.bump_tool_generation();
        Ok(())
    }

    /// Enter selection mode, replacing any active drawing or measurement tool.
    pub fn begin_selecting(&mut self) {
        self.tool = ToolState::Selecting;
        self.bump_tool_generation();
    }

    /// Start or restart the selected measurement algorithm.
    pub fn begin_measurement(&mut self, kind: MeasurementToolKind) -> CadResult<()> {
        self.authorize(CommandId::Measure)?;
        self.selected_measurement_kind = kind;
        self.tool = ToolState::Measuring(MeasurementTool::new(kind));
        self.bump_tool_generation();
        Ok(())
    }

    /// Change the retained measurement choice, restarting capture if active.
    pub fn set_measurement_kind(&mut self, kind: MeasurementToolKind) -> CadResult<()> {
        self.selected_measurement_kind = kind;
        Ok(())
    }

    pub fn selected_measurement_kind(&self) -> MeasurementToolKind {
        self.selected_measurement_kind
    }

    /// Replace any active capture with pan navigation without dispatching a Pan command.
    pub fn begin_pan(&mut self) {
        self.tool = ToolState::Panning;
        self.bump_tool_generation();
    }

    /// Clear only the active draw/measurement cursor, preserving capture and generation.
    pub fn clear_cursor(&mut self) {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_cursor(None),
            ToolState::Measuring(tool) => tool.set_cursor(None),
            ToolState::Idle | ToolState::Selecting | ToolState::Panning => {}
        }
    }

    /// Capture a finite point in the active drawing or measurement tool.
    /// Returns whether an auto-completing measurement became ready.
    ///
    /// For the entity-picking draw kinds (TRIM/EXTEND) this is **not** the
    /// capture path: their steps are entity picks and go through
    /// [`SessionState::capture_draw_pick`] with the ref the host resolved at
    /// capture time. A blind point for such a kind is refused rather than
    /// stored (see [`DrawTool::push_point`]).
    pub fn capture_point(&mut self, point: Point3) -> CadResult<bool> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("tool point must be finite".into()));
        }
        match &mut self.tool {
            ToolState::Drawing(capture) => {
                capture.tool.push_point(point)?;
                Ok(false)
            }
            ToolState::Measuring(tool) => {
                tool.push_point(point);
                Ok(tool.auto_ready())
            }
            _ => Err(CadError::InvalidInput(
                "no point-capture tool is active".into(),
            )),
        }
    }

    /// Capture one TRIM/EXTEND entity pick: the raw point and the resolved ref.
    ///
    /// The ref is resolved by the host **at capture time** through the one pick
    /// path; this method only stores the pair atomically, so the entity the user
    /// saw is exactly the one the commit will use. It refuses when no drawing
    /// tool is active or when the active kind is not an entity picker.
    pub fn capture_draw_pick(&mut self, point: Point3, reference: SelectionRef) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.push_pick(point, reference),
            _ => Err(CadError::InvalidInput("no drawing tool is active".into())),
        }
    }

    /// Whether the active drawing tool has a pending entity-pick step.
    ///
    /// True only for TRIM/EXTEND while a step is still un-captured; the host
    /// uses it to route a click to the pick path and to know whether to resolve
    /// a cursor-time candidate.
    pub fn draw_requires_entity_pick(&self) -> bool {
        matches!(
            &self.tool,
            ToolState::Drawing(capture) if capture.tool.requires_entity_pick()
        )
    }

    /// Record the entity under the cursor for the pending pick step.
    ///
    /// Presentation feedback only (never a commit source); a no-op for any tool
    /// that is not a drawing tool.
    pub fn set_draw_candidate(&mut self, candidate: Option<SelectionRef>) {
        if let ToolState::Drawing(capture) = &mut self.tool {
            capture.tool.set_candidate(candidate);
        }
    }

    /// Move the preview cursor without advancing the lifecycle generation.
    pub fn move_cursor(&mut self, point: Point3) -> CadResult<()> {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(CadError::InvalidInput("tool cursor must be finite".into()));
        }
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_cursor(Some(point)),
            ToolState::Measuring(tool) => tool.set_cursor(Some(point)),
            _ => return Err(CadError::InvalidInput("no cursor tool is active".into())),
        }
        Ok(())
    }

    pub fn draw_preview(&self) -> Option<DrawPreview> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(self.annotate_draw_preview(capture.tool.preview())),
            _ => None,
        }
    }

    /// Annotate a pure tool preview with the session parameters it needs.
    ///
    /// Only the POLYGON preview carries a session sysvar (the side count); the
    /// field is `None` for every other kind, so the frozen frontend projection
    /// (`CadTool.params.sides`) can never report a side count for a tool that has
    /// none.
    fn annotate_draw_preview(&self, mut preview: DrawPreview) -> DrawPreview {
        preview.polygon_sides =
            (preview.kind == DrawToolKind::Polygon).then_some(self.polygon_sides());
        preview
    }

    pub fn draw_intent(&self) -> CadResult<DrawIntent> {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.intent(),
            _ => Err(CadError::InvalidInput("no drawing tool is active".into())),
        }
    }

    /// Submit the terminal text of a text tool (ADR 0019 decision A.1).
    ///
    /// The authoritative validation lives in [`DrawTool::set_text`] (state,
    /// emptiness, the 1024-character bound), so this is the single entrance used
    /// by the host after `cad_set_tool_text`. A refusal names its reason and
    /// touches nothing; on success the stored text makes the pending command
    /// complete on the next confirmation — the host calls
    /// [`HostController::confirm_draw_traced`] immediately (text is terminal).
    pub fn set_tool_text(&mut self, text: &str) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Drawing(capture) => capture.tool.set_text(text),
            _ => Err(CadError::InvalidInput(
                "tool.text_not_awaiting: no drawing tool is active".into(),
            )),
        }
    }

    /// The first `Esc` while a text prompt is open (ADR 0019 decision A.1).
    ///
    /// Pure focus validation: the tool stays active and still awaits text. A
    /// second `Esc` is the ordinary cancel ([`Self::cancel_tool`]).
    pub fn dismiss_text_focus(&self) -> CadResult<()> {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.dismiss_text_focus(),
            _ => Err(CadError::InvalidInput(
                "tool.text_not_awaiting: no drawing tool is active".into(),
            )),
        }
    }

    /// Whether the active tool awaits its terminal text.
    pub fn draw_awaiting_text(&self) -> bool {
        matches!(&self.tool, ToolState::Drawing(capture) if capture.tool.awaiting_text())
    }

    pub fn draw_selection_snapshot(&self) -> Option<&[SelectionRef]> {
        match &self.tool {
            ToolState::Drawing(capture) => Some(&capture.selection),
            _ => None,
        }
    }

    pub fn tool_snapshot(&self) -> ToolSnapshot {
        match &self.tool {
            ToolState::Idle => ToolSnapshot::Idle,
            ToolState::Selecting => ToolSnapshot::Selecting,
            ToolState::Drawing(capture) => {
                ToolSnapshot::Drawing(self.annotate_draw_preview(capture.tool.preview()))
            }
            ToolState::Measuring(tool) => ToolSnapshot::Measuring(tool.preview()),
            ToolState::Panning => ToolSnapshot::Panning,
        }
    }

    /// Clear an active tool after successful execution, invalidating stale input.
    pub(crate) fn complete_tool(&mut self) {
        if !matches!(self.tool, ToolState::Idle) {
            self.tool = ToolState::Idle;
            self.bump_tool_generation();
        }
    }

    /// Number of entities currently held in the in-process clipboard.
    ///
    /// Pure getter; reading it never captures or mutates anything.
    pub fn clipboard_len(&self) -> usize {
        self.clipboard.len()
    }

    /// The last structured measurement result, if one has been confirmed.
    ///
    /// Pure getter: reading it never advances a tool or writes a database.
    pub fn last_measurement(&self) -> Option<&MeasurementRecord> {
        self.last_measurement.as_ref()
    }

    /// Whether a confirmed measurement result is currently retained.
    pub fn has_last_measurement(&self) -> bool {
        self.last_measurement.is_some()
    }

    /// Record the structured result of a completed measurement (F06).
    pub fn set_last_measurement(&mut self, record: MeasurementRecord) {
        self.last_measurement = Some(record);
    }

    /// Snapshot of the active measurement for preview rendering, if any.
    pub fn measurement_preview(&self) -> Option<MeasurementPreview> {
        match &self.tool {
            ToolState::Measuring(tool) => Some(tool.preview()),
            _ => None,
        }
    }

    /// Move the measurement preview cursor without capturing a point.
    ///
    /// This is a pure state update: it emits no command and touches no database.
    pub fn set_measurement_cursor(&mut self, cursor: Option<Point3>) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Measuring(tool) => {
                tool.set_cursor(cursor);
                Ok(())
            }
            _ => Err(CadError::InvalidInput(
                "no measurement tool is active".into(),
            )),
        }
    }
}

/// The Work-only command a [`ToolParameter`] is authorized against.
///
/// `PolygonSides` keeps its original authorization (`CreatePolygon`) exactly as
/// before the migration. `FilletRadius` and the two `ChamferDistance`s belong to
/// the FILLET/CHAMFER edit commands (ADR 0016 decisions B/C); the array
/// parameters belong to the ARRAYRECT/ARRAYPOLAR edit commands (ADR 0016 decision
/// D). Each parameter is authorized against its own command, so a Viewer-mode set
/// is refused exactly like the create commands.
fn authorizing_command(parameter: ToolParameter) -> CommandId {
    match parameter {
        ToolParameter::PolygonSides => CommandId::CreatePolygon,
        ToolParameter::FilletRadius => CommandId::FilletLines,
        ToolParameter::ChamferDistanceA | ToolParameter::ChamferDistanceB => {
            CommandId::ChamferLines
        }
        ToolParameter::ArrayRows
        | ToolParameter::ArrayColumns
        | ToolParameter::ArrayRowSpacing
        | ToolParameter::ArrayColumnSpacing => CommandId::ArrayRectangular,
        ToolParameter::ArrayItems
        | ToolParameter::ArrayFillAngle
        | ToolParameter::ArrayRotateItems => CommandId::ArrayPolar,
        // ALIGN (P4d): `AlignScale` is authorized against the ALIGN edit command,
        // so a Viewer-mode set is refused exactly like the other edit parameters.
        ToolParameter::AlignScale => CommandId::AlignEntities,
        // HATCH (P6b): every hatch parameter is authorized against the HATCH
        // create command, so a Viewer-mode set is refused exactly like the create
        // commands.
        ToolParameter::HatchPatternType
        | ToolParameter::HatchSpacing
        | ToolParameter::HatchAngle
        | ToolParameter::HatchDouble
        | ToolParameter::HatchStyle => CommandId::Hatch,
        // TEXT (P7a): the glyph height is authorized against the TEXT create
        // command, so a Viewer-mode set is refused exactly like the other create
        // parameters.
        ToolParameter::TextHeight => CommandId::CreateText,
    }
}
