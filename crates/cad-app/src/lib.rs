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
pub use draw_tool::{
    circle_radius, DrawIntent, DrawPreview, DrawTool, DrawToolKind, PointArity, ToolParameter,
    ToolParameterStore, ToolValue, ToolValueKind, DEFAULT_POLYGON_SIDES,
};
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

/// The two straight-line picks of a CHAMFER/FILLET command (ADR 0016 decisions
/// B/C): the two picked refs in pick order and the raw click point that selects
/// each pick side.
///
/// Boxed inside [`CommandPayload::Chamfer`]/`Fillet` so the command enum stays
/// small (the same reason [`CommandPayload::Geometry`] boxes its geometry).
pub struct TwoLineCommand {
    pub first: SelectionRef,
    pub second: SelectionRef,
    pub pick_first: Point3,
    pub pick_second: Point3,
}

/// The input of a DIMRADIUS/DIMDIAMETER command (ADR 0017 decision C, P5d): the
/// picked arc/circle ref and the dimension-line location.
///
/// Boxed inside [`CommandPayload::DimRadial`] so the command enum stays small (the
/// same reason [`TwoLineCommand`] is boxed). Which of the two commands it is comes
/// from the [`CommandId`], so the payload carries no kind field (a second source
/// of truth).
pub struct RadialDimensionCommand {
    pub target: SelectionRef,
    pub location: Point3,
}

/// The two supported DIMANGULAR constructions (ADR 0017 decision C, P5e).
///
/// * [`Self::TwoLine`] — the interactive form: two picked straight lines plus the
///   dimension-arc location. The vertex is the intersection of the two infinite
///   lines (ADR 0017 U-8, pinned against the committed QCAD fixtures).
/// * [`Self::ThreePoint`] — the command-level form: a vertex, the two side
///   points and the dimension-arc location. It has no interactive capture yet:
///   AutoCAD's first prompt ("select a line **or** specify a vertex") cannot be
///   expressed by the fixed-step capture state machine. This is declared in the
///   capability notes, not hidden.
///
/// Which variant it is is part of the input, not a second command: both produce
/// one `DIMENSION` entity through the same funnel.
#[derive(Debug, Clone, PartialEq)]
pub enum AngularDimensionInput {
    /// Vertex, first side point, second side point, dimension-arc location.
    ThreePoint {
        vertex: Point3,
        first: Point3,
        second: Point3,
        location: Point3,
    },
    /// Two picked straight lines plus the dimension-arc location.
    TwoLine {
        first: SelectionRef,
        second: SelectionRef,
        location: Point3,
    },
}

/// The axis a DIMORDINATE measures (ADR 0017 decision C/E, P5e).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdinateAxis {
    /// X ordinate: group 70 bit `0x40` set.
    X,
    /// Y ordinate: group 70 bit `0x40` clear.
    Y,
}

/// The input of a DIMORDINATE command (ADR 0017 decision C, P5e): the feature
/// point, the leader endpoint and an optional explicit axis.
///
/// `axis = None` is the interactive form and applies the documented auto rule —
/// "if the Y difference is greater, measure the X ordinate" — comparing the
/// feature and leader points. Autodesk does not state the equal case; our
/// declared reading is **Y** (the documented rule's "otherwise" branch).
#[derive(Debug, Clone, PartialEq)]
pub struct OrdinateDimensionCommand {
    pub feature: Point3,
    pub leader: Point3,
    pub axis: Option<OrdinateAxis>,
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
    /// CHAMFER two straight lines (ADR 0016 decision C). The two picks and their
    /// raw click points travel in one boxed [`TwoLineCommand`] (boxed like
    /// [`CommandPayload::Geometry`] so the command enum stays small); the two
    /// distances are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ChamferDistanceA`]/`B`), read by the
    /// handler so the session is the single authority for them.
    Chamfer(Box<TwoLineCommand>),
    /// FILLET two straight lines (ADR 0016 decision B). Same shape as
    /// [`CommandPayload::Chamfer`]; the radius is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::FilletRadius`]).
    Fillet(Box<TwoLineCommand>),
    /// ARRAYRECT the selected refs on a rectangular grid (ADR 0016 decision D).
    ///
    /// The row/column counts and spacings are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayRows`] … `ArrayColumnSpacing`),
    /// read by the handler so the session stays the single authority for them.
    /// The source refs are retained; every copy is a fresh entity in one
    /// transaction. Associativity is NOT IMPLEMENTED.
    ArrayRect {
        refs: Vec<SelectionRef>,
    },
    /// ARRAYPOLAR the selected refs around `center` (ADR 0016 decision D).
    ///
    /// `center` is the captured array centre; the item count, fill angle and
    /// rotate-items flag are the session typed channel
    /// ([`crate::draw_tool::ToolParameter::ArrayItems`] … `ArrayRotateItems`).
    /// The source refs are retained; every copy is a fresh entity in one
    /// transaction. Associativity is NOT IMPLEMENTED.
    ArrayPolar {
        refs: Vec<SelectionRef>,
        center: Point3,
    },
    /// ALIGN the selected refs from two point pairs (ADR 0016 decision F).
    ///
    /// `refs` is the selection snapshot; the four points are the captured
    /// `source1`/`dest1`/`source2`/`dest2` (AutoCAD's capture order). The
    /// `AlignScale` flag is the session typed channel
    /// ([`crate::draw_tool::ToolParameter::AlignScale`]), read by the handler so
    /// the session stays the single authority for it. The composed transform is
    /// applied **in place** (no copies) in one transaction / one undo step.
    Align {
        refs: Vec<SelectionRef>,
        source1: Point3,
        dest1: Point3,
        source2: Point3,
        dest2: Point3,
    },
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// `rect_logical` is the two opposite corners of the **crossing window** in
    /// **canvas logical pixels** — the same convention as
    /// [`CommandPayload::BoxSelect`] and [`CommandPayload::ZoomAt`]; the command
    /// maps them through the active viewport's orthographic work plane and
    /// recomputes the enclosure classification from the rectangle and the
    /// geometry (it is **never** stored in the selection: single source of
    /// truth). The window is **always treated as CROSSING** (AutoCAD requires a
    /// crossing window; cpolygon/lasso are NOT IMPLEMENTED).
    ///
    /// `base` is the captured base point and `delta` the captured displacement
    /// point, both world coordinates; the effective translation is
    /// `delta − base`. A zero displacement (`delta == base`) is refused rather
    /// than a silent no-op.
    Stretch {
        rect_logical: [[f64; 2]; 2],
        base: Point3,
        delta: Point3,
    },
    /// Set the session's active layer for new entities. Session state only.
    ActiveLayer(LayerId),
    /// DIMRADIUS/DIMDIAMETER: the picked arc/circle (`target`) plus the
    /// dimension-line location (`location`, ADR 0017 decision C, P5d). The kind
    /// (radius vs diameter) comes from the [`CommandId`]; the handler resolves the
    /// entity, builds the definition through the shared synthesis and inserts one
    /// `AcDbDimension`.
    DimRadial(Box<RadialDimensionCommand>),
    /// DIMCENTER: the picked arc/circle (`target`) for the non-associative centre
    /// mark/centrelines (ADR 0017 decision C/D.7, P5d). The handler resolves the
    /// entity, reads the resolved `DIMCEN` and inserts ordinary LINE geometry
    /// (never a `DIMENSION`); `DIMCEN = 0` inserts nothing and reports it.
    DimCenter(SelectionRef),
    /// DIMANGULAR: the two-line construction (interactive) or the three-point
    /// construction (command-level), ADR 0017 decision C, P5e. The handler builds
    /// an `AcDbDimension` through the shared synthesis and inserts it on the
    /// active layer in one transaction / one undo step.
    DimAngular(Box<AngularDimensionInput>),
    /// DIMORDINATE: the feature point, leader endpoint and optional axis override
    /// (ADR 0017 decision C, P5e). The measurement is the feature's datum-relative
    /// (world-origin-relative) X or Y coordinate magnitude, matching P5b's
    /// ordinate semantics and the acadrust writer's group 42.
    DimOrdinate(Box<OrdinateDimensionCommand>),
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
