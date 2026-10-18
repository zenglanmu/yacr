//! Shared commands and sessions. Hosts assemble services, not CAD business logic.
//!
//! Spec v2.0 §4.7, §4.10: commands express intent; transactions guard data;
//! history restores it; the change set notifies downstream. Mode permissions are
//! enforced here at the command layer, not by hiding UI buttons.

use cad_db::{ChangeSet, DrawingDatabase, MeasurementAlgorithm, MeasurementRecord};
use cad_domain::*;
use cad_history::History;
use cad_measure::{MeasurementEngine, MeasurementRequest, MeasurementSpace};
use cad_query::QueryService;
use cad_representation::{enumerate_layouts, viewport_transform, SpaceSelection, ViewportState};
use std::{collections::BTreeMap, sync::Arc};

pub mod api;
pub mod camera;
pub mod draw_tool;
pub mod host;
pub mod input;
pub mod interaction;
pub mod layers;
pub mod measure_tool;
pub mod picking;
pub mod recovery;
pub mod render_scene;
mod select_all;
mod select_box;
pub mod selection;
pub mod tasks;
pub mod tool_state;
pub mod viewer_config;

mod command_id;
mod space;

pub use api::{
    ApiOutcome, ApiRequest, ApiTarget, EntityEdit, NewEntity, RawCommandTrace, TracedCommandOutcome,
};
pub use cad_spatial::{
    BackFacePolicy, GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
};
pub use camera::{
    orthonormal_work_plane, xy_work_plane, Camera, Camera2dParams, Camera3dParams, Projection,
    ProjectionKind, StandardView, ViewBasis,
};
pub use command_id::*;
pub use draw_tool::{circle_radius, DrawIntent, DrawPreview, DrawTool, DrawToolKind};
pub use input::{
    apply_canvas_metrics, classify_key, escape_action, CancelReason, CanvasMetrics,
    DiagnosticsDrawer, EscapeAction, InputOutcome, InputPolicy, KeyAction, LoadingState,
    PointerPhase, PointerUpdate, StatusModel, ToolStatus, ViewMetrics, DRAG_THRESHOLD_LOGICAL_PX,
};
pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
pub use picking::{
    drawing_pick_items, filter_by_index, last_pick_stats, pick_at_screen, pick_ray,
    pick_ray_exhaustive, pick_ray_with_index, pick_tolerance, PickIndex, PickStats,
};
pub use recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
pub use selection::{entity_property_rows, PropertyRow, SelectionProperties, SelectionSet};
pub use space::*;
pub use tasks::{
    import_phase_key, AsyncOpenPoll, ImportJob, ImportManager, ImportProgressSnapshot,
    ImportTerminal,
};
pub use tool_state::{DrawCapture, ToolSnapshot, ToolState};

use interaction::SelectionMode;
use layers::LayerOverrideSet;

pub struct Document {
    pub id: DocumentId,
    pub drawing: Arc<DrawingDatabase>,
    pub identity: DocumentIdentity,
    pub units: UnitContext,
    pub resource_keys: Vec<String>,
}

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
    clipboard: Vec<ClipboardEntry>,
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
            DrawToolKind::Move => CommandId::MoveEntities,
            DrawToolKind::Trim => CommandId::TrimEntity,
            DrawToolKind::Copy => CommandId::CopyEntities,
            DrawToolKind::Extend => CommandId::ExtendEntity,
            DrawToolKind::Rotate => CommandId::RotateEntities,
            DrawToolKind::Scale => CommandId::ScaleEntities,
            DrawToolKind::Mirror => CommandId::MirrorEntities,
            DrawToolKind::Offset => CommandId::OffsetEntity,
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
            ToolState::Drawing(capture) => Some(capture.tool.preview()),
            _ => None,
        }
    }

    pub fn draw_intent(&self) -> CadResult<DrawIntent> {
        match &self.tool {
            ToolState::Drawing(capture) => capture.tool.intent(),
            _ => Err(CadError::InvalidInput("no drawing tool is active".into())),
        }
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
            ToolState::Drawing(capture) => ToolSnapshot::Drawing(capture.tool.preview()),
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

#[cfg(test)]
mod p2_interaction_tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn set_measurement_kind_does_not_start_or_replace_capture() {
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        session
            .set_measurement_kind(MeasurementToolKind::Area)
            .unwrap();
        assert_eq!(
            session.selected_measurement_kind(),
            MeasurementToolKind::Area
        );
        assert!(session.measurement_preview().is_none());
        assert_eq!(session.tool_generation, 0);

        session
            .begin_measurement(MeasurementToolKind::Distance)
            .unwrap();
        session.capture_point(point(1.0, 2.0)).unwrap();
        let generation = session.tool_generation;
        session
            .set_measurement_kind(MeasurementToolKind::Area)
            .unwrap();
        let preview = session.measurement_preview().unwrap();
        assert_eq!(preview.kind, MeasurementToolKind::Distance);
        assert_eq!(preview.points, vec![point(1.0, 2.0)]);
        assert_eq!(
            session.selected_measurement_kind(),
            MeasurementToolKind::Area
        );
        assert_eq!(session.tool_generation, generation);
    }

    #[test]
    fn begin_pan_replaces_capture_and_advances_tool_generation() {
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        session
            .begin_measurement(MeasurementToolKind::Distance)
            .unwrap();
        session.capture_point(point(1.0, 2.0)).unwrap();
        let generation = session.tool_generation;
        session.begin_pan();
        assert!(matches!(session.tool_snapshot(), ToolSnapshot::Panning));
        assert!(session.measurement_preview().is_none());
        assert_eq!(session.tool_generation, generation + 1);
    }

    #[test]
    fn clear_cursor_preserves_captured_points_and_tool_generation() {
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        session
            .begin_measurement(MeasurementToolKind::Distance)
            .unwrap();
        session.capture_point(point(1.0, 2.0)).unwrap();
        session.move_cursor(point(3.0, 4.0)).unwrap();
        let generation = session.tool_generation;
        session.clear_cursor();
        let preview = session.measurement_preview().unwrap();
        assert_eq!(preview.points, vec![point(1.0, 2.0)]);
        assert_eq!(preview.cursor, None);
        assert_eq!(session.tool_generation, generation);

        session.begin_draw(DrawToolKind::Line).unwrap();
        session.capture_point(point(5.0, 6.0)).unwrap();
        session.move_cursor(point(7.0, 8.0)).unwrap();
        let generation = session.tool_generation;
        session.clear_cursor();
        let ToolSnapshot::Drawing(preview) = session.tool_snapshot() else {
            panic!("draw capture must remain active");
        };
        assert_eq!(preview.points, vec![point(5.0, 6.0)]);
        assert_eq!(preview.cursor, None);
        assert_eq!(session.tool_generation, generation);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayStyle {
    Wireframe,
    Shaded,
    ShadedWithEdges,
}

#[derive(Debug)]
pub struct Viewport {
    pub id: ViewportId,
    pub document: DocumentId,
    pub camera: Camera,
    /// The observation mode; `TwoD` stores the camera to restore on switch-back.
    pub view_mode: ViewMode2d3d,
    pub work_plane: WorkPlane,
    pub logical_size: [f64; 2],
    pub dpi_scale: f64,
    pub clip: Option<Bounds3>,
    pub style: DisplayStyle,
}

/// Whether the viewport observes the drawing in 2D (plan) or 3D.
///
/// The switch is *lossless*: entering 3D snapshots the exact 2D camera, and a
/// switch back restores it bit-for-bit rather than re-deriving a top view
/// (audit F13 "回到 2D"). A fresh viewport starts in 2D.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewMode2d3d {
    /// Plan view. Carries the camera to restore after a 3D excursion.
    TwoD { saved: Camera },
    /// 3D orbit view. Carries the 2D camera captured when the excursion began.
    ThreeD { saved_2d: Camera },
}

impl ViewMode2d3d {
    /// The kind without the saved camera payload.
    pub fn kind(&self) -> ProjectionKind {
        match self {
            ViewMode2d3d::TwoD { .. } => ProjectionKind::TwoD,
            ViewMode2d3d::ThreeD { .. } => ProjectionKind::ThreeD,
        }
    }

    /// The 2D camera that a switch back to plan will restore.
    pub fn saved_2d_camera(&self) -> Camera {
        match self {
            ViewMode2d3d::TwoD { saved } => *saved,
            ViewMode2d3d::ThreeD { saved_2d } => *saved_2d,
        }
    }
}

impl Viewport {
    pub fn new(id: ViewportId, document: DocumentId, logical_size: [f64; 2]) -> Self {
        let camera = Camera::top_view_2d();
        Viewport {
            id,
            document,
            camera,
            view_mode: ViewMode2d3d::TwoD { saved: camera },
            work_plane: xy_work_plane(0.0),
            logical_size,
            dpi_scale: 1.0,
            clip: None,
            style: DisplayStyle::Wireframe,
        }
    }

    /// World units per logical pixel in the top-down 2D view.
    pub fn world_per_px(&self) -> f64 {
        match self.camera.projection {
            Projection::Orthographic { scale } => scale.max(camera::MIN_ORTHO_SCALE),
            Projection::Perspective { .. } => 1.0,
        }
    }

    /// Map a canvas point in logical pixels to a world point on the work plane.
    ///
    /// `logical` is measured from the top-left of the CAD content rectangle with
    /// y growing downwards (the screen/pointer convention); the world y axis
    /// points up, so the sign flips. The camera target sits at the rectangle
    /// centre. This is the inverse of the 2D view mapping and is the function a
    /// host uses to turn a measurement pick into a real point.
    ///
    /// Returns `None` for a non-finite or degenerate input rather than producing
    /// a fabricated point (audit B17/B24).
    pub fn screen_to_world(
        &self,
        logical: [f64; 2],
        canvas_logical_size: [f64; 2],
    ) -> Option<Point3> {
        self.camera
            .screen_to_plan_world(logical, canvas_logical_size)
            .map(|p| Point3 {
                z: self.work_plane.origin.z,
                ..p
            })
    }

    /// Switch the observation mode, preserving the 2D camera on a round trip.
    ///
    /// Entering `ThreeD` snapshots the current 2D camera and promotes the
    /// projection to perspective. Returning `TwoD` restores the exact camera
    /// captured on the way in, so the round trip is lossless.
    pub fn set_view_mode(&mut self, mode: ViewMode2d3d) -> CadResult<()> {
        match mode {
            ViewMode2d3d::ThreeD { saved_2d } => {
                if self.camera.projection.is_orthographic() {
                    self.camera.projection =
                        Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
                }
                self.view_mode = ViewMode2d3d::ThreeD { saved_2d };
                Ok(())
            }
            ViewMode2d3d::TwoD { saved } => {
                self.camera = saved;
                self.view_mode = ViewMode2d3d::TwoD { saved };
                Ok(())
            }
        }
    }
}

pub struct PreviewState {
    pub viewport: ViewportId,
    pub geometry: Vec<SemanticGeometry>,
}

#[derive(Default)]
pub struct Workspace {
    pub documents: BTreeMap<DocumentId, Document>,
    pub viewports: BTreeMap<ViewportId, Viewport>,
}

pub enum CommandPayload {
    None,
    Points(Vec<Point3>),
    Layer(LayerId, bool),
    /// Distinct existing layer ids and their temporary visibility values.
    /// Empty batches and duplicate ids (including equal values) are invalid.
    LayerVisibilities(Vec<(LayerId, bool)>),
    Space(SpaceId),
    StandardView(StandardView),
    Backend(BackendChoice),
    /// Start (or restart) a measurement tool with an explicit algorithm.
    MeasureTool(MeasurementToolKind),
    /// Orbit the current view: yaw about world +Z, pitch about the view right
    /// axis, both in radians. Preserves the eye-target distance (audit F13).
    Orbit {
        yaw: f64,
        pitch: f64,
    },
    /// Structured zoom: a positive factor plus the cursor in logical pixels that
    /// must stay anchored. Distinct from the legacy `Points`-payload zoom that
    /// takes no cursor.
    ZoomAt {
        factor: f64,
        cursor: [f64; 2],
    },
    /// Target mode for [`CommandId::SetMode`] (audit U02).
    Mode(AppMode),
    /// Replace the current selection with these refs (read-only; never writes
    /// the DWG). An empty list clears the selection.
    Selection(Vec<SelectionRef>),
    /// Select every visible model-space item whose geometry satisfies a
    /// window/crossing rectangle (ADR 0014 B).
    ///
    /// `rect_logical` is two opposite corners in **canvas logical pixels**, the
    /// same convention as [`CommandPayload::ZoomAt`]; the command maps them to
    /// the active viewport's work plane. `crossing == false` is a WINDOW
    /// selection (fully inside only); `crossing == true` is a CROSSING
    /// selection (intersect or contain), i.e. the right-to-left drag semantic.
    /// `mode` is the merge mode applied to the current selection. Read-only: no
    /// ChangeSet, no revision change, no history entry.
    BoxSelect {
        rect_logical: [[f64; 2]; 2],
        crossing: bool,
        mode: SelectionMode,
    },
    /// Translate a set of selected refs by a world delta (MOVE, spec F-EDIT).
    Move {
        refs: Vec<SelectionRef>,
        delta: Point3,
    },
    /// Clone a set of selected refs with fresh ids, translating each copy by a
    /// world delta (COPY, spec F-EDIT). Same interaction shape as `Move`.
    Copy {
        refs: Vec<SelectionRef>,
        delta: Point3,
    },
    /// Rotate a set of selected refs about `base` by `angle` radians in the XY
    /// plane (ROTATE, spec F-EDIT).
    Rotate {
        refs: Vec<SelectionRef>,
        base: Point3,
        angle: f64,
    },
    /// Scale a set of selected refs uniformly about `base` by `factor` (SCALE,
    /// spec F-EDIT). `factor` must be positive and finite.
    Scale {
        refs: Vec<SelectionRef>,
        base: Point3,
        factor: f64,
    },
    /// Mirror a set of selected refs across the line through `axis_start` and
    /// `axis_end`, keeping the originals and adding mirrored copies (MIRROR,
    /// spec F-EDIT). Same interaction shape as `Copy`.
    Mirror {
        refs: Vec<SelectionRef>,
        axis_start: Point3,
        axis_end: Point3,
    },
    /// Offset a single selected LINE/CIRCLE by the distance and side defined by
    /// `side_point` (OFFSET, spec F-EDIT). Exactly one source entity is required;
    /// any other geometry kind is an explicit refusal.
    Offset {
        refs: Vec<SelectionRef>,
        side_point: Point3,
    },
    /// Trim one target against a list of boundary refs, keeping the side that
    /// contains `pick_point` (TRIM, spec F-EDIT). See `docs/drawing-edit.md` §3
    /// for the supported subset.
    Trim {
        target: SelectionRef,
        boundary: Vec<SelectionRef>,
        pick_point: Point3,
    },
    /// Extend one target LINE to the nearest boundary segment beyond the picked
    /// end (EXTEND, spec F-EDIT). Same interaction shape and supported subset as
    /// [`CommandPayload::Trim`], but the target grows instead of shrinking.
    Extend {
        target: SelectionRef,
        boundary: Vec<SelectionRef>,
        pick_point: Point3,
    },
    /// Set the session's active layer for new entities. Session state only.
    ActiveLayer(LayerId),
    /// A fully formed semantic geometry, used by `CreateLine` as the
    /// programmatic alternative to a point list.
    Geometry(Box<SemanticGeometry>),
}

pub struct Command {
    pub schema_version: u32,
    pub id: CommandId,
    pub document: DocumentId,
    pub viewport: ViewportId,
    pub payload: CommandPayload,
}

#[derive(Debug)]
pub struct CommandOutcome {
    pub objects: Vec<ObjectId>,
    pub changes: Option<Arc<ChangeSet>>,
    pub diagnostics: Vec<Diagnostic>,
    /// Structured measurement result, when the command produced one (spec F06).
    pub measurement: Option<MeasurementRecord>,
}

impl CommandOutcome {
    pub fn none() -> Self {
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: Vec::new(),
            measurement: None,
        }
    }
}

/// Independent undo/redo availability for one document.
///
/// UI layers must bind undo and redo to their own flags (audit U11); reading
/// this snapshot is a pure operation and never emits a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryAvailability {
    pub can_undo: bool,
    pub can_redo: bool,
}

pub struct Application {
    pub workspace: Workspace,
    pub history: BTreeMap<DocumentId, History>,
    pub measurement: MeasurementEngine,
    pub query: QueryService,
}

impl Default for Application {
    fn default() -> Self {
        Application::new()
    }
}

pub enum InputEvent {
    Pointer {
        logical_position: [f64; 2],
        contacts: u8,
    },
    Confirm,
    Cancel,
    Key {
        key: String,
        composing: bool,
        text_focus: bool,
    },
    ViewMetrics {
        logical_size: [f64; 2],
        dpi_scale: f64,
    },
}

pub trait Tool {
    fn handle(
        &mut self,
        event: InputEvent,
        session: &mut SessionState,
        preview: &mut PreviewState,
    ) -> CadResult<Option<Command>>;
}

mod app_drawing;
mod app_history;
mod app_measure;
mod application;

#[cfg(not(target_arch = "wasm32"))]
pub mod background;
#[cfg(test)]
mod tests;
