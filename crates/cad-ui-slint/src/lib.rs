use std::cell::{Cell, RefCell};

use std::rc::Rc;

use cad_app::{AnnotationToolKind, Command, CommandId, CommandPayload, MeasurementToolKind};

use cad_domain::{
    AnnotationId, CadError, CadResult, DocumentId, LayerId, Point3, SpaceId, ViewportId,
};

/// Source of the shared shell, kept for packaging/documentation tooling.
pub const UI_DEFINITION: &str = include_str!("../ui/app.slint");

/// Default (Chinese) UI strings; UI text is externalised per spec §3.5.
///
/// Re-exported from [`i18n::ZH_CN_JSON`]; the catalog module owns the source of
/// truth. Kept for packaging/documentation tooling.
pub const ZH_CN_MESSAGES: &str = i18n::ZH_CN_JSON;

slint::include_modules!();

pub mod bridge;

pub mod draw;

pub mod i18n;

pub mod responsive;

pub mod status;

#[cfg(target_arch = "wasm32")]
pub mod web;

pub use bridge::{
    build_scene, build_scene_with_annotations, build_scene_with_annotations_in_space,
    build_scene_with_fonts, build_scene_with_overrides, build_scene_with_space,
    camera2d_from_params, camera3d_from_params, fit_camera, install as install_cad_bridge,
    install_with_preference, layout_descriptors, BridgeCamera, CadView, IncomingDocument,
};

pub use draw::{
    draw_error_text, draw_kind_from_label, draw_kind_labels, draw_overlay_preview, DrawCommandSink,
    DrawPreviewSink, DrawUiState,
};

pub use i18n::{Locale, LocaleResolution, Message, MessageCatalog, MessageSource};

pub use responsive::{Breakpoint, ResponsiveMetrics, MIN_POINTER_TARGET, MIN_TOUCH_TARGET};

pub use status::{DiagnosticRowUi, DiagnosticsPanelState, ReasonText};

use slint::{ComponentHandle, Image, Weak};

/// Navigator input routed from the shell's canvas into the CAD view.
pub trait ViewInput {
    /// kind: 0=down 1=up 2=move 3=cancel; button: 0=none 1=left 2=right 3=middle.
    fn pointer(&self, kind: i32, button: i32, x: f64, y: f64);
    fn scroll(&self, dx: f64, dy: f64);
}

/// Maps a canvas click in logical pixels to a world-space point.
///
/// The host owns the camera, so only it can invert the 2D view mapping; the
/// adapter never fabricates a world point from raw pixels (audit B17/U04). A
/// host installs this alongside [`ViewInput`] via
/// [`UiAdapter::set_canvas_pick_mapper`]. Until it is installed, a measurement
/// pick is reported as unwired instead of silently dropped.
pub trait CanvasPickMapper: 'static {
    /// `logical` is relative to the CAD content rectangle. Returns `None` when
    /// this host cannot resolve a point yet.
    fn to_world(&self, logical: [f64; 2]) -> Option<Point3>;
}

/// Receives a paper-space switch chosen in the layout panel (audit F04/U03).
///
/// The UI cannot select a space by itself: which layouts are drawable and how a
/// viewport is clipped lives in the host's representation pipeline. A host
/// installs this via [`UiAdapter::set_layout_switch_sink`]; until then a click is
/// reported as unwired, never silently ignored.
pub trait LayoutSwitchSink: 'static {
    fn select(&mut self, space: cad_representation::SpaceSelection);
}

/// Cloneable handle the host uses to push state into the UI.
#[derive(Clone)]
pub struct UiHandle {
    ui: Weak<YacrWindow>,
    /// The active catalog. Shared with the adapter so `set_locale` can rebuild
    /// every chrome label, not just the Open button.
    messages: Rc<RefCell<MessageSource>>,
    /// Shared with the adapter so a state push also updates the algorithm the
    /// Measure button will start, keeping the panel and the button consistent.
    selected_kind: Rc<Cell<MeasurementToolKind>>,
    /// Shared with the adapter; canvas clicks only act as picks while a
    /// measurement tool is running, so navigation clicks stay silent.
    measurement_active: Rc<Cell<bool>>,
    /// Shared with the adapter; the annotation tool selector and the Annotate
    /// button use the same kind so they cannot disagree.
    selected_annotation_kind: Rc<Cell<AnnotationToolKind>>,
    /// Shared with the adapter; canvas picks while an annotation tool is active
    /// become annotation points rather than measurement points.
    annotation_active: Rc<Cell<bool>>,
    /// Ordered `AnnotationId`s matching the pushed `annotation-rows` model, so a
    /// visibility toggle / delete callback index maps back to the exact id.
    annotation_order: Rc<RefCell<Vec<AnnotationId>>>,
    /// Ordered `LayerId`s matching the pushed `layer-rows` model, so a toggle
    /// callback index maps back to the exact id.
    layer_order: Rc<RefCell<Vec<LayerId>>>,
    /// Ordered `LayoutId`s matching the pushed `layout-rows` model.
    layout_order: Rc<RefCell<Vec<cad_domain::LayoutId>>>,
    /// Last pushed override/selection counts, so a locale switch can reformat
    /// their labels without the host re-pushing the whole panel.
    layer_override_count: Rc<Cell<i32>>,
    selection_count: Rc<Cell<i32>>,
    /// Last pushed annotation hidden count, for locale reformatting.
    annotation_hidden_count: Rc<Cell<i32>>,
    /// Mirrors the pushed 3D view mode; gates orbit-by-drag and the 2D/3D
    /// affordance. Set through [`UiHandle::set_view_state`].
    view_3d: Rc<Cell<bool>>,
    /// Last authoritative mode, so a locale switch re-emits the correct mode
    /// label instead of resetting it to a default (audit U02).
    work_mode: Rc<Cell<bool>>,
    /// Last pushed asynchronous-open snapshot (F01), so a locale switch can
    /// reformat the phase/progress labels without the host re-polling.
    import_snapshot: Rc<RefCell<Option<cad_app::ImportProgressSnapshot>>>,
}

/// Owns the Slint component and routes UI callbacks into commands.
pub struct UiAdapter {
    pub configuration: UiConfiguration,
    ui: YacrWindow,
    view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>>,
    pick_mapper: Rc<RefCell<Option<Rc<dyn CanvasPickMapper>>>>,
    /// Host sink for layout (paper-space) switches (F04); see
    /// [`LayoutSwitchSink`]. `None` makes the layout click report "unwired".
    layout_switch: Rc<RefCell<Option<Box<dyn LayoutSwitchSink>>>>,
    /// Host sink that turns a confirmed draw/edit [`cad_app::DrawIntent`] into
    /// exactly one drawing command (drawing-edit §2/§4). `None` makes a confirm
    /// report "unwired" instead of fabricating a command.
    draw_command_sink: Rc<RefCell<Option<Box<dyn DrawCommandSink>>>>,
    /// Host sink that receives the in-progress draw preview for the CAD overlay
    /// ([`CadView::set_draw_preview`]). `None` simply means no live overlay.
    draw_preview_sink: Rc<RefCell<Option<Rc<dyn DrawPreviewSink>>>>,
    /// The active draw/edit capture state, driven by this adapter's callbacks.
    draw_tool: Rc<RefCell<Option<cad_app::DrawTool>>>,
    /// Kind currently selected in the shell; the Measure button and the canvas
    /// picks both use it so they cannot disagree.
    selected_kind: Rc<Cell<MeasurementToolKind>>,
    /// Mirrors `MeasurementUiState::active` so canvas clicks are only picks
    /// while a tool runs.
    measurement_active: Rc<Cell<bool>>,
    /// Kind currently selected for annotation creation (Annotate button +
    /// canvas picks agree with the selector).
    selected_annotation_kind: Rc<Cell<AnnotationToolKind>>,
    /// Mirrors whether an annotation tool is active, so canvas picks route to
    /// the annotation tool rather than the measurement tool.
    annotation_active: Rc<Cell<bool>>,
    /// Ordered `AnnotationId`s matching the pushed `annotation-rows` model.
    annotation_order: Rc<RefCell<Vec<AnnotationId>>>,
    /// Ordered `LayerId`s matching the pushed `layer-rows` model.
    layer_order: Rc<RefCell<Vec<LayerId>>>,
    /// Ordered `LayoutId`s matching the pushed `layout-rows` model.
    layout_order: Rc<RefCell<Vec<cad_domain::LayoutId>>>,
    /// The active catalog, shared with every handle.
    messages: Rc<RefCell<MessageSource>>,
    /// Last pushed counts, mirrored into handles for locale reformatting.
    layer_override_count: Rc<Cell<i32>>,
    selection_count: Rc<Cell<i32>>,
    annotation_hidden_count: Rc<Cell<i32>>,
    /// Mirrors the pushed 3D view mode so a left drag in 3D emits `Orbit`
    /// instead of being routed as a 2D navigation gesture.
    view_3d: Rc<Cell<bool>>,
    /// Last authoritative mode, shared with handles so a locale switch can
    /// re-emit the correct mode label (audit U02).
    work_mode: Rc<Cell<bool>>,
    /// Last pushed asynchronous-open snapshot (F01), shared with handles so a
    /// locale switch reforms the phase/progress labels.
    import_snapshot: Rc<RefCell<Option<cad_app::ImportProgressSnapshot>>>,
}

mod adapter;
mod chrome;
mod command_line;
mod handle;
mod state;

pub use adapter::*;
pub use chrome::*;
pub use state::*;

#[cfg(test)]
mod tests;
