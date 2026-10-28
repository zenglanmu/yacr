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
pub mod grips;
pub mod host;
pub mod input;
pub mod interaction;
pub mod layer_property;
pub mod layers;
pub mod measure_tool;
pub mod ortho;
pub mod picking;
pub mod point_report;
pub mod property_value;
pub mod recovery;
pub mod render_scene;
mod select_all;
mod select_box;
pub mod selection;
pub mod snap;
pub mod tasks;
pub mod tool_state;
pub mod viewer_config;
pub mod zoom_history;

mod app_zoom;
mod layer_history;
mod space;

pub use api::{
    ApiOutcome, ApiRequest, ApiTarget, EntityEdit, NewEntity, RawCommandTrace, TracedCommandOutcome,
};
pub use app_measure_object::FULL_TURN_TOLERANCE_RADIANS;
pub use app_zoom::{canonical_logical_rect, FIT_MARGIN, WINDOW_MARGIN};
pub use cad_spatial::{
    BackFacePolicy, GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
};
pub use camera::{
    orthonormal_work_plane, xy_work_plane, Camera, Camera2dParams, Camera3dParams, Projection,
    ProjectionKind, StandardView, ViewBasis,
};
pub mod command_id;
pub use command_id::{AppMode, CommandId};
pub use draw_tool::{
    circle_radius, DrawIntent, DrawPreview, DrawTool, DrawToolKind, PointArity, ToolParameter,
    ToolParameterStore, ToolValue, ToolValueKind, DEFAULT_POLYGON_SIDES,
    HATCH_GRADIENT_DEFAULT_ACI_A, HATCH_GRADIENT_DEFAULT_ACI_B, HATCH_GRADIENT_LINEAR,
    HATCH_GRADIENT_SPHERICAL, HATCH_PATTERN_TYPE_GRADIENT, HATCH_PATTERN_TYPE_PREDEFINED,
    HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED, HATCH_STYLE_IGNORE,
    HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER, MAX_TOOL_TEXT_CHARS,
};
pub use input::{
    apply_canvas_metrics, classify_key, escape_action, CancelReason, CanvasMetrics,
    DiagnosticsDrawer, EscapeAction, InputOutcome, InputPolicy, KeyAction, LoadingState,
    PointerPhase, PointerUpdate, StatusModel, ToolStatus, ViewMetrics, DRAG_THRESHOLD_LOGICAL_PX,
};
pub use layer_history::{LayerOverrideHistory, LAYER_OVERRIDE_HISTORY_CAP};
pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
pub use ortho::{ortho_constrain, ortho_constrain_point};
pub use picking::{
    drawing_pick_items, filter_by_index, last_pick_stats, pick_at_screen, pick_ray,
    pick_ray_exhaustive, pick_ray_with_index, pick_tolerance, PickIndex, PickStats,
};
pub use point_report::{unit_symbol, IdSpace, InputSnap, PointReport};
pub use recovery::{ActiveBackendKind, BackendFailure, BackendOutcome};
pub use selection::{
    entity_property_rows, enum_options, property_row_editable, PropertyGroup, PropertyKind,
    PropertyRow, SelectionProperties, SelectionSet, MAX_ENUM_OPTIONS,
};
pub use snap::{snap_point, snap_spacing_is_valid, snap_to_grid, DEFAULT_SNAP_SPACING};
pub use space::*;
pub use tasks::{
    import_phase_key, AsyncOpenPoll, ImportJob, ImportManager, ImportProgressSnapshot,
    ImportTerminal,
};
pub use tool_state::{DrawCapture, ToolSnapshot, ToolState};
pub use zoom_history::{ViewSnapshot, ZoomHistory, ZOOM_HISTORY_CAP};

use interaction::SelectionMode;

pub struct Document {
    pub id: DocumentId,
    pub drawing: Arc<DrawingDatabase>,
    pub identity: DocumentIdentity,
    pub units: UnitContext,
    pub resource_keys: Vec<String>,
}

mod session_state;

pub use session_state::{BackendChoice, ClipboardEntry, SessionState};

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
    /// Bounded history of discrete view snapshots for `ZOOM PREVIOUS`
    /// ([`crate::zoom_history`]): per viewport, session state only, never
    /// persisted.
    pub zoom_history: ZoomHistory,
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
            zoom_history: ZoomHistory::new(),
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

mod command_payload;

pub use command_payload::{
    AngularDimensionInput, CommandPayload, DimensionChainCommand, GripStretchCommand, OrdinateAxis,
    OrdinateDimensionCommand, RadialDimensionCommand, TwoLineCommand,
};

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
    ///
    /// Strictly a **measurement**: `ID` produces [`PointReport`] instead, so a
    /// consumer can never mistake a coordinate report for a measured scalar.
    pub measurement: Option<MeasurementRecord>,
    /// The `ID` point-coordinate report, when the command produced one.
    ///
    /// `None` for every other command, including every real measurement.
    pub point_report: Option<PointReport>,
}

impl CommandOutcome {
    pub fn none() -> Self {
        CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: Vec::new(),
            measurement: None,
            point_report: None,
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
mod app_measure_object;
mod application;
mod layers_isolate;

#[cfg(not(target_arch = "wasm32"))]
pub mod background;
#[cfg(test)]
mod tests;
