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
use std::{collections::BTreeMap, sync::Arc};

pub mod annotation_list;
pub mod annotation_tool;
pub mod camera;
pub mod host;
pub mod input;
pub mod layers;
pub mod measure_tool;
pub mod recovery;
pub mod selection;

pub use annotation_list::{annotation_rows, geometry_kind, AnnotationRow, AnnotationVisibilitySet};
pub use annotation_tool::{AnnotationPreview, AnnotationTool, AnnotationToolKind};
pub use camera::{
    orthonormal_work_plane, xy_work_plane, Camera, Projection, ProjectionKind, StandardView,
    ViewBasis,
};
pub use input::{
    apply_canvas_metrics, classify_key, escape_action, CancelReason, CanvasMetrics,
    DiagnosticsDrawer, EscapeAction, InputOutcome, InputPolicy, KeyAction, LoadingState,
    PointerPhase, PointerUpdate, StatusModel, ToolStatus, ViewMetrics, DRAG_THRESHOLD_LOGICAL_PX,
};
pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
pub use recovery::{
    ActiveBackendKind, BackendFailure, BackendOutcome, RecoverySnapshot, UnsavedDecision,
    UnsavedFlow, UnsavedOutcome,
};
pub use selection::{entity_property_rows, PropertyRow, SelectionProperties, SelectionSet};

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
    RestoreLayers,
    SwitchSpace,
    Select,
    Measure,
    /// Confirm the points captured by the active open-ended measurement tool.
    ConfirmMeasurement,
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
}

impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(
            self,
            Self::Measure
                | Self::ConfirmMeasurement
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
    /// Replace the current selection with these refs (read-only; never writes
    /// the DWG). An empty list clears the selection.
    Selection(Vec<SelectionRef>),
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

impl Application {
    pub fn new() -> Self {
        Application {
            workspace: Workspace::default(),
            history: BTreeMap::new(),
            measurement: MeasurementEngine::default(),
            annotations: AnnotationService,
            query: QueryService::new(),
        }
    }

    /// Execute a command. This is the single entry point shared by UI and CLI.
    pub fn execute(
        &mut self,
        session: &mut SessionState,
        command: Command,
    ) -> CadResult<CommandOutcome> {
        session.authorize(command.id)?;
        if session.document != command.document {
            return Err(CadError::StaleResult);
        }
        match command.id {
            CommandId::FitDrawing => self.fit_drawing(session, &command),
            CommandId::RestoreLayers => {
                // Drop temporary overrides; the drawing's own layer flags remain
                // the authority and the database revision is untouched (F03).
                session.layer_overrides.clear();
                Ok(CommandOutcome::none())
            }
            CommandId::ToggleLayer => {
                if let CommandPayload::Layer(id, visible) = command.payload {
                    session.layer_overrides.set(id, visible);
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "ToggleLayer needs a layer payload".into(),
                    ))
                }
            }
            CommandId::SwitchSpace => {
                if let CommandPayload::Space(space) = command.payload {
                    session.active_space = space;
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchSpace needs a space payload".into(),
                    ))
                }
            }
            CommandId::Measure => self.measure(session, &command),
            CommandId::AppendAnnotationPoints => match &command.payload {
                CommandPayload::AppendAnnotationPoints(points) | CommandPayload::Points(points) => {
                    self.capture_annotation_points(session, points)
                }
                _ => Err(CadError::InvalidInput(
                    "AppendAnnotationPoints needs a points payload".into(),
                )),
            },
            CommandId::AnnotationText => match &command.payload {
                CommandPayload::AnnotationText(text) => {
                    session.set_annotation_text(text.clone())?;
                    Ok(self.annotation_preview_outcome(session))
                }
                _ => Err(CadError::InvalidInput(
                    "AnnotationText needs a text payload".into(),
                )),
            },
            CommandId::ConfirmMeasurement => self.confirm_measurement(session, &command),
            CommandId::CancelMeasurement => {
                // Cancelling is always allowed and never opens a transaction.
                session.cancel_tool()?;
                Ok(CommandOutcome::none())
            }
            CommandId::CreateAnnotation
            | CommandId::UpdateAnnotation
            | CommandId::DeleteAnnotation => self.annotation_command(&command),
            CommandId::DeleteAnnotationById => match &command.payload {
                CommandPayload::DeleteAnnotation(id) => {
                    let forwarded = Command {
                        schema_version: command.schema_version,
                        id: CommandId::DeleteAnnotation,
                        document: command.document,
                        viewport: command.viewport,
                        payload: CommandPayload::Annotation(Box::new(AnnotationCommand::Delete(
                            *id,
                        ))),
                    };
                    self.annotation_command(&forwarded)
                }
                _ => Err(CadError::InvalidInput(
                    "DeleteAnnotation needs an id payload".into(),
                )),
            },
            CommandId::BeginAnnotationTool => self.begin_annotation_tool(session, &command),
            CommandId::ConfirmAnnotationTool => self.confirm_annotation_tool(session),
            CommandId::CancelAnnotationTool => {
                // Cancelling is always allowed and never opens a transaction.
                session.cancel_tool()?;
                Ok(CommandOutcome::none())
            }
            CommandId::SetAnnotationVisibility => match &command.payload {
                CommandPayload::AnnotationVisibility(id, visible) => {
                    session.set_annotation_visibility(*id, *visible);
                    Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "annotation.visibility".into(),
                            message: format!(
                                "批注 {} 临时{}（不写库）",
                                id.0,
                                if *visible { "显示" } else { "隐藏" }
                            ),
                        }],
                        measurement: None,
                        annotation: None,
                    })
                }
                _ => Err(CadError::InvalidInput(
                    "SetAnnotationVisibility needs a visibility payload".into(),
                )),
            },
            CommandId::SelectAnnotation => match &command.payload {
                CommandPayload::SelectAnnotation(id) => {
                    session.selected_annotation = *id;
                    Ok(CommandOutcome::none())
                }
                _ => Err(CadError::InvalidInput(
                    "SelectAnnotation needs a selection payload".into(),
                )),
            },
            CommandId::Undo => self.undo(&command),
            CommandId::Redo => self.redo(&command),
            CommandId::SwitchProjection => {
                // Explicit toggle of the projection kind, preserving the target.
                // 2D/3D mode is updated to stay consistent with the projection.
                let viewport = self.viewport_mut(&command)?;
                match viewport.camera.projection {
                    Projection::Orthographic { .. } => {
                        viewport.camera.projection =
                            Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
                        let saved = viewport.view_mode.saved_2d_camera();
                        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
                    }
                    Projection::Perspective { .. } => {
                        let saved = viewport.view_mode.saved_2d_camera();
                        viewport.camera = saved;
                        viewport.view_mode = ViewMode2d3d::TwoD { saved };
                    }
                }
                Ok(CommandOutcome::none())
            }
            CommandId::StandardView => {
                if let CommandPayload::StandardView(view) = command.payload {
                    self.apply_standard_view(session, &command, view)?;
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "StandardView needs a view payload".into(),
                    ))
                }
            }
            CommandId::ResetView => {
                let viewport = self.viewport_mut(&command)?;
                let camera = Camera::top_view_2d();
                viewport.camera = camera;
                viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
                viewport.work_plane = xy_work_plane(0.0);
                Ok(CommandOutcome::none())
            }
            CommandId::Pan => self.pan(&command),
            CommandId::Zoom => self.zoom(&command),
            CommandId::OpenDrawing | CommandId::CancelLoading => Err(CadError::Unsupported(
                "file open/cancel is performed by the platform host, not the application".into(),
            )),
            CommandId::ImportAnnotations | CommandId::ExportAnnotations => {
                Err(CadError::Unsupported(
                    "annotation file I/O is performed by the platform host".into(),
                ))
            }
            CommandId::Select => {
                // Selection is read-only: it stores the picked refs and enters
                // the Selecting tool state, but never writes the DWG (F05).
                session.tool = ToolState::Selecting;
                if let CommandPayload::Selection(refs) = command.payload {
                    session.selection = SelectionSet::from_refs(refs);
                }
                Ok(CommandOutcome::none())
            }
            CommandId::Resources => self.resources_report(&command),
            CommandId::Diagnostics => self.diagnostics_report(&command),
            CommandId::SwitchBackend => {
                if let CommandPayload::Backend(choice) = command.payload {
                    session.backend = choice;
                    Ok(CommandOutcome {
                        objects: Vec::new(),
                        changes: None,
                        diagnostics: vec![Diagnostic {
                            object: None,
                            code: "backend.preference".into(),
                            message: format!("后端偏好：{choice:?}（渲染会话由宿主重建）"),
                        }],
                        measurement: None,
                        annotation: None,
                    })
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchBackend needs a backend payload".into(),
                    ))
                }
            }
            CommandId::Switch2d3d => self.switch_2d3d(session, &command),
            CommandId::Orbit => self.orbit(&command),
        }
    }

    /// Toggle between the 2D plan view and a 3D perspective view.
    ///
    /// Entering 3D promotes the projection to perspective and snapshots the 2D
    /// camera; returning to 2D restores that camera exactly (audit F13). This is
    /// pure camera state — no GPU view or depth buffer is involved here.
    fn switch_2d3d(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let viewport = self.viewport_mut(command)?;
        let message = match viewport.view_mode.kind() {
            ProjectionKind::TwoD => {
                let saved = viewport.camera;
                viewport.set_view_mode(ViewMode2d3d::ThreeD { saved_2d: saved })?;
                "已切换到三维视图（透视）"
            }
            ProjectionKind::ThreeD => {
                let saved = viewport.view_mode.saved_2d_camera();
                viewport.set_view_mode(ViewMode2d3d::TwoD { saved })?;
                "已回到二维俯视图"
            }
        };
        session.generation = session.generation.saturating_add(1);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "view.mode".into(),
                message: message.into(),
            }],
            measurement: None,
            annotation: None,
        })
    }

    /// Orbit the current view about its target (spec F13).
    ///
    /// Requires an `Orbit { yaw, pitch }` payload; the camera must be in 3D mode
    /// so an accidental orbit never disturbs the exact 2D view. The near-plane
    /// guard lives in [`Camera::orbit`].
    fn orbit(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Orbit { yaw, pitch } = command.payload else {
            return Err(CadError::InvalidInput(
                "Orbit needs a { yaw, pitch } payload in radians".into(),
            ));
        };
        let viewport = self.viewport_mut(command)?;
        if viewport.view_mode.kind() != ProjectionKind::ThreeD {
            return Err(CadError::InvalidInput(
                "Orbit is only defined in the three-dimensional view".into(),
            ));
        }
        viewport.camera.orbit(yaw, pitch)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "view.orbit".into(),
                message: format!("轨道旋转 yaw={yaw:.4} pitch={pitch:.4}"),
            }],
            measurement: None,
            annotation: None,
        })
    }

    /// Report the resources the open document references (spec F10/F11).
    fn resources_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let keys = if document.resource_keys.is_empty() {
            "无外部资源引用".to_string()
        } else {
            document.resource_keys.join(", ")
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "resources.summary".into(),
                message: format!("资源引用 {}：{keys}", document.resource_keys.len()),
            }],
            measurement: None,
            annotation: None,
        })
    }

    /// Structured diagnostics summary for the UI panel and CLI (spec §19).
    fn diagnostics_report(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document
            .drawing
            .bounds()
            .map(|(min, max)| format!("范围 {:.3},{:.3} → {:.3},{:.3}", min.x, min.y, max.x, max.y))
            .unwrap_or_else(|| "空图纸".to_string());
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![
                Diagnostic {
                    object: None,
                    code: "diagnostics.summary".into(),
                    message: format!(
                        "图元 {}，批注 {}（{}），单位 {:?}",
                        document.drawing.entity_count(),
                        document.annotations.len(),
                        if document.annotations.is_dirty() {
                            "未保存"
                        } else {
                            "已保存"
                        },
                        document.units.source
                    ),
                },
                Diagnostic {
                    object: None,
                    code: "diagnostics.bounds".into(),
                    message: bounds,
                },
            ],
            measurement: None,
            annotation: None,
        })
    }

    fn viewport_mut(&mut self, command: &Command) -> CadResult<&mut Viewport> {
        self.workspace
            .viewports
            .get_mut(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))
    }

    fn fit_drawing(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.fit_viewport(session, &command.viewport)?;
        Ok(CommandOutcome::none())
    }

    /// Fit a viewport to the current document bounds. Hosts call this after an
    /// import so the first frame is usable without a synthetic command.
    ///
    /// Fitting always returns the viewport to the 2D plan view (audit F13), so a
    /// fit is a deterministic reset-and-frame, not a 3D manipulation.
    pub fn fit_viewport(
        &mut self,
        session: &mut SessionState,
        viewport_id: &ViewportId,
    ) -> CadResult<()> {
        let document = self
            .workspace
            .documents
            .get(&session.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let bounds = document.drawing.bounds();
        let viewport = self
            .workspace
            .viewports
            .get_mut(viewport_id)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let Some((min, max)) = bounds else {
            return Err(CadError::InvalidInput(
                "drawing has no measurable extent".into(),
            ));
        };
        let cx = (min.x + max.x) * 0.5;
        let cy = (min.y + max.y) * 0.5;
        let ex = (max.x - min.x).max(1e-6);
        let ey = (max.y - min.y).max(1e-6);
        let w = viewport.logical_size[0].max(1.0);
        let h = viewport.logical_size[1].max(1.0);
        let scale = ((ex / w).max(ey / h) * 1.05).max(camera::MIN_ORTHO_SCALE);
        let camera = Camera {
            eye: Point3 {
                x: cx,
                y: cy,
                z: viewport.camera.eye.z,
            },
            target: Point3 {
                x: cx,
                y: cy,
                z: 0.0,
            },
            up: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            projection: Projection::Orthographic { scale },
        };
        viewport.camera = camera;
        viewport.view_mode = ViewMode2d3d::TwoD { saved: camera };
        session.generation += 1;
        Ok(())
    }

    /// Pan the view by a world-space delta.
    ///
    /// Only the eye and target move; the projection and the orbit frame are
    /// preserved. The 2D plan pan is the special case where `eye` and `target`
    /// share an `x/y`, so this works unchanged in 3D.
    fn pan(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "Pan needs a world-space delta point".into(),
            ));
        };
        let delta = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("Pan delta missing".into()))?;
        if !camera::is_finite_point(delta) {
            return Err(CadError::InvalidInput("Pan delta must be finite".into()));
        }
        let viewport = self.viewport_mut(command)?;
        // The delta is a screen-plane world offset; shift both eye and target so
        // the view direction and distance are unchanged (audit F02 large coords).
        let shift = Point3 {
            x: -delta.x,
            y: -delta.y,
            z: 0.0,
        };
        viewport.camera.target = camera::add(viewport.camera.target, shift);
        viewport.camera.eye = camera::add(viewport.camera.eye, shift);
        // Keep a saved 2D camera in sync while in plan mode.
        if let ViewMode2d3d::TwoD { .. } = viewport.view_mode {
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        }
        Ok(CommandOutcome::none())
    }

    /// Zoom the view.
    ///
    /// Two payloads are accepted for compatibility:
    /// * [`CommandPayload::ZoomAt`] — a factor plus the logical cursor to anchor
    ///   (the real zoom-to-cursor path used by both 2D and 3D).
    /// * [`CommandPayload::Points`] — the legacy factor-only zoom (no cursor),
    ///   retained for direct callers and the CLI.
    fn zoom(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::ZoomAt { factor, cursor } => self.zoom_at(command, *factor, *cursor),
            CommandPayload::Points(points) => {
                let factor = points
                    .first()
                    .map(|p| p.x)
                    .ok_or_else(|| CadError::InvalidInput("Zoom factor missing".into()))?;
                // Centre-of-canvas cursor for the legacy, anchor-free path.
                let size = self.viewport_mut(command)?.logical_size;
                self.zoom_at(command, factor, [size[0] * 0.5, size[1] * 0.5])
            }
            _ => Err(CadError::InvalidInput(
                "Zoom needs a factor or a { factor, cursor } payload".into(),
            )),
        }
    }

    /// Apply a zoom factor anchored at a logical cursor in both projections.
    fn zoom_at(
        &mut self,
        command: &Command,
        factor: f64,
        cursor: [f64; 2],
    ) -> CadResult<CommandOutcome> {
        if factor <= 0.0 || !factor.is_finite() {
            return Err(CadError::InvalidInput(
                "Zoom factor must be positive and finite".into(),
            ));
        }
        let viewport = self.viewport_mut(command)?;
        let size = viewport.logical_size;
        viewport.camera.zoom_at(factor, cursor, size)?;
        if let ViewMode2d3d::TwoD { .. } = viewport.view_mode {
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        }
        Ok(CommandOutcome::none())
    }

    /// Apply a named standard view to the current target.
    ///
    /// The camera is placed at `target + offset * distance`, where `offset` is
    /// the standard view's unit eye direction; the distance is taken from the
    /// current camera (or the drawing's diagonal the first time). The resulting
    /// basis is right-handed and orthonormal ([`Camera::view_basis`]). The plan
    /// (top) view returns the viewport to 2D mode with an orthographic
    /// projection; every other standard view is a 3D view.
    fn apply_standard_view(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        view: StandardView,
    ) -> CadResult<()> {
        let viewport = self.viewport_mut(command)?;
        let target = viewport.camera.target;
        let distance = {
            let d = viewport.camera.distance();
            if d.is_finite() && d > 1e-6 {
                d
            } else {
                1000.0
            }
        };
        let offset = camera::scale3(view.eye_offset(), distance);
        let eye = camera::add(target, offset);
        let camera = Camera {
            eye,
            target,
            up: view.up_hint(),
            projection: viewport.camera.projection,
        };
        camera.validate()?;
        viewport.camera = camera;

        if view.is_plan() {
            // The plan view is the canonical 2D view: force orthographic and keep
            // the current scale, then remember the camera for 2D↔3D round trips.
            let scale = match viewport.camera.projection {
                Projection::Orthographic { scale } => scale,
                Projection::Perspective { .. } => camera::DEFAULT_ORTHO_SCALE,
            };
            viewport.camera.projection = Projection::Orthographic { scale };
            viewport.view_mode = ViewMode2d3d::TwoD {
                saved: viewport.camera,
            };
        } else {
            // A non-plan standard view is a 3D observation.
            if viewport.camera.projection.is_orthographic() {
                viewport.camera.projection =
                    Projection::perspective(camera::DEFAULT_PERSPECTIVE_FOV_RADIANS)?;
            }
            let saved = viewport.view_mode.saved_2d_camera();
            viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        }
        Ok(())
    }

    /// Dispatch the measurement tool (spec F06/U04).
    ///
    /// The algorithm comes from the active tool kind, never from the raw point
    /// count. A stateless `Points` payload with no active tool keeps the old
    /// convenience inference for direct callers and CLI; the interactive tool
    /// path is the one the UI uses.
    fn measure(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::MeasureTool(kind) => {
                session.tool = ToolState::Measuring(MeasurementTool::new(*kind));
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::None => {
                // The shell currently sends no algorithm; default to distance.
                session.tool =
                    ToolState::Measuring(MeasurementTool::new(MeasurementToolKind::Distance));
                Ok(self.measure_preview_outcome(session))
            }
            CommandPayload::Points(points) => self.capture_measure_points(session, command, points),
            _ => Err(CadError::InvalidInput(
                "Measure needs picked points or a measurement tool".into(),
            )),
        }
    }

    /// Capture points for the active tool, or evaluate the stateless fallback.
    fn capture_measure_points(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        let auto_complete = if let ToolState::Measuring(tool) = &mut session.tool {
            for point in points {
                tool.push_point(*point);
            }
            if tool.auto_ready() {
                Some((tool.kind(), tool.points().to_vec()))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((kind, captured)) = auto_complete {
            let outcome = self.evaluate_measurement(command, kind.algorithm(), captured)?;
            // A one-shot tool returns to navigation after a result; open-ended
            // tools stay active until confirmed or cancelled.
            session.tool = ToolState::Idle;
            return Ok(outcome);
        }
        if matches!(session.tool, ToolState::Measuring(_)) {
            return Ok(self.measure_preview_outcome(session));
        }
        // An annotation tool captures through the same point channel so a host
        // only has to map a click once, regardless of which tool is active.
        if matches!(session.tool, ToolState::Annotating(_)) {
            return self.capture_annotation_points(session, points);
        }

        // No active tool: stateless inference retained for direct callers/CLI.
        let algorithm = match points.len() {
            2 => MeasurementAlgorithm::Distance3d,
            3 => MeasurementAlgorithm::Angle3Points,
            n if n >= 4 => MeasurementAlgorithm::PolylineLength,
            _ => {
                return Err(CadError::InvalidInput(
                    "measurement needs two (distance), three (angle) or more (length) points"
                        .into(),
                ))
            }
        };
        self.evaluate_measurement(command, algorithm, points.to_vec())
    }

    /// Confirm an open-ended measurement tool (polyline/area) and evaluate it.
    fn confirm_measurement(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let (kind, points) = match &session.tool {
            ToolState::Measuring(tool) if tool.is_ready() => (tool.kind(), tool.points().to_vec()),
            ToolState::Measuring(tool) => {
                return Err(CadError::InvalidInput(format!(
                    "measurement needs {} more point(s)",
                    tool.remaining()
                )))
            }
            _ => return Err(CadError::InvalidInput("no measurement in progress".into())),
        };
        let outcome = self.evaluate_measurement(command, kind.algorithm(), points)?;
        session.tool = ToolState::Idle;
        Ok(outcome)
    }

    fn measure_preview_outcome(&self, session: &SessionState) -> CommandOutcome {
        let Some(preview) = session.measurement_preview() else {
            return CommandOutcome::none();
        };
        let message = preview.status_line();
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.preview".into(),
                message,
            }],
            measurement: None,
            annotation: None,
        }
    }

    /// Evaluate a measurement through the engine and return the structured
    /// record. Area uses the viewport work plane so non-coplanar input is
    /// rejected rather than silently flattened (audit B24).
    fn evaluate_measurement(
        &self,
        command: &Command,
        algorithm: MeasurementAlgorithm,
        points: Vec<Point3>,
    ) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let space = match algorithm {
            // Planar algorithms (2D distance, polyline length and area) are
            // only defined on an explicit work plane; the viewport supplies it.
            MeasurementAlgorithm::Distance2d
            | MeasurementAlgorithm::PolylineLength
            | MeasurementAlgorithm::PlanarPolygonArea => {
                let viewport = self
                    .workspace
                    .viewports
                    .get(&command.viewport)
                    .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
                MeasurementSpace::Plane(viewport.work_plane)
            }
            // 3D distance and angle are spatial model measurements.
            MeasurementAlgorithm::Distance3d | MeasurementAlgorithm::Angle3Points => {
                MeasurementSpace::World3d
            }
        };
        let request = MeasurementRequest {
            algorithm,
            points,
            tapped: Vec::new(),
            space,
            units: document.units.clone(),
            source: GeometrySource::UserPoints,
            precision: Precision::Analytic,
        };
        let record = self.measurement.measure(&request)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.result".into(),
                message: format!("{:?}: {:.6}", record.algorithm, record.value),
            }],
            measurement: Some(record),
            annotation: None,
        })
    }

    fn annotation_command(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let payload = match &command.payload {
            CommandPayload::Annotation(annotation_command) => annotation_command.as_ref(),
            _ => {
                return Err(CadError::InvalidInput(
                    "annotation command needs an annotation payload".into(),
                ))
            }
        };
        self.commit_annotation(document_id, payload)
    }

    /// Start (or restart) an annotation creation tool (spec F07/U04).
    fn begin_annotation_tool(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::AnnotationTool(kind) = &command.payload else {
            return Err(CadError::InvalidInput(
                "BeginAnnotationTool needs an annotation kind payload".into(),
            ));
        };
        session.tool = ToolState::Annotating(AnnotationTool::new(*kind));
        Ok(self.annotation_preview_outcome(session))
    }

    /// Capture world points / text into the active annotation tool.
    ///
    /// Fixed-point kinds capture the exact complement and then return; open
    /// ended kinds accumulate until an explicit confirm. A stray point with no
    /// active tool is refused rather than silently ignored.
    fn capture_annotation_points(
        &mut self,
        session: &mut SessionState,
        points: &[Point3],
    ) -> CadResult<CommandOutcome> {
        let outcome = match &mut session.tool {
            ToolState::Annotating(tool) => {
                for point in points {
                    tool.push_point(*point)?;
                }
                if tool.auto_ready() {
                    Some(self.build_annotation_outcome(session)?)
                } else {
                    None
                }
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "no annotation tool is active".into(),
                ))
            }
        };
        if let Some(outcome) = outcome {
            // A one-shot tool returns to navigation after committing.
            session.tool = ToolState::Idle;
            return Ok(outcome);
        }
        Ok(self.annotation_preview_outcome(session))
    }

    /// Confirm an annotation tool: build and commit exactly one transaction.
    ///
    /// The tool is cleared only after the transaction has been applied, so a
    /// rejected commit leaves the captured parameters in place for the user to
    /// fix (mirroring the measurement tool).
    fn confirm_annotation_tool(&mut self, session: &mut SessionState) -> CadResult<CommandOutcome> {
        let outcome = self.build_annotation_outcome(session)?;
        session.tool = ToolState::Idle;
        Ok(outcome)
    }

    /// Build and commit the annotation described by the active tool.
    fn build_annotation_outcome(&mut self, session: &SessionState) -> CadResult<CommandOutcome> {
        let (annotation, document_id) = {
            let tool = match &session.tool {
                ToolState::Annotating(tool) => tool,
                _ => {
                    return Err(CadError::InvalidInput(
                        "no annotation tool is active".into(),
                    ))
                }
            };
            let id = self.next_annotation_id(session.document);
            let annotation = tool.build(
                id,
                session.active_space.clone(),
                0,
                0,
                AnnotationStyle::default(),
            )?;
            (annotation, session.document)
        };
        let command = AnnotationCommand::Create(annotation);
        self.commit_annotation(document_id, &command)
    }

    /// Allocate the next annotation id in the open document's id space.
    ///
    /// Deterministic and monotonic: one past the highest existing id, so a
    /// delete never lets a later create reuse an id (which would make a stale
    /// visibility override or selection point at the wrong annotation). A real
    /// host may prefer UUIDs, but id allocation is not the tool's concern and a
    /// stable sequence keeps the command path reproducible for tests and the CLI.
    fn next_annotation_id(&self, document: DocumentId) -> AnnotationId {
        let next = self
            .workspace
            .documents
            .get(&document)
            .map(|d| {
                d.annotations
                    .annotations()
                    .map(|a| a.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1
            })
            .unwrap_or(1);
        AnnotationId(next)
    }

    fn annotation_preview_outcome(&self, session: &SessionState) -> CommandOutcome {
        let Some(preview) = session.annotation_preview() else {
            return CommandOutcome::none();
        };
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "annotation.preview".into(),
                message: preview.status_line(),
            }],
            measurement: None,
            annotation: None,
        }
    }

    /// Commit one annotation command through the shared history path.
    ///
    /// This is the single write entry used both by the tool confirm and by the
    /// management UI's delete/edit actions, so there is exactly one transaction
    /// and one undo record per user action. Any failure leaves the database,
    /// its revision and history untouched (the service rolls the transaction
    /// back before this function records anything).
    fn commit_annotation(
        &mut self,
        document_id: DocumentId,
        command: &AnnotationCommand,
    ) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // Build the undo patch from the before/after state.
        let (patch, label) = match command {
            AnnotationCommand::Create(a) => {
                (patch(a.id, None, Some(a.clone())), "create annotation")
            }
            AnnotationCommand::Update(a) => {
                let before = document.annotations.get(a.id).cloned();
                (patch(a.id, before, Some(a.clone())), "update annotation")
            }
            AnnotationCommand::Delete(id) => {
                let before = document.annotations.get(*id).cloned();
                (patch(*id, before, None), "delete annotation")
            }
        };
        let annotation = match command {
            AnnotationCommand::Create(a) | AnnotationCommand::Update(a) => a.id,
            AnnotationCommand::Delete(id) => *id,
        };
        let inner = match command {
            AnnotationCommand::Create(a) => AnnotationCommand::Create(a.clone()),
            AnnotationCommand::Update(a) => AnnotationCommand::Update(a.clone()),
            AnnotationCommand::Delete(id) => AnnotationCommand::Delete(*id),
        };
        let changes = self.annotations.apply(&mut document.annotations, inner)?;

        let history = self.history.entry(document_id).or_default();
        let transaction = changes.transaction;
        history.record(UndoRecord {
            transaction,
            label: label.to_string(),
            patches: vec![patch],
            merge_key: None,
        })?;

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: Some(annotation),
        })
    }

    fn undo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(command.document).or_default();
        let changes = history.undo(&mut document.annotations)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: None,
        })
    }

    fn redo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(command.document).or_default();
        let changes = history.redo(&mut document.annotations)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: None,
        })
    }

    pub fn can_undo(&self, document: &DocumentId) -> bool {
        self.history
            .get(document)
            .map(|h| h.can_undo())
            .unwrap_or(false)
    }

    /// Whether the document has a redo record. Derived from the redo stack, not
    /// from `can_undo` (audit U11): after undoing to empty, redo stays available.
    pub fn can_redo(&self, document: &DocumentId) -> bool {
        self.history
            .get(document)
            .map(|h| h.can_redo())
            .unwrap_or(false)
    }

    /// Pure snapshot of undo/redo availability. UI refreshes call this instead
    /// of dispatching a command, so no duplicate command is emitted.
    pub fn history_availability(&self, document: &DocumentId) -> HistoryAvailability {
        HistoryAvailability {
            can_undo: self.can_undo(document),
            can_redo: self.can_redo(document),
        }
    }

    /// Guard a document switch/exit; an unsaved annotation is never discarded
    /// without an explicit user decision (spec §16.3).
    ///
    /// This is the decision-only half of the leave flow: the host owns the real
    /// save/recovery writes, so `Save`/`PreserveRecovery` are treated as
    /// "recorded intent" here and the host confirms the durable write through
    /// [`Application::resolve_leave`] with the actual write results.
    pub fn prepare_leave(
        &mut self,
        document: DocumentId,
        decision: UnsavedDecision,
    ) -> CadResult<()> {
        match self.resolve_leave(document, decision, true, true) {
            UnsavedOutcome::Proceed => Ok(()),
            UnsavedOutcome::Cancelled => Err(CadError::Cancelled),
            UnsavedOutcome::SaveFailed => Err(CadError::Unsupported(
                "annotation save must be confirmed by the platform host".into(),
            )),
            UnsavedOutcome::RecoveryFailed => Err(CadError::Invariant(
                "recovery snapshot could not be persisted".into(),
            )),
        }
    }

    /// Apply the unsaved-work decision model and return its structured outcome.
    ///
    /// `save_succeeded` / `recovery_succeeded` are the host's results for the
    /// real atomic export and recovery write. A failed save never proceeds and
    /// never clears dirty (audit B07); a `Cancel` keeps the current document and
    /// any recovery data (audit U09). This method performs no writes itself.
    pub fn resolve_leave(
        &self,
        document: DocumentId,
        decision: UnsavedDecision,
        save_succeeded: bool,
        recovery_succeeded: bool,
    ) -> UnsavedOutcome {
        let dirty = self
            .workspace
            .documents
            .get(&document)
            .map(|d| d.annotations.is_dirty())
            .unwrap_or(false);
        UnsavedFlow::new(dirty).apply(decision, save_succeeded, recovery_succeeded)
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

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{Annotation, AnnotationGeometry, AnnotationStyle, DrawingDatabaseBuilder};

    // Aliases kept local so the tests read without re-importing camera helpers.
    fn cam_xy_work_plane(z: f64) -> WorkPlane {
        xy_work_plane(z)
    }
    fn cad_app_len(v: Point3) -> f64 {
        camera::length3(v)
    }
    fn cad_app_dot(a: Point3, b: Point3) -> f64 {
        camera::dot3(a, b)
    }

    fn application_with_document() -> (Application, SessionState) {
        let mut app = Application::new();
        let document_id = DocumentId(1);
        let drawing = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
        app.workspace.documents.insert(
            document_id,
            Document {
                id: document_id,
                drawing: Arc::new(drawing),
                annotations: AnnotationDatabase::new(DatabaseId(2)),
                identity: DocumentIdentity::Sha256([0u8; 32]),
                units: UnitContext::drawing_units(),
                resource_keys: Vec::new(),
            },
        );
        app.workspace.viewports.insert(
            ViewportId(1),
            Viewport::new(ViewportId(1), document_id, [800.0, 600.0]),
        );
        let session = SessionState::new(document_id, AppMode::Work);
        (app, session)
    }

    fn command(id: CommandId, payload: CommandPayload) -> Command {
        Command {
            schema_version: 1,
            id,
            document: DocumentId(1),
            viewport: ViewportId(1),
            payload,
        }
    }

    fn ann(id: u128) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            text: "note".into(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    #[test]
    fn viewer_mode_rejects_annotation_commands_at_the_command_layer() {
        let (mut app, mut session) = application_with_document();
        let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
        let cmd = command(
            CommandId::CreateAnnotation,
            CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(1)))),
        );
        assert_eq!(
            app.execute(&mut viewer, cmd).unwrap_err(),
            CadError::PermissionDenied
        );
        // Work mode succeeds.
        let cmd = command(
            CommandId::CreateAnnotation,
            CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(1)))),
        );
        app.execute(&mut session, cmd).unwrap();
    }

    #[test]
    fn create_then_undo_then_redo_through_the_application() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::CreateAnnotation,
                CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(5)))),
            ),
        )
        .unwrap();
        assert_eq!(
            app.workspace
                .documents
                .get(&DocumentId(1))
                .unwrap()
                .annotations
                .len(),
            1
        );
        app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
            .unwrap();
        assert_eq!(
            app.workspace
                .documents
                .get(&DocumentId(1))
                .unwrap()
                .annotations
                .len(),
            0
        );
        app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
            .unwrap();
        assert_eq!(
            app.workspace
                .documents
                .get(&DocumentId(1))
                .unwrap()
                .annotations
                .len(),
            1
        );
    }

    #[test]
    fn stale_document_command_is_rejected() {
        let (mut app, mut session) = application_with_document();
        let mut cmd = command(CommandId::FitDrawing, CommandPayload::None);
        cmd.document = DocumentId(99);
        assert_eq!(
            app.execute(&mut session, cmd).unwrap_err(),
            CadError::StaleResult
        );
    }

    #[test]
    fn measure_distance_is_reported() {
        let (mut app, mut session) = application_with_document();
        let cmd = command(
            CommandId::Measure,
            CommandPayload::Points(vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 3.0,
                    y: 4.0,
                    z: 0.0,
                },
            ]),
        );
        let outcome = app.execute(&mut session, cmd).unwrap();
        assert!(outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "measure.result"));
    }

    #[test]
    fn layer_override_does_not_touch_the_database() {
        let (mut app, mut session) = application_with_document();
        let before = app
            .workspace
            .documents
            .get(&DocumentId(1))
            .unwrap()
            .drawing
            .revision();
        app.execute(
            &mut session,
            command(
                CommandId::ToggleLayer,
                CommandPayload::Layer(LayerId(3), false),
            ),
        )
        .unwrap();
        assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
        assert_eq!(
            app.workspace
                .documents
                .get(&DocumentId(1))
                .unwrap()
                .drawing
                .revision(),
            before
        );
    }

    #[test]
    fn restore_layers_clears_overrides_without_touching_the_database() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::ToggleLayer,
                CommandPayload::Layer(LayerId(0), false),
            ),
        )
        .unwrap();
        assert!(session.layer_overrides.clear_changed());

        let before = app
            .workspace
            .documents
            .get(&DocumentId(1))
            .unwrap()
            .drawing
            .revision();
        app.execute(
            &mut session,
            command(CommandId::RestoreLayers, CommandPayload::None),
        )
        .unwrap();
        assert!(session.layer_overrides.is_empty());
        assert_eq!(
            app.workspace
                .documents
                .get(&DocumentId(1))
                .unwrap()
                .drawing
                .revision(),
            before
        );
    }

    #[test]
    fn select_command_records_selection_and_does_not_mutate_the_drawing() {
        let (mut app, mut session) = application_with_document();
        let revision_before = app.workspace.documents[&DocumentId(1)].drawing.revision();
        // No entity exists in the empty database, but selection is still a pure
        // state update: refs are recorded and the drawing is untouched.
        let refs = vec![SelectionRef {
            document: DocumentId(1),
            entity: EntityId(7),
            instance: InstancePath::default(),
            sub_element: None,
        }];
        app.execute(
            &mut session,
            command(CommandId::Select, CommandPayload::Selection(refs)),
        )
        .unwrap();
        assert_eq!(session.selection.len(), 1);
        assert!(matches!(session.tool, ToolState::Selecting));
        assert_eq!(
            app.workspace.documents[&DocumentId(1)].drawing.revision(),
            revision_before
        );
        assert!(!app.can_undo(&DocumentId(1)));
    }

    #[test]
    fn select_with_empty_payload_clears_the_selection() {
        let (mut app, mut session) = application_with_document();
        session.selection.replace([SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        }]);
        assert_eq!(session.selection.len(), 1);
        app.execute(
            &mut session,
            command(CommandId::Select, CommandPayload::Selection(Vec::new())),
        )
        .unwrap();
        assert!(session.selection.is_empty());
    }

    #[test]
    fn leave_with_unsaved_annotations_requires_a_decision() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::CreateAnnotation,
                CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(1)))),
            ),
        )
        .unwrap();
        assert!(matches!(
            app.prepare_leave(DocumentId(1), UnsavedDecision::Cancel),
            Err(CadError::Cancelled)
        ));
        assert!(app
            .prepare_leave(DocumentId(1), UnsavedDecision::Discard)
            .is_ok());
    }

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn measure_tool_selects_the_algorithm_from_the_active_kind() {
        let (mut app, mut session) = application_with_document();
        // Three points would be inferred as an angle by the stateless path, but
        // an explicit polyline tool must measure length (audit F06).
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::MeasureTool(MeasurementToolKind::PolylineLength),
            ),
        )
        .unwrap();
        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::Measure,
                    CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 0.0), point(3.0, 4.0)]),
                ),
            )
            .unwrap();
        // Open-ended tools capture but do not auto-complete.
        assert!(outcome.measurement.is_none());
        assert_eq!(
            session.measurement_preview().map(|p| p.kind),
            Some(MeasurementToolKind::PolylineLength)
        );
        let confirmed = app
            .execute(
                &mut session,
                command(CommandId::ConfirmMeasurement, CommandPayload::None),
            )
            .unwrap();
        let record = confirmed.measurement.expect("structured record");
        assert_eq!(record.algorithm, MeasurementAlgorithm::PolylineLength);
        assert!((record.value - 7.0).abs() < 1e-9);
        assert!(matches!(session.tool, ToolState::Idle));
    }

    #[test]
    fn measure_tool_captures_points_across_commands_and_auto_completes() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::MeasureTool(MeasurementToolKind::Distance),
            ),
        )
        .unwrap();
        let first = app
            .execute(
                &mut session,
                command(
                    CommandId::Measure,
                    CommandPayload::Points(vec![point(0.0, 0.0)]),
                ),
            )
            .unwrap();
        assert!(first.measurement.is_none());
        let preview = session.measurement_preview().expect("preview");
        assert_eq!(preview.points.len(), 1);
        assert_eq!(preview.remaining, 1);
        assert!(!preview.ready);

        let second = app
            .execute(
                &mut session,
                command(
                    CommandId::Measure,
                    CommandPayload::Points(vec![point(3.0, 4.0)]),
                ),
            )
            .unwrap();
        let record = second.measurement.expect("auto-completed record");
        assert_eq!(record.algorithm, MeasurementAlgorithm::Distance3d);
        assert!((record.value - 5.0).abs() < 1e-9);
        assert!(session.measurement_preview().is_none());
        assert!(matches!(session.tool, ToolState::Idle));
    }

    #[test]
    fn measure_none_starts_a_distance_tool_without_capturing() {
        let (mut app, mut session) = application_with_document();
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::Measure, CommandPayload::None),
            )
            .unwrap();
        assert!(outcome.measurement.is_none());
        let preview = session.measurement_preview().expect("preview");
        assert_eq!(preview.kind, MeasurementToolKind::Distance);
        assert!(preview.points.is_empty());
    }

    #[test]
    fn measure_area_uses_the_viewport_work_plane() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::MeasureTool(MeasurementToolKind::Area),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![
                    point(0.0, 0.0),
                    point(2.0, 0.0),
                    point(2.0, 3.0),
                    point(0.0, 3.0),
                ]),
            ),
        )
        .unwrap();
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::ConfirmMeasurement, CommandPayload::None),
            )
            .unwrap();
        let record = outcome.measurement.expect("area record");
        assert_eq!(record.algorithm, MeasurementAlgorithm::PlanarPolygonArea);
        assert!((record.value - 6.0).abs() < 1e-9);
        assert!(record.plane.is_some());
    }

    #[test]
    fn cancelled_measurement_produces_zero_transactions() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::MeasureTool(MeasurementToolKind::Angle),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(1.0, 0.0)]),
            ),
        )
        .unwrap();
        let revision_before = app.workspace.documents[&DocumentId(1)]
            .annotations
            .revision();
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::CancelMeasurement, CommandPayload::None),
            )
            .unwrap();
        assert!(outcome.changes.is_none());
        assert!(matches!(session.tool, ToolState::Idle));
        assert!(session.measurement_preview().is_none());
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .annotations
                .revision(),
            revision_before
        );
        assert!(!app.can_undo(&DocumentId(1)));
    }

    #[test]
    fn confirm_before_enough_points_is_rejected_without_changes() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::MeasureTool(MeasurementToolKind::Area),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::Measure,
                CommandPayload::Points(vec![point(0.0, 0.0)]),
            ),
        )
        .unwrap();
        assert!(matches!(
            app.execute(
                &mut session,
                command(CommandId::ConfirmMeasurement, CommandPayload::None),
            ),
            Err(CadError::InvalidInput(_))
        ));
        // The in-progress tool and its captured point survive the rejection.
        assert_eq!(
            session.measurement_preview().map(|p| p.points.len()),
            Some(1)
        );
    }

    #[test]
    fn undo_to_empty_still_allows_redo_and_refresh_is_pure() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::CreateAnnotation,
                CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(7)))),
            ),
        )
        .unwrap();
        assert_eq!(
            app.history_availability(&DocumentId(1)),
            HistoryAvailability {
                can_undo: true,
                can_redo: false
            }
        );

        app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
            .unwrap();
        // Undo to empty: undo disabled, but redo must stay available (U11).
        let availability = app.history_availability(&DocumentId(1));
        assert!(!availability.can_undo);
        assert!(availability.can_redo);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(app.can_redo(&DocumentId(1)));

        // A pure refresh repeats the same snapshot and changes nothing.
        let annotation_count = app.workspace.documents[&DocumentId(1)].annotations.len();
        assert_eq!(
            app.history_availability(&DocumentId(1)),
            app.history_availability(&DocumentId(1))
        );
        assert_eq!(
            app.workspace.documents[&DocumentId(1)].annotations.len(),
            annotation_count
        );

        app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
            .unwrap();
        let availability = app.history_availability(&DocumentId(1));
        assert!(availability.can_undo);
        assert!(!availability.can_redo);
    }

    #[test]
    fn screen_center_maps_to_camera_target_and_corners_are_symmetric() {
        let mut viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
        viewport.camera.target = Point3 {
            x: 100.0,
            y: 50.0,
            z: 0.0,
        };
        viewport.camera.projection = Projection::Orthographic { scale: 2.0 };

        let center = viewport.screen_to_world([400.0, 300.0], [800.0, 600.0]);
        assert_eq!(
            center,
            Some(Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0
            })
        );

        // Top-left is left of and above the centre; world y flips relative to
        // the downward-growing screen y.
        let top_left = viewport
            .screen_to_world([0.0, 0.0], [800.0, 600.0])
            .unwrap();
        assert!((top_left.x - (100.0 - 400.0 * 2.0)).abs() < 1e-9);
        assert!((top_left.y - (50.0 + 300.0 * 2.0)).abs() < 1e-9);
    }

    #[test]
    fn screen_to_world_rejects_degenerate_input() {
        let viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
        assert!(viewport
            .screen_to_world([f64::NAN, 0.0], [800.0, 600.0])
            .is_none());
        assert!(viewport.screen_to_world([0.0, 0.0], [0.0, 600.0]).is_none());
        assert!(viewport
            .screen_to_world([0.0, 0.0], [800.0, -1.0])
            .is_none());
    }

    #[test]
    fn switch_2d3d_and_orbit_are_real_transitions() {
        let (mut app, mut session) = application_with_document();
        // Start in 2D orthographic.
        let before = app.workspace.viewports[&ViewportId(1)].camera;
        assert!(before.projection.is_orthographic());

        // Switch to 3D: perspective, still 3D mode.
        app.execute(
            &mut session,
            command(CommandId::Switch2d3d, CommandPayload::None),
        )
        .unwrap();
        let vp = &app.workspace.viewports[&ViewportId(1)];
        assert!(!vp.camera.projection.is_orthographic());
        assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);

        // Orbit in 3D succeeds and preserves the eye-target distance.
        let d = app.workspace.viewports[&ViewportId(1)].camera.distance();
        app.execute(
            &mut session,
            command(
                CommandId::Orbit,
                CommandPayload::Orbit {
                    yaw: 0.3,
                    pitch: 0.2,
                },
            ),
        )
        .unwrap();
        let after_orbit = app.workspace.viewports[&ViewportId(1)].camera.distance();
        assert!((after_orbit - d).abs() < 1e-9);

        // Switch back restores the exact 2D camera.
        app.execute(
            &mut session,
            command(CommandId::Switch2d3d, CommandPayload::None),
        )
        .unwrap();
        let vp = &app.workspace.viewports[&ViewportId(1)];
        assert_eq!(vp.camera, before);
        assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    }

    #[test]
    fn orbit_outside_3d_is_rejected_without_changing_the_camera() {
        let (mut app, mut session) = application_with_document();
        let before = app.workspace.viewports[&ViewportId(1)].camera;
        let result = app.execute(
            &mut session,
            command(
                CommandId::Orbit,
                CommandPayload::Orbit {
                    yaw: 0.1,
                    pitch: 0.1,
                },
            ),
        );
        assert!(matches!(result, Err(CadError::InvalidInput(_))));
        assert_eq!(app.workspace.viewports[&ViewportId(1)].camera, before);
    }

    #[test]
    fn orbit_without_payload_is_rejected() {
        let (mut app, mut session) = application_with_document();
        assert!(matches!(
            app.execute(
                &mut session,
                command(CommandId::Orbit, CommandPayload::None)
            ),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn standard_view_is_a_real_projection_and_plan_returns_to_2d() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::StandardView,
                CommandPayload::StandardView(StandardView::Front),
            ),
        )
        .unwrap();
        let vp = &app.workspace.viewports[&ViewportId(1)];
        assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
        let basis = vp.camera.view_basis().unwrap();
        assert!(basis.is_orthonormal(1e-9));

        // Top (plan) returns to 2D orthographic.
        app.execute(
            &mut session,
            command(
                CommandId::StandardView,
                CommandPayload::StandardView(StandardView::Top),
            ),
        )
        .unwrap();
        let vp = &app.workspace.viewports[&ViewportId(1)];
        assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
        assert!(vp.camera.projection.is_orthographic());
    }

    #[test]
    fn zoom_at_anchors_the_cursor_in_2d() {
        let (mut app, mut session) = application_with_document();
        let cursor = [200.0, 150.0];
        let size = [800.0, 600.0];
        let world_before = app.workspace.viewports[&ViewportId(1)]
            .camera
            .screen_to_plan_world(cursor, size)
            .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::Zoom,
                CommandPayload::ZoomAt {
                    factor: 2.0,
                    cursor,
                },
            ),
        )
        .unwrap();
        let world_after = app.workspace.viewports[&ViewportId(1)]
            .camera
            .screen_to_plan_world(cursor, size)
            .unwrap();
        assert!((world_before.x - world_after.x).abs() < 1e-9);
        assert!((world_before.y - world_after.y).abs() < 1e-9);
        // The scale actually changed.
        assert!(app.workspace.viewports[&ViewportId(1)].world_per_px() < 1.0);
    }

    #[test]
    fn work_plane_is_right_handed_and_orthonormal() {
        let plane = cam_xy_work_plane(0.0);
        assert!((cad_app_len(plane.u) - 1.0).abs() < 1e-12);
        assert!((cad_app_len(plane.v) - 1.0).abs() < 1e-12);
        assert!(cad_app_dot(plane.u, plane.v).abs() < 1e-12);
        let n = camera::cross(plane.u, plane.v);
        assert!(n.z > 0.0, "normal points +Z in the plan view");
    }

    // --- F07/F08/F09 annotation tool + management ---------------

    fn annotation_count(app: &Application) -> usize {
        app.workspace.documents[&DocumentId(1)].annotations.len()
    }

    #[test]
    fn begin_annotation_tool_previews_without_committing() {
        let (mut app, mut session) = application_with_document();
        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::BeginAnnotationTool,
                    CommandPayload::AnnotationTool(AnnotationToolKind::Rectangle),
                ),
            )
            .unwrap();
        assert!(outcome.changes.is_none());
        assert!(outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "annotation.preview"));
        let preview = session.annotation_preview().expect("active preview");
        assert_eq!(preview.kind, AnnotationToolKind::Rectangle);
        assert_eq!(preview.remaining, 2);
        assert_eq!(annotation_count(&app), 0);
        assert!(!app.can_undo(&DocumentId(1)));
    }

    #[test]
    fn rectangle_tool_auto_commits_exactly_one_transaction() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Rectangle),
            ),
        )
        .unwrap();
        let first = app
            .execute(
                &mut session,
                command(
                    CommandId::AppendAnnotationPoints,
                    CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0)]),
                ),
            )
            .unwrap();
        assert!(first.changes.is_none(), "not committed until complete");
        assert!(session.annotation_preview().is_some());

        let second = app
            .execute(
                &mut session,
                command(
                    CommandId::AppendAnnotationPoints,
                    CommandPayload::AppendAnnotationPoints(vec![point(4.0, 3.0)]),
                ),
            )
            .unwrap();
        // Completing the rectangle commits exactly one transaction.
        assert_eq!(second.changes.as_ref().map(|c| c.changes.len()), Some(1));
        assert_eq!(second.annotation, Some(AnnotationId(1)));
        assert!(matches!(session.tool, ToolState::Idle));
        assert_eq!(annotation_count(&app), 1);
        assert!(app.can_undo(&DocumentId(1)));
        assert!(!app.can_redo(&DocumentId(1)));
    }

    #[test]
    fn text_tool_requires_text_before_confirm() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Text),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::AppendAnnotationPoints,
                CommandPayload::AppendAnnotationPoints(vec![point(1.0, 2.0)]),
            ),
        )
        .unwrap();
        // Missing text: confirm is rejected and the tool survives.
        assert!(matches!(
            app.execute(
                &mut session,
                command(CommandId::ConfirmAnnotationTool, CommandPayload::None),
            ),
            Err(CadError::InvalidInput(_))
        ));
        assert!(session.annotation_preview().is_some());
        assert_eq!(annotation_count(&app), 0);

        app.execute(
            &mut session,
            command(
                CommandId::AnnotationText,
                CommandPayload::AnnotationText("检查批注".into()),
            ),
        )
        .unwrap();
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::ConfirmAnnotationTool, CommandPayload::None),
            )
            .unwrap();
        assert_eq!(outcome.changes.as_ref().map(|c| c.changes.len()), Some(1));
        assert_eq!(annotation_count(&app), 1);
        let stored = app.workspace.documents[&DocumentId(1)]
            .annotations
            .get(AnnotationId(1))
            .unwrap();
        assert_eq!(stored.text, "检查批注");
    }

    #[test]
    fn cancelling_an_annotation_tool_produces_zero_transactions() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Freehand),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::AppendAnnotationPoints,
                CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0), point(1.0, 1.0)]),
            ),
        )
        .unwrap();
        let revision_before = app.workspace.documents[&DocumentId(1)]
            .annotations
            .revision();
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::CancelAnnotationTool, CommandPayload::None),
            )
            .unwrap();
        assert!(outcome.changes.is_none());
        assert!(session.annotation_preview().is_none());
        assert!(matches!(session.tool, ToolState::Idle));
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .annotations
                .revision(),
            revision_before
        );
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(!app.can_redo(&DocumentId(1)));
    }

    #[test]
    fn freehand_needs_an_explicit_confirm_and_commits_once() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Freehand),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::AppendAnnotationPoints,
                CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0), point(1.0, 0.0)]),
            ),
        )
        .unwrap();
        // Open-ended: no auto-commit even when the minimum is met.
        assert!(session.annotation_preview().is_some());
        assert_eq!(annotation_count(&app), 0);
        let outcome = app
            .execute(
                &mut session,
                command(CommandId::ConfirmAnnotationTool, CommandPayload::None),
            )
            .unwrap();
        assert_eq!(outcome.changes.as_ref().map(|c| c.changes.len()), Some(1));
        assert_eq!(outcome.annotation, Some(AnnotationId(1)));
        assert_eq!(annotation_count(&app), 1);
    }

    #[test]
    fn annotation_points_without_an_active_tool_are_refused() {
        let (mut app, mut session) = application_with_document();
        assert!(matches!(
            app.execute(
                &mut session,
                command(
                    CommandId::AppendAnnotationPoints,
                    CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0)]),
                ),
            ),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(annotation_count(&app), 0);
    }

    #[test]
    fn confirm_without_an_active_tool_produces_no_transaction() {
        let (mut app, mut session) = application_with_document();
        assert!(matches!(
            app.execute(
                &mut session,
                command(CommandId::ConfirmAnnotationTool, CommandPayload::None),
            ),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(annotation_count(&app), 0);
        assert!(!app.can_undo(&DocumentId(1)));
    }

    #[test]
    fn ellipse_tool_reads_center_and_axes_from_two_points() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Ellipse),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::AppendAnnotationPoints,
                CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0), point(2.0, 1.0)]),
            ),
        )
        .unwrap();
        let stored = app.workspace.documents[&DocumentId(1)]
            .annotations
            .get(AnnotationId(1))
            .unwrap();
        match &stored.geometry {
            cad_db::AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => {
                assert_eq!(*center, point(0.0, 0.0));
                assert_eq!(*axis_u, point(2.0, 0.0));
                assert_eq!(*axis_v, point(0.0, 1.0));
            }
            other => panic!("expected ellipse, got {other:?}"),
        }
    }

    #[test]
    fn annotation_visibility_round_trips_without_a_transaction() {
        let (mut app, mut session) = application_with_document();
        // Seed one annotation directly through the command path.
        app.execute(
            &mut session,
            command(
                CommandId::CreateAnnotation,
                CommandPayload::Annotation(Box::new(AnnotationCommand::Create(ann(9)))),
            ),
        )
        .unwrap();
        let revision_before = app.workspace.documents[&DocumentId(1)]
            .annotations
            .revision();

        let outcome = app
            .execute(
                &mut session,
                command(
                    CommandId::SetAnnotationVisibility,
                    CommandPayload::AnnotationVisibility(AnnotationId(9), false),
                ),
            )
            .unwrap();
        assert!(outcome.changes.is_none(), "visibility writes nothing");
        assert!(session.annotation_hidden(AnnotationId(9)));
        let rows = session.annotation_rows(&app.workspace.documents[&DocumentId(1)].annotations);
        assert!(!rows[0].visible);
        assert!(rows[0].is_overridden());
        // The database revision and history are untouched.
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .annotations
                .revision(),
            revision_before
        );
        assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

        // Show again: state round-trips back to visible.
        app.execute(
            &mut session,
            command(
                CommandId::SetAnnotationVisibility,
                CommandPayload::AnnotationVisibility(AnnotationId(9), true),
            ),
        )
        .unwrap();
        assert!(session.annotation_visible(AnnotationId(9)));
    }

    #[test]
    fn selecting_an_annotation_is_read_only_state() {
        let (mut app, mut session) = application_with_document();
        app.execute(
            &mut session,
            command(
                CommandId::SelectAnnotation,
                CommandPayload::SelectAnnotation(Some(AnnotationId(4))),
            ),
        )
        .unwrap();
        assert_eq!(session.selected_annotation, Some(AnnotationId(4)));
        assert!(!app.can_undo(&DocumentId(1)));
        app.execute(
            &mut session,
            command(
                CommandId::SelectAnnotation,
                CommandPayload::SelectAnnotation(None),
            ),
        )
        .unwrap();
        assert_eq!(session.selected_annotation, None);
    }

    #[test]
    fn annotation_transactions_drive_undo_and_redo_availability() {
        let (mut app, mut session) = application_with_document();
        // One confirmed annotation tool = one undoable transaction.
        app.execute(
            &mut session,
            command(
                CommandId::BeginAnnotationTool,
                CommandPayload::AnnotationTool(AnnotationToolKind::Rectangle),
            ),
        )
        .unwrap();
        app.execute(
            &mut session,
            command(
                CommandId::AppendAnnotationPoints,
                CommandPayload::AppendAnnotationPoints(vec![point(0.0, 0.0), point(1.0, 1.0)]),
            ),
        )
        .unwrap();
        assert!(app.can_undo(&DocumentId(1)));
        assert!(!app.can_redo(&DocumentId(1)));

        // Undo removes it; redo stays available (U11).
        app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
            .unwrap();
        assert_eq!(annotation_count(&app), 0);
        assert!(!app.can_undo(&DocumentId(1)));
        assert!(app.can_redo(&DocumentId(1)));

        // Redo restores it.
        app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
            .unwrap();
        assert_eq!(annotation_count(&app), 1);
        assert!(app.can_undo(&DocumentId(1)));
        assert!(!app.can_redo(&DocumentId(1)));
    }

    #[test]
    fn viewer_mode_rejects_annotation_tool_commands() {
        let (mut app, _) = application_with_document();
        let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
        for id in [
            CommandId::BeginAnnotationTool,
            CommandId::ConfirmAnnotationTool,
            CommandId::CancelAnnotationTool,
            CommandId::AppendAnnotationPoints,
            CommandId::AnnotationText,
            CommandId::SetAnnotationVisibility,
            CommandId::SelectAnnotation,
        ] {
            assert_eq!(
                app.execute(&mut viewer, command(id, CommandPayload::None))
                    .unwrap_err(),
                CadError::PermissionDenied,
                "{id:?} must be Work-only"
            );
        }
    }
}
