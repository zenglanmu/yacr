//! Shared commands and sessions. Hosts assemble services, not CAD business logic.
use cad_annotations::AnnotationCommand;
use cad_db::{AnnotationDatabase, ChangeSet, DrawingDatabase};
use cad_domain::*;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode { Viewer, Work }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenDrawing, CancelLoading, Pan, Zoom, FitDrawing, ResetView, ToggleLayer,
    RestoreLayers, SwitchSpace, Select, Measure, CreateAnnotation, UpdateAnnotation,
    DeleteAnnotation, Undo, Redo, ImportAnnotations, ExportAnnotations, Resources,
    Diagnostics, SwitchBackend, Switch2d3d, StandardView, Orbit, SwitchProjection,
}
impl CommandId {
    pub fn requires_work_mode(self) -> bool {
        matches!(self, Self::Measure | Self::CreateAnnotation | Self::UpdateAnnotation |
            Self::DeleteAnnotation | Self::Undo | Self::Redo | Self::ImportAnnotations)
    }
}
pub struct Document {
    pub id: DocumentId, pub drawing: Arc<DrawingDatabase>, pub annotations: AnnotationDatabase,
    pub identity: DocumentIdentity, pub units: UnitContext, pub resource_keys: Vec<String>,
}
#[derive(Debug, Clone)]
pub enum ToolState { Idle, Selecting, Measuring { points: Vec<Point3> }, Annotating { points: Vec<Point3> }, Panning }
pub struct SessionState {
    pub document: DocumentId, mode: AppMode, pub active_space: SpaceId,
    pub selection: Vec<SelectionRef>, pub layer_overrides: BTreeMap<LayerId, bool>,
    pub tool: ToolState, pub generation: u64,
}
impl SessionState {
    pub fn new(document: DocumentId, mode: AppMode) -> Self {
        Self { document, mode, active_space: SpaceId::Model, selection: vec![], layer_overrides: BTreeMap::new(), tool: ToolState::Idle, generation: 0 }
    }
    pub fn mode(&self) -> AppMode { self.mode }
    pub fn authorize(&self, command: CommandId) -> CadResult<()> {
        if self.mode == AppMode::Viewer && command.requires_work_mode() { Err(CadError::PermissionDenied) } else { Ok(()) }
    }
    pub fn switch_mode(&mut self, _mode: AppMode) -> CadResult<()> { pending("app.switch_mode_cancel_or_confirm") }
    pub fn cancel_tool(&mut self) -> CadResult<()> { pending("app.cancel_tool_and_preview") }
}
pub enum Projection { Orthographic { scale: f64 }, Perspective { vertical_fov_radians: f64 } }
pub enum StandardView { Top, Bottom, Front, Back, Left, Right, Isometric }
pub enum DisplayStyle { Wireframe, Shaded, ShadedWithEdges }
pub struct Camera { pub eye: Point3, pub target: Point3, pub up: Point3, pub projection: Projection }
pub struct Viewport {
    pub id: ViewportId, pub document: DocumentId, pub camera: Camera, pub work_plane: WorkPlane,
    pub logical_size: [f64; 2], pub dpi_scale: f64, pub clip: Option<Bounds3>, pub style: DisplayStyle,
}
pub struct PreviewState { pub viewport: ViewportId, pub geometry: Vec<SemanticGeometry> }
#[derive(Default)]
pub struct Workspace { pub documents: BTreeMap<DocumentId, Document>, pub viewports: BTreeMap<ViewportId, Viewport> }
pub enum CommandPayload { None, Annotation(AnnotationCommand), Points(Vec<Point3>), Layer(LayerId, bool), Space(SpaceId), StandardView(StandardView) }
pub struct Command { pub schema_version: u32, pub id: CommandId, pub document: DocumentId, pub viewport: ViewportId, pub payload: CommandPayload }
pub struct CommandOutcome { pub objects: Vec<ObjectId>, pub changes: Option<ChangeSet>, pub diagnostics: Vec<Diagnostic> }
pub struct CommandDeclaration { pub id: CommandId, pub undoable: bool, pub annotation_database: bool, pub merge_key: Option<String> }
pub trait CommandHandler {
    fn declaration(&self) -> CommandDeclaration;
    fn execute(&self, session: &mut SessionState, document: &mut Document, command: &Command) -> CadResult<CommandOutcome>;
}
pub struct Application { pub workspace: Workspace }
impl Application {
    pub fn execute(&mut self, session: &mut SessionState, command: Command) -> CadResult<CommandOutcome> {
        session.authorize(command.id)?;
        if session.document != command.document { return Err(CadError::StaleResult); }
        pending("app.command_dispatch")
    }
    pub fn prepare_leave(&mut self, _document: DocumentId, _decision: UnsavedDecision) -> CadResult<()> { pending("app.unsaved_guard") }
}
pub enum UnsavedDecision { Save, PreserveRecovery, ExplicitDiscard, Cancel }
pub enum InputEvent {
    Pointer { logical_position: [f64; 2], contacts: u8 }, Confirm, Cancel,
    Key { key: String, composing: bool, text_focus: bool }, ViewMetrics { logical_size: [f64; 2], dpi_scale: f64 },
}
pub trait Tool { fn handle(&mut self, event: InputEvent, session: &mut SessionState, preview: &mut PreviewState) -> CadResult<Option<Command>>; }
