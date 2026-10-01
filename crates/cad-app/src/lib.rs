//! Shared commands and sessions. Hosts assemble services, not CAD business logic.
//!
//! Spec v2.0 §4.7, §4.10: commands express intent; transactions guard data;
//! history restores it; the change set notifies downstream. Mode permissions are
//! enforced here at the command layer, not by hiding UI buttons.

use cad_annotations::{AnnotationCommand, AnnotationService};
use cad_db::{
    AnnotationDatabase, ChangeSet, DrawingDatabase, MeasurementAlgorithm, MeasurementRecord,
};
use cad_domain::*;
use cad_history::{patch, History, UndoRecord};
use cad_measure::{MeasurementEngine, MeasurementRequest, MeasurementSpace};
use cad_query::QueryService;
use std::{collections::BTreeMap, sync::Arc};

pub mod host;
pub mod measure_tool;

pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};

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
    Annotating { points: Vec<Point3> },
    Panning,
}

pub struct SessionState {
    pub document: DocumentId,
    mode: AppMode,
    pub active_space: SpaceId,
    pub selection: Vec<SelectionRef>,
    pub layer_overrides: BTreeMap<LayerId, bool>,
    pub tool: ToolState,
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
            selection: vec![],
            layer_overrides: BTreeMap::new(),
            tool: ToolState::Idle,
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
}

pub enum Projection {
    Orthographic { scale: f64 },
    Perspective { vertical_fov_radians: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardView {
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Isometric,
}

pub enum DisplayStyle {
    Wireframe,
    Shaded,
    ShadedWithEdges,
}

pub struct Camera {
    pub eye: Point3,
    pub target: Point3,
    pub up: Point3,
    pub projection: Projection,
}

pub struct Viewport {
    pub id: ViewportId,
    pub document: DocumentId,
    pub camera: Camera,
    pub work_plane: WorkPlane,
    pub logical_size: [f64; 2],
    pub dpi_scale: f64,
    pub clip: Option<Bounds3>,
    pub style: DisplayStyle,
}

impl Viewport {
    pub fn new(id: ViewportId, document: DocumentId, logical_size: [f64; 2]) -> Self {
        Viewport {
            id,
            document,
            camera: Camera {
                eye: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1000.0,
                },
                target: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                up: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                projection: Projection::Orthographic { scale: 1.0 },
            },
            work_plane: WorkPlane {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                u: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                v: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
            logical_size,
            dpi_scale: 1.0,
            clip: None,
            style: DisplayStyle::Wireframe,
        }
    }

    /// World units per logical pixel in the top-down 2D view.
    pub fn world_per_px(&self) -> f64 {
        match self.camera.projection {
            Projection::Orthographic { scale } => scale.max(1e-9),
            Projection::Perspective { .. } => 1.0,
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
                session.layer_overrides.clear();
                Ok(CommandOutcome::none())
            }
            CommandId::ToggleLayer => {
                if let CommandPayload::Layer(id, visible) = command.payload {
                    session.layer_overrides.insert(id, visible);
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
            CommandId::ConfirmMeasurement => self.confirm_measurement(session, &command),
            CommandId::CancelMeasurement => {
                // Cancelling is always allowed and never opens a transaction.
                session.cancel_tool()?;
                Ok(CommandOutcome::none())
            }
            CommandId::CreateAnnotation
            | CommandId::UpdateAnnotation
            | CommandId::DeleteAnnotation => self.annotation_command(&command),
            CommandId::Undo => self.undo(&command),
            CommandId::Redo => self.redo(&command),
            CommandId::SwitchProjection => {
                let viewport = self.viewport_mut(&command)?;
                viewport.camera.projection = match viewport.camera.projection {
                    Projection::Orthographic { scale } => Projection::Perspective {
                        vertical_fov_radians: 45f64.to_radians() * scale,
                    },
                    Projection::Perspective { .. } => Projection::Orthographic { scale: 1.0 },
                };
                Ok(CommandOutcome::none())
            }
            CommandId::StandardView => {
                if let CommandPayload::StandardView(view) = command.payload {
                    self.apply_standard_view(&command, view)?;
                    Ok(CommandOutcome::none())
                } else {
                    Err(CadError::InvalidInput(
                        "StandardView needs a view payload".into(),
                    ))
                }
            }
            CommandId::ResetView => {
                let viewport = self.viewport_mut(&command)?;
                viewport.camera.target = Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                };
                viewport.camera.projection = Projection::Orthographic { scale: 1.0 };
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
                session.tool = ToolState::Selecting;
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
                    })
                } else {
                    Err(CadError::InvalidInput(
                        "SwitchBackend needs a backend payload".into(),
                    ))
                }
            }
            CommandId::Switch2d3d => pending("app.command.switch_2d3d"),
            CommandId::Orbit => pending("app.command.orbit"),
        }
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
        viewport.camera.target = Point3 {
            x: cx,
            y: cy,
            z: 0.0,
        };
        viewport.camera.eye = Point3 {
            x: cx,
            y: cy,
            z: viewport.camera.eye.z,
        };
        let ex = (max.x - min.x).max(1e-6);
        let ey = (max.y - min.y).max(1e-6);
        let w = viewport.logical_size[0].max(1.0);
        let h = viewport.logical_size[1].max(1.0);
        let scale = (ex / w).max(ey / h) * 1.05;
        viewport.camera.projection = Projection::Orthographic { scale };
        session.generation += 1;
        Ok(())
    }

    fn pan(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput(
                "Pan needs a world-space delta point".into(),
            ));
        };
        let delta = *points
            .first()
            .ok_or_else(|| CadError::InvalidInput("Pan delta missing".into()))?;
        let viewport = self.viewport_mut(command)?;
        viewport.camera.target = Point3 {
            x: viewport.camera.target.x - delta.x,
            y: viewport.camera.target.y - delta.y,
            z: 0.0,
        };
        viewport.camera.eye = Point3 {
            x: viewport.camera.target.x,
            y: viewport.camera.target.y,
            z: viewport.camera.eye.z,
        };
        Ok(CommandOutcome::none())
    }

    fn zoom(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Points(points) = &command.payload else {
            return Err(CadError::InvalidInput("Zoom needs a scale factor".into()));
        };
        let factor = points
            .first()
            .map(|p| p.x)
            .ok_or_else(|| CadError::InvalidInput("Zoom factor missing".into()))?;
        if factor <= 0.0 || !factor.is_finite() {
            return Err(CadError::InvalidInput(
                "Zoom factor must be positive and finite".into(),
            ));
        }
        let viewport = self.viewport_mut(command)?;
        if let Projection::Orthographic { scale } = viewport.camera.projection {
            viewport.camera.projection = Projection::Orthographic {
                scale: (scale / factor).max(1e-9),
            };
        }
        Ok(CommandOutcome::none())
    }

    fn apply_standard_view(&mut self, command: &Command, view: StandardView) -> CadResult<()> {
        let viewport = self.viewport_mut(command)?;
        let t = viewport.camera.target;
        let d = 1000.0;
        viewport.camera.up = Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        match view {
            StandardView::Top => {
                viewport.camera.eye = Point3 {
                    x: t.x,
                    y: t.y,
                    z: t.z + d,
                };
                viewport.camera.up = Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                };
            }
            StandardView::Bottom => {
                viewport.camera.eye = Point3 {
                    x: t.x,
                    y: t.y,
                    z: t.z - d,
                }
            }
            StandardView::Front => {
                viewport.camera.eye = Point3 {
                    x: t.x,
                    y: t.y - d,
                    z: t.z,
                }
            }
            StandardView::Back => {
                viewport.camera.eye = Point3 {
                    x: t.x,
                    y: t.y + d,
                    z: t.z,
                }
            }
            StandardView::Left => {
                viewport.camera.eye = Point3 {
                    x: t.x - d,
                    y: t.y,
                    z: t.z,
                }
            }
            StandardView::Right => {
                viewport.camera.eye = Point3 {
                    x: t.x + d,
                    y: t.y,
                    z: t.z,
                }
            }
            StandardView::Isometric => {
                viewport.camera.eye = Point3 {
                    x: t.x + d,
                    y: t.y - d,
                    z: t.z + d,
                }
            }
        }
        viewport.camera.target = t;
        Ok(())
    }

    /// Dispatch the measurement tool (spec F06/U04).
    ///
    /// The algorithm comes from the active tool kind, never from the raw point
    /// count. A stateless `Points` payload with no active tool keeps the old
    /// convenience inference for direct callers and CLI; the interactive tool
    /// path is the one the UI uses.
    fn measure(&self, session: &mut SessionState, command: &Command) -> CadResult<CommandOutcome> {
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
        &self,
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
        let message = if preview.remaining > 0 {
            format!(
                "{}：已选 {} 点，还需 {} 点",
                preview.kind.label(),
                preview.points.len(),
                preview.remaining
            )
        } else {
            format!(
                "{}：已选 {} 点，可确认",
                preview.kind.label(),
                preview.points.len()
            )
        };
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: "measure.preview".into(),
                message,
            }],
            measurement: None,
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
        let space = if matches!(algorithm, MeasurementAlgorithm::PlanarPolygonArea) {
            let viewport = self
                .workspace
                .viewports
                .get(&command.viewport)
                .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
            MeasurementSpace::Plane(viewport.work_plane)
        } else {
            MeasurementSpace::World3d
        };
        let request = MeasurementRequest {
            algorithm,
            points,
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
        })
    }

    fn annotation_command(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let CommandPayload::Annotation(annotation_command) = &command.payload else {
            return Err(CadError::InvalidInput(
                "annotation command needs an annotation payload".into(),
            ));
        };
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // Build the undo patch from the before/after state.
        let (patch, label) = match annotation_command.as_ref() {
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
        let inner = match annotation_command.as_ref() {
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
    pub fn prepare_leave(
        &mut self,
        document: DocumentId,
        decision: UnsavedDecision,
    ) -> CadResult<()> {
        let dirty = self
            .workspace
            .documents
            .get(&document)
            .map(|d| d.annotations.is_dirty())
            .unwrap_or(false);
        if !dirty {
            return Ok(());
        }
        match decision {
            UnsavedDecision::Cancel => Err(CadError::Cancelled),
            UnsavedDecision::Save => Err(CadError::Unsupported(
                "annotation export is performed by the platform host".into(),
            )),
            UnsavedDecision::PreserveRecovery | UnsavedDecision::ExplicitDiscard => Ok(()),
        }
    }
}

pub enum UnsavedDecision {
    Save,
    PreserveRecovery,
    ExplicitDiscard,
    Cancel,
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
        assert_eq!(session.layer_overrides.get(&LayerId(3)), Some(&false));
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
            .prepare_leave(DocumentId(1), UnsavedDecision::ExplicitDiscard)
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
    fn switch_2d3d_and_orbit_stay_pending() {
        let (mut app, mut session) = application_with_document();
        for id in [CommandId::Switch2d3d, CommandId::Orbit] {
            assert!(matches!(
                app.execute(&mut session, command(id, CommandPayload::None)),
                Err(CadError::NotImplemented(_))
            ));
        }
    }
}
