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
pub mod groups;
pub mod host;
pub mod import;
pub mod input;
pub mod interaction;
pub mod layer_property;
pub mod layers;
pub mod measure_tool;
pub mod ortho;
pub mod picking;
pub mod plugin_api;
pub mod point_report;
pub mod property_value;
pub mod recovery;
pub mod render_scene;
mod select_all;
mod select_box;
mod select_filter;
mod select_similar;
// `MULTIPLE` session repeat state (split out of `session_state`).
mod session_multiple;
// SSX: the Express-Tool selection-filter builder (criteria + apply).
pub mod selection;
pub mod snap;
mod ssx;
pub mod tasks;
pub mod tool_state;
pub mod vendor;
pub mod viewer_config;
pub mod zoom_history;

mod app_audit;
// The extended-command wrapper: dispatch, session toggles and reports.
mod app_dispatch;
mod app_drafting;
// The Wave 2 extended-command handlers (session tokens, reports, refusals).
mod app_extended_refusals;
// The Wave 17-24 data-driven named refusals (one registry, one handler).
mod app_extended_refusal_names;
mod app_extended_reports2;
mod app_extended_session2;
// The session-only SYSVARMONITOR watch set.
mod app_extended_sysvar;
// The Wave 3 extended-command handlers (reports/session and refusals).
mod app_extended_refusals3;
mod app_extended_reports3;
// The Wave 4 extended-command handlers (real selection edits and refusals).
mod app_extended_edit4;
mod app_extended_refusals4;
// The Wave 5 extended-command handlers (attribute/xref token+reports+refusals).
mod app_extended_attrs5;
mod app_extended_refusals5;
// The Wave 6 extended-command handlers (file/plot/export token+reports+refusals).
mod app_extended_file6;
mod app_extended_refusals6;
// The Wave 7 extended-command handlers (dynamic-block visibility reports+refusals).
mod app_extended_dyn7;
mod app_extended_refusals7;
// The Wave 8 extended-command handlers (session tokens+refusals).
mod app_extended_refusals8;
mod app_extended_session8;
// The Wave 9 extended-command handlers (2D draw/modify, aliases, reports).
mod app_extended_w9_alias;
mod app_extended_w9_draw;
mod app_extended_w9_reports;
// The Wave 10 extended-command handlers (layer/view/session reports, refusals).
mod app_extended_w10_layer;
mod app_extended_w10_refusals;
mod app_extended_w10_session;
// The Wave 11 extended-command handlers (3D/mesh/surface refusals).
mod app_extended_w11_refusals;
// The Wave 12 extended-command handlers (file/import/export refusals).
mod app_extended_w12_refusals;
// The Wave 13 extended-command handlers (2D/table/page-setup reports+refusals).
mod app_extended_w13_refusals;
mod app_extended_w13_reports;
mod app_extended_w14_refusals;
mod app_extended_w15_refusals;
mod app_extended_w16_refusals;
// The Wave 16 read-only raster-image report (`IMAGE` / `-IMAGE`).
mod app_extended_w16_reports;
// The 2026-11-07 extras batch dispatcher and its view-only commands.
mod app_extras;
mod app_layout;
mod app_layout_geometry;
mod app_layout_layvpi;
mod app_layout_names;
mod app_layout_session;
// The Wave 27 read-only AutoCAD 2026 reports (`RECOVER`, `RECOVERALL`,
// `PURGEAECDATA`, `SHOWURLS`).
mod app_recovery;
// The bounded 2D in-place block-reference edit (`REFEDIT`/`REFSET`/`REFCLOSE`).
mod app_refedit;
mod app_reports;
mod app_reports_ext;
mod app_utility;
mod app_vendor_snapshot;
mod app_view;
mod app_view_extras;
mod app_zoom;
mod layer_history;
mod layer_states;
mod space;
pub mod view_states;

pub use api::{
    ApiOutcome, ApiRequest, ApiTarget, EntityEdit, NewEntity, RawCommandTrace, TracedCommandOutcome,
};
pub use app_extended_w9_reports::install_outline_fonts;
pub use app_measure_object::FULL_TURN_TOLERANCE_RADIANS;
pub use app_zoom::{canonical_logical_rect, FIT_MARGIN, WINDOW_MARGIN};
pub use cad_import_acadrust::{VendorRecord, VendorSource};
pub use cad_spatial::{
    BackFacePolicy, GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
};
pub use camera::{
    orthonormal_work_plane, xy_work_plane, Camera, Camera2dParams, Camera3dParams, Projection,
    ProjectionKind, StandardView, ViewBasis,
};
pub mod command_aliases;
pub mod command_id;
pub use command_id::{AppMode, CommandId, ExtendedCommandId, RefusalId};
pub use draw_tool::{
    circle_radius, AlignPairSet, DrawIntent, DrawPreview, DrawTool, DrawToolKind, PointArity,
    ToolParameter, ToolParameterStore, ToolValue, ToolValueKind, DEFAULT_POLYGON_SIDES,
    HATCH_GRADIENT_DEFAULT_ACI_A, HATCH_GRADIENT_DEFAULT_ACI_B, HATCH_GRADIENT_LINEAR,
    HATCH_GRADIENT_SPHERICAL, HATCH_PATTERN_TYPE_GRADIENT, HATCH_PATTERN_TYPE_PREDEFINED,
    HATCH_PATTERN_TYPE_SOLID, HATCH_PATTERN_TYPE_USER_DEFINED, HATCH_STYLE_IGNORE,
    HATCH_STYLE_NORMAL, HATCH_STYLE_OUTER, MAX_TOOL_TEXT_CHARS,
};
pub use groups::{EntityGroup, EntityGroups, ENTITY_GROUP_CAP};
pub use input::{
    apply_canvas_metrics, classify_key, escape_action, CancelReason, CanvasMetrics,
    DiagnosticsDrawer, EscapeAction, InputOutcome, InputPolicy, KeyAction, LoadingState,
    PointerPhase, PointerUpdate, StatusModel, ToolStatus, ViewMetrics, DRAG_THRESHOLD_LOGICAL_PX,
};
pub use layer_history::{LayerOverrideHistory, LAYER_OVERRIDE_HISTORY_CAP};
pub use layer_states::{LayerStates, NamedLayerState, LAYER_STATE_CAP};
pub use measure_tool::{MeasurementPreview, MeasurementTool, MeasurementToolKind};
pub use ortho::{ortho_constrain, ortho_constrain_point};
pub use picking::{
    drawing_pick_items, filter_by_index, last_pick_stats, pick_at_screen, pick_ray,
    pick_ray_exhaustive, pick_ray_with_index, pick_tolerance, PickIndex, PickStats,
};
pub use plugin_api::PluginDocument;
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
pub use view_states::{NamedView, ViewStates, VIEW_STATE_CAP};
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

pub use session_state::{
    BackendChoice, ChamferMethod, ClipboardEntry, IsoPlane, LayoutViewportRef, ModifyOptions,
    MultipleRepeat, RefEditActive, RefEditMember, RefEditSession, SessionState, ViewExtras,
    VisualStyleToken, WipeoutFrames,
};

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
    /// `REDRAW` refresh stamp: advanced without touching the camera, the
    /// drawing or the `ZOOM PREVIOUS` history.
    pub refresh_generation: u64,
    /// `REGEN` regeneration stamp: the host may invalidate its derived scene
    /// cache when this advances. No database write or revision bump.
    pub regen_generation: u64,
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
            refresh_generation: 0,
            regen_generation: 0,
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
mod draw2_payload;
mod layout_payload;
pub mod ml_style;

pub use draw2_payload::{Draw2Payload, DrawOrderMode, MlStyleCommand, TrexMode};
pub use layout_payload::{LayoutPayload, PaperSetup};
pub use ml_style::{MultilineStyle, MultilineStyleStore, MULTILINE_STYLE_CAP};

pub use command_payload::{
    AddSelectedCommand, AngularDimensionInput, AnnotationBatchPayload, CommandPayload,
    DashLayerOps, DimEditCommand, DimJogLineCommand, DimOverrideCommand, DimStyleCommand,
    DimTextEditCommand, DimensionBatchPayload, DimensionChainCommand, FilterExpression,
    FilterLogic, FilterRequest, GripStretchCommand, GroupCommand, GroupEditCommand, HatchDrawOrder,
    HatchEditOption, HatchOriginCommand, ImageAdjustCommand, ImageAttachCommand, ImageClipCommand,
    JoinMode, JoinOptions, JustifyTextCommand, LayerStateCommand, LeaderCommand, LinetypeCommand,
    MInsertCommand, MLeaderAlignCommand, MLeaderAlignMode, MLeaderEditAction, MLeaderEditCommand,
    MatchPropSettings, MeasureGeomOption, MirrorOptions, MlEditCommand, MlEditMode, MocoroCommand,
    NamedObjectKind, OffsetGapType, OffsetLayer, OffsetMode, OffsetOptions, OrdinateAxis,
    OrdinateDimensionCommand, PeditEditCommand, PeditEditOption, PeditOption, PeditVertexAction,
    PeditVertexCommand, PurgeTarget, QDimCommand, QDimMode, QDimSnap, QSelectRequest,
    RadialDimensionCommand, RefCloseOption, RefSetOption, RenameCommand, RevCloudStyle,
    RevisionCloudMode, RevisionCloudOptions, RotatedLinearDimensionCommand, SelectSimilarSettings,
    SelectionApply, SelectionOperator, SelectionPredicate, SelectionProperty, SelectionScope,
    SelectionToolPayload, SetByLayerSettings, ShapeCommand, StretchPolygonCommand,
    StyleTableCommand, StyleTableKind, StyleTableOp, SysVarMonitorAction, TCircleCommand,
    TCountCommand, TableCreateCommand, TableEditCommand, TextAlignAxis, TextAlignCommand,
    TextAlignEdge, TextAlignSpacing, TextAlignSpacingCommand, TextBoundaryShape, TextCaseMode,
    TextFitCommand, TextMaskKind, TextMaskOptions, TextOrder, TextScaleCommand, ToleranceCommand,
    TwoLineCommand, Txt2MTextOptions, UtilityPayload, VPointInput, ViewCommand, WipeoutPayload,
    XClipCommand,
};

pub use app_audit::AuditRequest;
pub use app_drawing::OverkillSettings;
pub use ssx::{SsxCriterion, SsxCriterionKind, SsxFilter, SsxRequest};

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
    /// Latest settled vendor derived-representation snapshot.
    ///
    /// Display and measurement both read this same immutable store; it is not
    /// document state and is replaced/cleared with the pass. `None` means the
    /// carrier has no settled vendor pass (or the document was replaced).
    pub vendor_representations: Option<Arc<crate::vendor::VendorRepresentationStore>>,
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
mod app_massprop;
mod app_measure;
mod app_measure_object;
mod app_measure_vendor;
mod application;
mod layers_isolate;

#[cfg(not(target_arch = "wasm32"))]
pub mod background;
#[cfg(test)]
mod tests;
