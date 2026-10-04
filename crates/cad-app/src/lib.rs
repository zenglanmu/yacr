//! Shared commands and sessions. Hosts assemble services, not CAD business logic.
//!
//! Spec v2.0 §4.7, §4.10: commands express intent; transactions guard data;
//! history restores it; the change set notifies downstream. Mode permissions are
//! enforced here at the command layer, not by hiding UI buttons.

use cad_annotations::{AnnotationCommand, AnnotationService};
use cad_db::{
    AnnotationDatabase, AnnotationStyle, ChangeSet, DrawingDatabase, MeasurementAlgorithm,
    MeasurementRecord,
};
use cad_domain::*;
use cad_history::{patch, History, UndoRecord};
use cad_measure::{MeasurementEngine, MeasurementRequest, MeasurementSpace};
use cad_query::QueryService;
use cad_representation::{enumerate_layouts, viewport_transform, SpaceSelection, ViewportState};
use std::{collections::BTreeMap, sync::Arc};

pub mod annotation_list;
pub mod annotation_tool;
pub mod camera;
pub mod draw_tool;
pub mod host;
pub mod host_files;
pub mod input;
pub mod layers;
pub mod measure_tool;
pub mod picking;
pub mod recovery;
pub mod render_scene;
mod select_all;
pub mod selection;
pub mod tasks;
pub mod viewer_config;

pub use annotation_list::{annotation_rows, geometry_kind, AnnotationRow, AnnotationVisibilitySet};
pub use annotation_tool::{AnnotationPreview, AnnotationTool, AnnotationToolKind};
pub use cad_spatial::{
    BackFacePolicy, GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
};
pub use camera::{
    orthonormal_work_plane, xy_work_plane, Camera, Camera2dParams, Camera3dParams, Projection,
    ProjectionKind, StandardView, ViewBasis,
};
pub use draw_tool::{circle_radius, DrawIntent, DrawPreview, DrawTool, DrawToolKind};
pub use host_files::{
    decision_name, parse_decision, plan_leave, LeavePlan, LeaveResolution, UnsavedDecisionSource,
    UnsavedSignal,
};
pub use input::{
    apply_canvas_metrics, classify_key, escape_action, CancelReason, CanvasMetrics,
    DiagnosticsDrawer, EscapeAction, InputOutcome, InputPolicy, KeyAction, LoadingState,
    PointerPhase, PointerUpdate, StatusModel, ToolStatus, ViewMetrics, DRAG_THRESHOLD_LOGICAL_PX,
};
pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
pub use picking::{drawing_pick_items, filter_by_index, pick_at_screen, pick_ray, pick_tolerance};
pub use recovery::{
    ActiveBackendKind, BackendFailure, BackendOutcome, RecoverySnapshot, UnsavedDecision,
    UnsavedFlow, UnsavedOutcome,
};
pub use selection::{entity_property_rows, PropertyRow, SelectionProperties, SelectionSet};
pub use tasks::{
    import_phase_key, AsyncOpenPoll, ImportJob, ImportManager, ImportProgressSnapshot,
    ImportTerminal,
};

use layers::LayerOverrideSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Viewer,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenDrawing,
    CancelLoading,
    Pan,
    Zoom,
    FitDrawing,
    ResetView,
    ToggleLayer,
    /// Atomically publish a nonempty batch of temporary layer overrides.
    /// Allowed in Viewer and Work; no drawing write, reparse or undo entry.
    SetLayerVisibilities,
    RestoreLayers,
    SwitchSpace,
    Select,
    /// Select visible model-space pick items without editing the drawing.
    /// Requires `CommandPayload::None`; incomplete picking is explicitly refused.
    SelectAll,
    Measure,
    /// Confirm the points captured by the active open-ended measurement tool.
    ConfirmMeasurement,
    /// Persist the last confirmed measurement as a measurement annotation
    /// (F06/F07). Exactly one transaction and one undo step through the shared
    /// annotation path; refused when no measurement has been confirmed.
    SaveMeasurementAsAnnotation,
    /// Cancel the active tool without committing anything.
    CancelMeasurement,
    CreateAnnotation,
    UpdateAnnotation,
    DeleteAnnotation,
    /// Delete one annotation addressed by payload id (UI-friendly form of
    /// `DeleteAnnotation`, which needs the raw annotation command payload).
    DeleteAnnotationById,
    /// Start (or restart) an annotation creation tool for an explicit kind.
    ///
    /// This is *not* the same as `CreateAnnotation`, which commits a fully
    /// specified `AnnotationCommand`. `BeginAnnotationTool` only opens the
    /// capture state machine and commits nothing by itself.
    BeginAnnotationTool,
    /// Confirm the annotation captured by the active tool: exactly one
    /// transaction through the shared history path.
    ConfirmAnnotationTool,
    /// Cancel the active annotation tool; never opens a transaction.
    CancelAnnotationTool,
    /// Append world points to the active annotation tool.
    AppendAnnotationPoints,
    /// Supply the text payload for the active annotation tool (text/leader).
    AnnotationText,
    /// Temporarily hide or show one existing annotation (session state only).
    SetAnnotationVisibility,
    /// Select one annotation for edit/delete, or clear the selection (None).
    SelectAnnotation,
    Undo,
    Redo,
    ImportAnnotations,
    ExportAnnotations,
    Resources,
    Diagnostics,
    SwitchBackend,
    Switch2d3d,
    StandardView,
    Orbit,
    SwitchProjection,
    /// Switch the session between Viewer and Work mode (audit U02).
    ///
    /// Allowed in both directions and in both modes: the mode change itself is
    /// not a Work-only operation, but it cancels any unconfirmed tool. The
    /// command layer still refuses every Work-only command while in Viewer mode.
    SetMode,
    /// Draw a LINE through two payload points into model space. Work-only.
    CreateLine,
    /// Draw a CIRCLE with a centre and an edge point. The radius is the exact
    /// centre→edge distance and must be positive. Work-only.
    CreateCircle,
    /// Translate one or more selected entities by a world delta. Exactly one
    /// transaction and one undo step for the whole selection. Work-only.
    MoveEntities,
    /// Trim one LINE against LINE/LWPOLYLINE segments (the honest subset of
    /// §3 of `docs/drawing-edit.md`); anything else is an explicit refusal that
    /// changes nothing. Work-only.
    TrimEntity,
    /// Set the session's active layer for new drawing entities. Session state
    /// only; no database write. Work-only.
    SetActiveLayer,
}

impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::Measure
                | Self::ConfirmMeasurement
                | Self::SaveMeasurementAsAnnotation
                | Self::CreateAnnotation
                | Self::UpdateAnnotation
                | Self::DeleteAnnotation
                | Self::DeleteAnnotationById
                | Self::BeginAnnotationTool
                | Self::ConfirmAnnotationTool
                | Self::CancelAnnotationTool
                | Self::AppendAnnotationPoints
                | Self::AnnotationText
                | Self::SetAnnotationVisibility
                | Self::SelectAnnotation
                | Self::Undo
                | Self::Redo
                | Self::ImportAnnotations
                | Self::CreateLine
                | Self::CreateCircle
                | Self::MoveEntities
                | Self::TrimEntity
                | Self::SetActiveLayer
        )
    }
}

pub struct Document {
    pub id: DocumentId,
    pub drawing: Arc<DrawingDatabase>,
    pub annotations: AnnotationDatabase,
    pub identity: DocumentIdentity,
    pub units: UnitContext,
    pub resource_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ToolState {
    Idle,
    Selecting,
    Measuring(MeasurementTool),
    Annotating(AnnotationTool),
    Panning,
}

pub struct SessionState {
    pub document: DocumentId,
    mode: AppMode,
    pub active_space: SpaceId,
    pub selection: SelectionSet,
    /// Temporary layer visibility. Never the drawing's layer table (F03).
    pub layer_overrides: LayerOverrideSet,
    pub tool: ToolState,
    /// Session-scoped temporary annotation visibility, keyed by annotation id.
    ///
    /// `AnnotationDatabase` has no hidden field and the sidecar format does not
    /// carry one, so visibility is a *session override* exactly like the layer
    /// overrides (F03/F09). It never mutates an annotation, never raises the
    /// annotation revision and therefore never creates a history entry.
    pub annotation_visibility: AnnotationVisibilitySet,
    /// The annotation currently selected for edit/delete in the management UI.
    pub selected_annotation: Option<AnnotationId>,
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
    /// The last structured measurement result produced in this session (F06).
    ///
    /// Set when a measurement is evaluated (auto-completed or explicitly
    /// confirmed). It is what [`CommandId::SaveMeasurementAsAnnotation`] turns
    /// into an annotation; it is never a fabricated placeholder. Cleared when the
    /// session is rebuilt for new content.
    last_measurement: Option<MeasurementRecord>,
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
            annotation_visibility: AnnotationVisibilitySet::new(),
            selected_annotation: None,
            generation: 0,
            backend: BackendChoice::Auto,
            active_layer: LayerId(0),
            last_measurement: None,
        }
    }

    pub fn mode(&self) -> AppMode {
        self.mode
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
        if !matches!(self.tool, ToolState::Idle) {
            self.tool = ToolState::Idle;
        }
        self.mode = mode;
        Ok(())
    }

    /// Cancel the current tool and any preview, returning to navigation.
    ///
    /// Cancelling never opens a transaction, so a cancelled measurement always
    /// leaves the annotation database and history untouched.
    pub fn cancel_tool(&mut self) -> CadResult<()> {
        self.tool = ToolState::Idle;
        Ok(())
    }

    /// The last structured measurement result, if one has been confirmed.
    ///
    /// Pure getter: reading it never advances a tool or writes a database.
    pub fn last_measurement(&self) -> Option<&MeasurementRecord> {
        self.last_measurement.as_ref()
    }

    /// Whether a confirmed measurement is available to persist as an annotation.
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

    /// Snapshot of the active annotation tool for preview, if any.
    pub fn annotation_preview(&self) -> Option<AnnotationPreview> {
        match &self.tool {
            ToolState::Annotating(tool) => Some(tool.preview()),
            _ => None,
        }
    }

    /// Move the annotation preview cursor without capturing a point.
    ///
    /// Pure state update: no command, no database write.
    pub fn set_annotation_cursor(&mut self, cursor: Option<Point3>) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Annotating(tool) => {
                tool.set_cursor(cursor);
                Ok(())
            }
            _ => Err(CadError::InvalidInput(
                "no annotation tool is active".into(),
            )),
        }
    }

    /// Set the text payload of the active annotation tool.
    pub fn set_annotation_text(&mut self, text: impl Into<String>) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Annotating(tool) => tool.set_text(text),
            _ => Err(CadError::InvalidInput(
                "no annotation tool is active".into(),
            )),
        }
    }

    /// Effective visibility of one annotation id under the session overrides.
    ///
    /// Absent overrides mean visible: the sidecar format has no hidden flag, so
    /// "visible" is the honest default and a hide is an explicit session action.
    pub fn annotation_visible(&self, id: AnnotationId) -> bool {
        self.annotation_visibility.effective(id)
    }

    /// Whether the session has hidden this annotation.
    pub fn annotation_hidden(&self, id: AnnotationId) -> bool {
        self.annotation_visibility.is_hidden(id)
    }

    /// Apply a temporary visibility override for one annotation.
    pub fn set_annotation_visibility(&mut self, id: AnnotationId, visible: bool) {
        self.annotation_visibility.set(id, visible);
    }

    /// Drop every annotation visibility override.
    pub fn clear_annotation_visibility(&mut self) {
        self.annotation_visibility.clear();
    }

    /// Read-only management rows for the annotation list panel.
    ///
    /// Pure projection of the document database plus the session visibility and
    /// selection; it never dispatches a command.
    pub fn annotation_rows(&self, database: &AnnotationDatabase) -> Vec<AnnotationRow> {
        annotation_list::annotation_rows(
            database,
            &self.annotation_visibility,
            self.selected_annotation,
        )
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
    /// y growing downwards (the Slint/pointer convention); the world y axis
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
    Annotation(Box<AnnotationCommand>),
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
    /// Start (or restart) an annotation creation tool with an explicit kind.
    AnnotationTool(AnnotationToolKind),
    /// Append world points to the active capture tool (measure or annotation).
    ///
    /// Reuses `Points`; the active tool decides what the points mean. Kept as a
    /// named variant so a caller can be explicit.
    AppendAnnotationPoints(Vec<Point3>),
    /// Text payload for the active annotation tool (text/leader kinds).
    AnnotationText(String),
    /// Hide/show one existing annotation. Session state only, no transaction.
    AnnotationVisibility(AnnotationId, bool),
    /// Select one annotation for edit/delete in the management panel.
    SelectAnnotation(Option<AnnotationId>),
    /// Delete one annotation by id. Opens exactly one transaction.
    DeleteAnnotation(AnnotationId),
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
    /// Translate a set of selected refs by a world delta (MOVE, spec F-EDIT).
    Move {
        refs: Vec<SelectionRef>,
        delta: Point3,
    },
    /// Trim one target against a list of boundary refs, keeping the side that
    /// contains `pick_point` (TRIM, spec F-EDIT). See `docs/drawing-edit.md` §3
    /// for the supported subset.
    Trim {
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
    pub changes: Option<ChangeSet>,
    pub diagnostics: Vec<Diagnostic>,
    /// Structured measurement result, when the command produced one (spec F06).
    pub measurement: Option<MeasurementRecord>,
    /// The annotation created/updated by a command, when it produced one.
    pub annotation: Option<AnnotationId>,
}

impl CommandOutcome {
    pub fn none() -> Self {
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: Vec::new(),
            measurement: None,
            annotation: None,
        }
    }
}

/// Validate that model space or a named paper layout can be drawn.
///
/// `SpaceSelection::Model` is always valid. A paper selection must name a layout
/// that exists in `database` **and** whose viewports this build can draw; an
/// unknown layout is `InvalidInput` and a known-but-undrawable one is
/// `Unsupported` with the representation layer's own reason. This is what keeps
/// an unsupported layout an explicit diagnostic instead of a blank success
/// (audit F04/B22).
pub fn validate_space(database: &DrawingDatabase, space: SpaceSelection) -> CadResult<()> {
    match space {
        SpaceSelection::Model => Ok(()),
        SpaceSelection::Paper(id) => match enumerate_layouts(database)
            .into_iter()
            .find(|descriptor| descriptor.id == id)
        {
            None => Err(CadError::InvalidInput(format!(
                "unknown paper layout {}",
                id.0
            ))),
            Some(descriptor) if !descriptor.supported => Err(CadError::Unsupported(format!(
                "layout {:?} cannot be drawn: {}",
                descriptor.name, descriptor.reason
            ))),
            Some(_) => Ok(()),
        },
    }
}

/// Validate the domain `SpaceId` payload form of a space switch.
///
/// `SpaceId::Block` is block-definition space, which is reached only through an
/// `INSERT` and is never a selectable observation space, so it is refused.
pub fn validate_space_id(database: &DrawingDatabase, space: &SpaceId) -> CadResult<()> {
    match space {
        SpaceId::Model => Ok(()),
        SpaceId::Paper(id) => validate_space(database, SpaceSelection::Paper(*id)),
        SpaceId::Block(_) => Err(CadError::InvalidInput(
            "block space is not a selectable drawing space".into(),
        )),
    }
}

/// The measurement space for a pick in the session's active space.
///
/// Model space keeps the existing split (planar algorithms use the viewport work
/// plane, spatial ones use [`MeasurementSpace::World3d`]). Paper space measures
/// the sheet directly, so a distance becomes a planar paper distance and an
/// angle (undefined on a 2D sheet) is refused rather than guessed. This is what
/// keeps a paper distance distinguishable from a model distance (F04/F06).
pub fn resolve_measurement_space(
    active_space: &SpaceId,
    work_plane: WorkPlane,
    algorithm: MeasurementAlgorithm,
) -> CadResult<(MeasurementAlgorithm, MeasurementSpace)> {
    match active_space {
        SpaceId::Model => {
            let space = match algorithm {
                MeasurementAlgorithm::Distance3d | MeasurementAlgorithm::Angle3Points => {
                    MeasurementSpace::World3d
                }
                _ => MeasurementSpace::Plane(work_plane),
            };
            Ok((algorithm, space))
        }
        SpaceId::Paper(layout) => match algorithm {
            // A 3D distance on a 2D sheet is a planar paper distance.
            MeasurementAlgorithm::Distance3d => Ok((
                MeasurementAlgorithm::Distance2d,
                MeasurementSpace::Paper(*layout),
            )),
            MeasurementAlgorithm::Angle3Points => Err(CadError::Unsupported(format!(
                "angle is not defined in paper space of layout {}; switch to model space",
                layout.0
            ))),
            _ => Ok((algorithm, MeasurementSpace::Paper(*layout))),
        },
        SpaceId::Block(_) => Err(CadError::InvalidInput(
            "block space is not a selectable drawing space".into(),
        )),
    }
}

/// The [`MeasurementSpace`] for measuring *inside* one layout viewport.
///
/// The returned space carries the viewport's verified paper→model inverse, so a
/// pick on the sheet measures real model distance. A viewport whose state cannot
/// be represented has no valid inverse and is refused with the representation
/// layer's stable reason: model measurement is disabled, never guessed from
/// paper pixels (F04).
pub fn viewport_measurement_space(
    database: &DrawingDatabase,
    layout_id: LayoutId,
    viewport_index: usize,
) -> CadResult<MeasurementSpace> {
    let layout = database
        .layout(layout_id)
        .ok_or_else(|| CadError::InvalidInput(format!("unknown paper layout {}", layout_id.0)))?;
    let viewport = layout.viewports.get(viewport_index).ok_or_else(|| {
        CadError::InvalidInput(format!(
            "layout {} has no viewport {}",
            layout_id.0, viewport_index
        ))
    })?;
    match viewport_transform(viewport) {
        ViewportState::Supported(t) => Ok(MeasurementSpace::ViewportModel {
            layout: layout_id,
            inverse: t.to_model,
        }),
        ViewportState::Unsupported(why) => Err(CadError::Unsupported(format!(
            "layout {} viewport {} cannot be measured in model space: {why}",
            layout_id.0, viewport_index
        ))),
    }
}

pub struct CommandDeclaration {
    pub id: CommandId,
    pub undoable: bool,
    pub annotation_database: bool,
    pub merge_key: Option<String>,
}

pub trait CommandHandler {
    fn declaration(&self) -> CommandDeclaration;
    fn execute(
        &self,
        session: &mut SessionState,
        document: &mut Document,
        command: &Command,
    ) -> CadResult<CommandOutcome>;
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
    pub annotations: AnnotationService,
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

mod app_annotation;
mod app_drawing;
mod app_history;
mod app_measure;
mod application;

#[cfg(test)]
mod tests;
