//! Shared UI boundary: a real Slint shell compositing a wgpu-produced CAD frame.
//!
//! Spec v2.0 §5: a single presentation coordinator (Slint) owns the window and
//! surface; the CAD renderer draws into a texture that Slint composites. The UI
//! never creates GPU objects and never draws CAD entities itself.

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
pub mod i18n;
pub mod responsive;
pub mod status;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub use bridge::{
    build_scene, build_scene_with_annotations, build_scene_with_annotations_in_space,
    build_scene_with_fonts, build_scene_with_overrides, build_scene_with_space,
    camera2d_from_params, camera3d_from_params, fit_camera, install as install_cad_bridge,
    layout_descriptors, BridgeCamera, CadView, IncomingDocument,
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

/// Snapshot of the measurement panel state pushed into the shell (audit U03/U04).
///
/// These are plain values so the whole panel can be derived from
/// `HostController::measurement_preview()` without the UI re-running the tool.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementUiState {
    /// Whether a tool is running.
    pub active: bool,
    /// Whether the confirm affordance is valid right now.
    pub can_confirm: bool,
    pub kind: MeasurementToolKind,
    /// Human-facing "picked N, need M" step text; empty when idle.
    pub step_label: String,
    pub unit_label: String,
}

impl Default for MeasurementUiState {
    fn default() -> Self {
        MeasurementUiState {
            active: false,
            can_confirm: false,
            kind: MeasurementToolKind::Distance,
            step_label: String::new(),
            unit_label: String::new(),
        }
    }
}

impl MeasurementUiState {
    /// Derive the panel state from an optional active preview and unit label.
    pub fn from_preview(
        preview: Option<&cad_app::MeasurementPreview>,
        unit_label: impl Into<String>,
    ) -> Self {
        match preview {
            Some(preview) => MeasurementUiState {
                active: true,
                can_confirm: preview.can_confirm(),
                kind: preview.kind,
                step_label: preview.status_line(),
                unit_label: unit_label.into(),
            },
            None => MeasurementUiState {
                active: false,
                can_confirm: false,
                kind: MeasurementToolKind::Distance,
                step_label: String::new(),
                unit_label: unit_label.into(),
            },
        }
    }

    /// Combobox index for [`MeasurementToolKind::ALL`].
    pub fn kind_index(&self) -> i32 {
        self.kind.index() as i32
    }
}

/// View/observation state pushed into the shell (F13/F14).
///
/// `is_3d` drives the 2D/3D affordance and the adapter's orbit-by-drag gate;
/// `perspective` drives the projection toggle's pressed state. Both are
/// derivations of the authoritative application viewport, never invented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewStateUi {
    pub is_3d: bool,
    pub perspective: bool,
}

/// One layer row pushed into the shell (audit F03/U03).
///
/// `id` is only a display value; the adapter keeps the ordered `LayerId` list so
/// a toggle maps back to the exact id without a lossy `u128 → i32` cast.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerRowUi {
    pub id: i32,
    pub name: String,
    /// Effective visibility the scene honours (database flag ⊕ override).
    pub visible: bool,
    /// Whether a temporary session override is active for this layer.
    pub overridden: bool,
}

/// Layer-panel snapshot derived from [`cad_app::layers::LayerRow`]s.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerPanelState {
    pub rows: Vec<LayerRowUi>,
    /// Number of layers currently overridden; drives the "restore" affordance.
    pub override_count: usize,
    /// Explicit empty-state text shown when there are no layers.
    pub empty_label: String,
}

impl LayerPanelState {
    /// Build the panel state from real layer rows.
    pub fn from_rows(rows: &[cad_app::layers::LayerRow], empty_label: impl Into<String>) -> Self {
        LayerPanelState {
            rows: rows
                .iter()
                .map(|row| LayerRowUi {
                    // Display-only: the low 32 bits are enough to label a row;
                    // the exact LayerId round-trips through the adapter's order.
                    id: (row.id.0 & 0xFFFF_FFFF) as i32,
                    name: row.name.clone(),
                    visible: row.effective_visible,
                    overridden: row.is_overridden(),
                })
                .collect(),
            override_count: rows.iter().filter(|r| r.is_overridden()).count(),
            empty_label: empty_label.into(),
        }
    }
}

/// One layout row pushed into the shell (audit F04/U03).
///
/// `id` is a display value; the adapter keeps the ordered `LayoutId` list so a
/// switch maps back to the exact id. `supported` is false when the layout has a
/// viewport this build cannot draw; `reason` then explains why.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRowUi {
    pub id: i32,
    pub name: String,
    pub supported: bool,
    pub reason: String,
    pub viewport_count: i32,
}

/// Layout-panel snapshot derived from the database's real layout table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutPanelState {
    pub rows: Vec<LayoutRowUi>,
    /// Index of the active row in `rows`, or `None` for model space.
    pub active_index: Option<i32>,
    /// Explicit empty-state text shown when the drawing has no paper layouts.
    pub empty_label: String,
}

impl LayoutPanelState {
    /// Build the panel state from the representation layer's descriptors.
    pub fn from_descriptors(
        descriptors: &[cad_representation::LayoutDescriptor],
        active: cad_representation::SpaceSelection,
        empty_label: impl Into<String>,
    ) -> Self {
        let rows: Vec<LayoutRowUi> = descriptors
            .iter()
            .map(|d| LayoutRowUi {
                // Display-only: the low 32 bits label a row; the exact LayoutId
                // round-trips through the adapter's order.
                id: (d.id.0 & 0xFFFF_FFFF) as i32,
                name: d.name.clone(),
                supported: d.supported,
                reason: d.reason.clone(),
                viewport_count: d.viewport_count as i32,
            })
            .collect();
        let active_index = active.layout().and_then(|id| {
            descriptors
                .iter()
                .position(|d| d.id == id)
                .map(|i| i as i32)
        });
        LayoutPanelState {
            rows,
            active_index,
            empty_label: empty_label.into(),
        }
    }
}

/// One read-only property row pushed into the shell (audit F05/U03).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRowUi {
    pub key: String,
    pub value: String,
}

/// One annotation row pushed into the shell (audit F09/U03).
///
/// As with layers, the `int` id is display-only; the adapter keeps the ordered
/// `AnnotationId` list so a visibility toggle / delete maps back exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationRowUi {
    pub id: i32,
    pub kind: String,
    pub text: String,
    /// Effective visibility the scene should honour.
    pub visible: bool,
    /// Whether a temporary session override is active for this annotation.
    pub overridden: bool,
    /// Whether this annotation is the current management selection.
    pub selected: bool,
}

/// Annotation management panel snapshot (F09) plus the active tool state (F07).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnnotationPanelState {
    pub rows: Vec<AnnotationRowUi>,
    /// Number of annotations currently hidden by a session override.
    pub hidden_count: usize,
    /// Explicit empty-state text shown when there are no annotations.
    pub empty_label: String,
    /// Whether an annotation creation tool is running (drives confirm/cancel).
    pub tool_active: bool,
    /// Whether the active tool has all its required parameters.
    pub tool_can_confirm: bool,
    /// Selected kind index for the tool selector.
    pub tool_kind_index: i32,
    /// Human-facing step text from the tool state machine; empty when idle.
    pub tool_step_label: String,
    /// Whether the tool's text payload has been supplied (text/leader kinds).
    pub text_supplied: bool,
    /// Whether the active kind needs a text payload at all.
    pub requires_text: bool,
}

impl AnnotationPanelState {
    /// Build the panel state from real management rows plus the tool preview.
    pub fn from_rows(
        rows: &[cad_app::AnnotationRow],
        preview: Option<&cad_app::AnnotationPreview>,
        empty_label: impl Into<String>,
    ) -> Self {
        let (active, can_confirm, kind_index, step, text_supplied, requires_text) = match preview {
            Some(preview) => (
                true,
                preview.can_confirm(),
                preview.kind.index() as i32,
                preview.status_line(),
                preview.text_supplied,
                preview.requires_text,
            ),
            None => (false, false, 0, String::new(), false, false),
        };
        AnnotationPanelState {
            rows: rows
                .iter()
                .map(|row| AnnotationRowUi {
                    // Display-only; the adapter keeps the exact ids in order.
                    id: (row.id.0 & 0xFFFF_FFFF) as i32,
                    kind: row.kind_label.to_string(),
                    text: row.text.clone(),
                    visible: row.visible,
                    overridden: row.overridden,
                    selected: row.selected,
                })
                .collect(),
            hidden_count: rows.iter().filter(|r| r.is_hidden()).count(),
            empty_label: empty_label.into(),
            tool_active: active,
            tool_can_confirm: can_confirm,
            tool_kind_index: kind_index,
            tool_step_label: step,
            text_supplied,
            requires_text,
        }
    }
}

/// Properties-panel snapshot for the current selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyPanelState {
    pub rows: Vec<PropertyRowUi>,
    /// Count of selected objects (0 = empty state).
    pub count: usize,
    /// Explicit empty-state text when nothing is selected.
    pub empty_label: String,
    /// Text describing keys whose values differ across a multi-select.
    pub mixed_label: String,
}

impl PropertyPanelState {
    /// Build the panel state from a real [`cad_app::SelectionProperties`].
    pub fn from_properties(
        properties: &cad_app::SelectionProperties,
        empty_label: impl Into<String>,
        mixed_label: impl Fn(&[&'static str]) -> String,
    ) -> Self {
        if properties.empty {
            return PropertyPanelState {
                rows: Vec::new(),
                count: 0,
                empty_label: empty_label.into(),
                mixed_label: String::new(),
            };
        }
        PropertyPanelState {
            rows: properties
                .rows
                .iter()
                .map(|row| PropertyRowUi {
                    key: row.key.to_string(),
                    value: row.value.clone(),
                })
                .collect(),
            count: properties.count,
            empty_label: empty_label.into(),
            mixed_label: if properties.mixed_keys.is_empty() {
                String::new()
            } else {
                mixed_label(&properties.mixed_keys)
            },
        }
    }
}

/// Layout/locale configuration for the shell.
#[derive(Debug, Clone)]
pub struct UiConfiguration {
    pub compact: bool,
    pub locale: String,
    pub safe_insets: [f64; 4],
    pub application_title: String,
    pub document: DocumentId,
    pub viewport: ViewportId,
    /// Initial logical window size; hosts keep it in sync with the surface.
    pub logical_size: [f64; 2],
}

impl Default for UiConfiguration {
    fn default() -> Self {
        UiConfiguration {
            compact: false,
            locale: "zh-CN".to_string(),
            safe_insets: [0.0; 4],
            application_title: "yacr CAD".to_string(),
            document: DocumentId(0),
            viewport: ViewportId(0),
            logical_size: [1280.0, 800.0],
        }
    }
}

/// Receives commands emitted by the UI; the application executes them.
pub trait UiCommandSink: 'static {
    fn send(&mut self, command: Command) -> CadResult<()>;
}

/// Push every catalog-driven chrome label/model into the shell (N01).
///
/// This is the single place that maps catalog keys to Slint properties, so a
/// locale switch and the initial construction cannot drift. Combobox models are
/// built from the application's authoritative `ALL` orderings, which is also how
/// a localized label maps back to a kind without relying on Chinese-only
/// `from_label`.
fn apply_chrome(ui: &YacrWindow, messages: &MessageSource) {
    // Toolbar + mode.
    ui.set_open_label(messages.text("file.open", &[]).into());
    ui.set_fit_label(messages.text("toolbar.fit", &[]).into());
    ui.set_undo_label(messages.text("toolbar.undo", &[]).into());
    ui.set_redo_label(messages.text("toolbar.redo", &[]).into());
    ui.set_export_label(messages.text("toolbar.export", &[]).into());
    ui.set_import_label(messages.text("toolbar.import", &[]).into());
    ui.set_diagnostics_label(messages.text("toolbar.diagnostics", &[]).into());
    ui.set_mode_label(messages.text("mode.enhanced", &[]).into());

    // Measurement panel.
    ui.set_measurement_panel_label(messages.text("measure.panel", &[]).into());
    ui.set_measure_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_measure_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_measurement_kind_labels(string_model(&status::measurement_kind_labels(messages)));

    // Annotation panel.
    ui.set_annotation_panel_label(messages.text("annotation.panel", &[]).into());
    ui.set_annotate_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_annotate_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_annotation_text_placeholder(messages.text("annotation.text_placeholder", &[]).into());
    ui.set_annotation_delete_label(messages.text("annotation.delete", &[]).into());
    ui.set_annotation_kind_labels(string_model(&status::annotation_kind_labels(messages)));

    // Layer + property panels.
    ui.set_layer_panel_label(messages.text("layers.panel", &[]).into());
    ui.set_layer_restore_label(messages.text("layers.restore", &[]).into());
    ui.set_layer_overridden_marker(messages.text("layers.overridden_marker", &[]).into());
    ui.set_layer_empty_label(messages.text("layers.empty", &[]).into());
    ui.set_property_panel_label(messages.text("properties.panel", &[]).into());
    ui.set_property_clear_label(messages.text("properties.clear_selection", &[]).into());
    ui.set_property_empty_label(messages.text("properties.empty", &[]).into());

    // Backend choice model and diagnostics drawer chrome.
    ui.set_backend_labels(string_model(&status::backend_labels(messages)));
    ui.set_diagnostics_drawer_title(messages.text("diagnostics.title", &[]).into());
    ui.set_diagnostics_close_label(messages.text("diagnostics.close", &[]).into());
    ui.set_diagnostics_empty_label(messages.text("diagnostics.empty", &[]).into());

    // Layout panel chrome (F04/U03).
    ui.set_layout_panel_label(messages.text("layout.panel", &[]).into());
    ui.set_layout_model_space_label(messages.text("layout.model_space", &[]).into());
    ui.set_layout_unsupported_marker(messages.text("layout.unsupported_marker", &[]).into());
    ui.set_layout_empty_label(messages.text("layout.empty", &[]).into());

    // View / projection chrome (F13/F14). The standard-view model is built from
    // the shared `StandardView::ALL` ordering so a row index is authoritative.
    ui.set_view_panel_label(messages.text("view.panel", &[]).into());
    ui.set_view_2d_label(messages.text("view.2d", &[]).into());
    ui.set_view_3d_label(messages.text("view.3d", &[]).into());
    ui.set_view_projection_label(messages.text("view.projection", &[]).into());
    ui.set_view_ortho_label(messages.text("view.projection.ortho", &[]).into());
    ui.set_view_perspective_label(messages.text("view.projection.perspective", &[]).into());
    ui.set_standard_view_labels(string_model(&status::standard_view_labels(messages)));

    // Responsive chrome (U01): the grouped-bar and drawer entry labels.
    ui.set_tools_label(messages.text("shell.tools", &[]).into());
    ui.set_drawer_label(messages.text("shell.drawer", &[]).into());
    ui.set_nav_label(messages.text("shell.nav", &[]).into());
    ui.set_view_mode_label(messages.text("shell.mode_view", &[]).into());
    ui.set_work_mode_label(messages.text("shell.mode_work", &[]).into());
}

/// Push the derived responsive geometry into the shell (audit U01/U07).
///
/// This is the wire that consumes [`UiConfiguration::compact`] and the current
/// logical viewport instead of leaving compact dead. It writes only geometry
/// properties, so pushing it never disturbs pushed panel data.
pub fn apply_responsive(ui: &YacrWindow, logical_size: [f64; 2], compact_config: bool) {
    let metrics = ResponsiveMetrics::derive(logical_size, compact_config);
    ui.set_compact_shell(metrics.compact);
    ui.set_phone_shell(metrics.breakpoint == Breakpoint::Phone);
    ui.set_control_height(metrics.control_height);
    ui.set_touch_target(metrics.touch_target);
    ui.set_side_panel_width(metrics.side_panel_width);
    ui.set_side_panel_collapsed(metrics.side_panel_collapsed);
    ui.set_drawer_height(metrics.drawer_height);
    ui.set_show_floating_nav(metrics.breakpoint.shows_floating_nav());
}

/// Build a Slint string model from owned labels.
fn string_model(labels: &[String]) -> slint::ModelRc<slint::SharedString> {
    let values: Vec<slint::SharedString> =
        labels.iter().map(|label| label.as_str().into()).collect();
    slint::ModelRc::new(slint::VecModel::from(values))
}

/// Localized "temporary override N" text for the layer panel.
fn layer_override_label(messages: &MessageSource, count: usize) -> String {
    messages.text("layers.override_count", &[("count", &count.to_string())])
}

/// Localized "N selected" text for the properties panel.
fn selected_count_label(messages: &MessageSource, count: usize) -> String {
    messages.text(
        "properties.selected_count",
        &[("count", &count.to_string())],
    )
}

/// Localized "N hidden" text for the annotation management panel.
fn annotation_hidden_label(messages: &MessageSource, count: usize) -> String {
    messages.text("annotation.hidden_count", &[("count", &count.to_string())])
}

/// Localized "Renderer backend: X" line for the diagnostics drawer.
fn backend_label(messages: &MessageSource, backend: &str) -> String {
    messages.text("backend.label", &[("backend", backend)])
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
}

impl UiHandle {
    fn with(&self, f: impl FnOnce(&YacrWindow)) -> CadResult<()> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        f(&ui);
        Ok(())
    }

    /// Replace the composited CAD frame with a new texture-backed image.
    pub fn set_cad_frame(&self, image: Image) -> CadResult<()> {
        self.with(|ui| ui.set_cad_frame(image))
    }

    /// Push the derived view/observation state into the shell (F13/F14).
    ///
    /// The values come from the authoritative application viewport (see
    /// [`bridge::CadView::sync_from_viewport`]); this only mirrors them so the
    /// 2D/3D and projection affordances show the real state. It also updates the
    /// shared flag the adapter uses to route a 3D drag to `Orbit`.
    pub fn set_view_state(&self, state: ViewStateUi) -> CadResult<()> {
        self.view_3d.set(state.is_3d);
        self.with(|ui| {
            ui.set_view_3d(state.is_3d);
            ui.set_view_perspective(state.perspective);
        })
    }

    pub fn set_status(&self, status: impl Into<slint::SharedString>) -> CadResult<()> {
        let s = status.into();
        self.with(|ui| ui.set_status_label(s))
    }

    pub fn set_work_mode(&self, work: bool) -> CadResult<()> {
        self.with(|ui| ui.set_work_mode(work))
    }

    pub fn set_can_undo(&self, can_undo: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_undo(can_undo))
    }

    /// Independent redo availability (audit U11). Never derived from
    /// `set_can_undo`.
    pub fn set_can_redo(&self, can_redo: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_redo(can_redo))
    }

    /// Push an application [`cad_app::HistoryAvailability`] snapshot verbatim.
    ///
    /// Hosts call this with `HostController::history_availability()` after every
    /// command, so the two flags always reflect the two history stacks — undoing
    /// to empty disables undo while keeping redo enabled.
    pub fn set_history_availability(
        &self,
        availability: cad_app::HistoryAvailability,
    ) -> CadResult<()> {
        self.with(|ui| {
            ui.set_can_undo(availability.can_undo);
            ui.set_can_redo(availability.can_redo);
        })
    }

    /// Push the whole measurement panel state in one call (audit U03/U04).
    pub fn set_measurement_state(&self, state: &MeasurementUiState) -> CadResult<()> {
        self.selected_kind.set(state.kind);
        self.measurement_active.set(state.active);
        let step = state.step_label.clone();
        let unit = state.unit_label.clone();
        self.with(|ui| {
            ui.set_measurement_active(state.active);
            ui.set_measurement_can_confirm(state.can_confirm);
            ui.set_measurement_kind_index(state.kind_index());
            ui.set_measurement_step_label(step.into());
            ui.set_unit_label(unit.into());
        })
    }

    /// Push the layer panel state (audit F03/U03).
    ///
    /// Also records the ordered `LayerId`s so a later toggle callback can map
    /// its row index back to the real layer without a lossy cast.
    pub fn set_layer_state(&self, state: &LayerPanelState, order: &[LayerId]) -> CadResult<()> {
        *self.layer_order.borrow_mut() = order.to_vec();
        let rows: Vec<LayerRow> = state
            .rows
            .iter()
            .map(|row| LayerRow {
                id: row.id,
                name: row.name.clone().into(),
                visible: row.visible,
                overridden: row.overridden,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let override_count = state.override_count as i32;
        let empty = state.empty_label.clone();
        self.layer_override_count.set(override_count);
        let messages = self.messages.borrow().clone();
        let override_label = layer_override_label(&messages, state.override_count);
        self.with(|ui| {
            ui.set_layer_rows(model);
            ui.set_layer_override_count(override_count);
            ui.set_layer_override_label(override_label.into());
            ui.set_layer_empty_label(empty.into());
        })
    }

    /// Push the layout (paper-space) list state (audit F04/U03).
    ///
    /// Rows come from the database's real layout table via
    /// [`bridge::layout_descriptors`]; until a host pushes them the panel shows
    /// its explicit empty state, never a fabricated layout. Also records the
    /// ordered `LayoutId`s so a later switch callback can map a row index back to
    /// the exact layout without a lossy cast.
    pub fn set_layout_state(
        &self,
        state: &LayoutPanelState,
        order: &[cad_domain::LayoutId],
    ) -> CadResult<()> {
        *self.layout_order.borrow_mut() = order.to_vec();
        let rows: Vec<LayoutRow> = state
            .rows
            .iter()
            .map(|row| LayoutRow {
                id: row.id,
                name: row.name.clone().into(),
                supported: row.supported,
                reason: row.reason.clone().into(),
                viewport_count: row.viewport_count,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let active = state.active_index.unwrap_or(-1);
        let empty = state.empty_label.clone();
        self.with(|ui| {
            ui.set_layout_rows(model);
            ui.set_layout_active_index(active);
            ui.set_layout_empty_label(empty.into());
        })
    }

    /// Push the read-only properties panel state (audit F05/U03).
    pub fn set_property_state(&self, state: &PropertyPanelState) -> CadResult<()> {
        let rows: Vec<PropertyRow> = state
            .rows
            .iter()
            .map(|row| PropertyRow {
                key: row.key.clone().into(),
                value: row.value.clone().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let count = state.count as i32;
        let empty = state.empty_label.clone();
        let mixed = state.mixed_label.clone();
        self.selection_count.set(count);
        let messages = self.messages.borrow().clone();
        let selected_label = selected_count_label(&messages, state.count);
        self.with(|ui| {
            ui.set_property_rows(model);
            ui.set_selection_count(count);
            ui.set_property_selected_label(selected_label.into());
            ui.set_property_empty_label(empty.into());
            ui.set_property_mixed_label(mixed.into());
        })
    }

    /// Push the annotation management + tool panel state (audit F07/F09/U03).
    ///
    /// Also records the ordered `AnnotationId`s so a later visibility/delete
    /// callback can map its row index back to the real annotation.
    pub fn set_annotation_state(
        &self,
        state: &AnnotationPanelState,
        order: &[AnnotationId],
    ) -> CadResult<()> {
        *self.annotation_order.borrow_mut() = order.to_vec();
        if let Some(kind) = AnnotationToolKind::from_index(state.tool_kind_index) {
            self.selected_annotation_kind.set(kind);
        }
        self.annotation_active.set(state.tool_active);
        let rows: Vec<AnnotationRow> = state
            .rows
            .iter()
            .map(|row| AnnotationRow {
                id: row.id,
                kind: row.kind.clone().into(),
                text: row.text.clone().into(),
                visible: row.visible,
                overridden: row.overridden,
                selected: row.selected,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let hidden = state.hidden_count as i32;
        self.annotation_hidden_count.set(hidden);
        let empty = state.empty_label.clone();
        let step = state.tool_step_label.clone();
        let messages = self.messages.borrow().clone();
        let hidden_label = annotation_hidden_label(&messages, state.hidden_count);
        self.with(|ui| {
            ui.set_annotation_rows(model);
            ui.set_annotation_hidden_count(hidden);
            ui.set_annotation_hidden_label(hidden_label.into());
            ui.set_annotation_empty_label(empty.into());
            ui.set_annotation_tool_active(state.tool_active);
            ui.set_annotation_tool_can_confirm(state.tool_can_confirm);
            ui.set_annotation_kind_index(state.tool_kind_index);
            ui.set_annotation_step_label(step.into());
            ui.set_annotation_requires_text(state.requires_text);
            ui.set_annotation_text_supplied(state.text_supplied);
        })
    }

    pub fn set_backend_index(&self, index: i32) -> CadResult<()> {
        self.with(|ui| ui.set_backend_index(index))
    }

    /// Re-apply the catalog for `locale` and update **all** chrome labels.
    ///
    /// Returns the resolution actually applied; callers can log a fallback. The
    /// adapter reformats the currently displayed override/selection counts too,
    /// so a live locale switch does not leave a stale Chinese count label. Hosts
    /// that supply their own `empty_label`/`mixed_label` strings should re-push
    /// the panel state with catalog-derived text after switching.
    pub fn set_locale(&self, locale: &str) -> CadResult<LocaleResolution> {
        let messages = MessageSource::from_request(locale);
        let resolution = messages.resolution().clone();
        *self.messages.borrow_mut() = messages.clone();
        let override_count = self.layer_override_count.get().max(0) as usize;
        let selection_count = self.selection_count.get().max(0) as usize;
        let hidden_count = self.annotation_hidden_count.get().max(0) as usize;
        let override_label = layer_override_label(&messages, override_count);
        let selected_label = selected_count_label(&messages, selection_count);
        let hidden_label = annotation_hidden_label(&messages, hidden_count);
        self.with(|ui| {
            apply_chrome(ui, &messages);
            ui.set_layer_override_label(override_label.into());
            ui.set_property_selected_label(selected_label.into());
            ui.set_annotation_hidden_label(hidden_label.into());
        })?;
        Ok(resolution)
    }

    /// Push the diagnostics drawer state (audit U08).
    ///
    /// The rows come from [`DiagnosticsPanelState`], which a host builds from the
    /// real `cad_diagnostics::DiagnosticsModel`. Until a host pushes one, the
    /// drawer shows its explicit empty state rather than fabricated rows.
    pub fn set_diagnostics_state(&self, state: &DiagnosticsPanelState) -> CadResult<()> {
        let rows: Vec<DiagnosticRow> = state
            .rows
            .iter()
            .map(|row| DiagnosticRow {
                code: row.code.clone().into(),
                severity: row.severity.clone().into(),
                description: row.description.clone().into(),
                object: row.object.clone().into(),
                details: row.details.clone().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let backend = backend_label(&self.messages.borrow().clone(), &state.backend);
        let summary = state.summary.clone();
        let empty = state.empty_label.clone();
        self.with(|ui| {
            ui.set_diagnostics_rows(model);
            ui.set_diagnostics_backend_label(backend.into());
            ui.set_diagnostics_summary_label(summary.into());
            ui.set_diagnostics_empty_label(empty.into());
        })
    }

    /// Open or close the diagnostics drawer without emitting a command.
    pub fn set_diagnostics_open(&self, open: bool) -> CadResult<()> {
        self.with(|ui| ui.set_diagnostics_open(open))
    }

    /// Trigger a redraw without restarting the event loop.
    pub fn request_redraw(&self) -> CadResult<()> {
        self.with(|ui| ui.window().request_redraw())
    }

    /// Current physical size of the window, if it still exists.
    pub fn physical_size(&self) -> Option<slint::PhysicalSize> {
        Some(self.ui.upgrade()?.window().size())
    }
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
}

/// Build the command a shell callback emits for the configured document.
fn command_for(
    id: CommandId,
    document: &DocumentId,
    viewport: ViewportId,
    payload: CommandPayload,
) -> Command {
    Command {
        schema_version: 1,
        id,
        document: document.clone(),
        viewport,
        payload,
    }
}

/// Radians of orbit per logical pixel of drag (a UI navigation constant).
pub const ORBIT_RADIANS_PER_PIXEL: f64 = 0.008;

/// Convert a 3D orbit drag delta (logical pixels) into `(yaw, pitch)` radians.
///
/// Horizontal drag yaws about world `+Z`; vertical drag pitches about the view
/// right axis. The sign is chosen so the scene follows the pointer. Non-finite
/// input yields `(0, 0)` rather than poisoning a camera with NaNs.
pub fn orbit_delta(from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() {
        return (0.0, 0.0);
    }
    (-dx * ORBIT_RADIANS_PER_PIXEL, dy * ORBIT_RADIANS_PER_PIXEL)
}

impl UiAdapter {
    /// Build the shell and connect its callbacks to `sink`.
    pub fn new<S: UiCommandSink>(
        configuration: UiConfiguration,
        sink: S,
        work_mode: bool,
    ) -> CadResult<Self> {
        let ui = YacrWindow::new().map_err(|e| CadError::Invariant(format!("slint: {e}")))?;
        // Kept for closures that need to report state directly (for example the
        // unwired canvas pick), since the command sink has no "set status" verb.
        let ui_weak = ui.as_weak();
        ui.set_application_title(configuration.application_title.clone().into());
        ui.window().set_size(slint::LogicalSize::new(
            configuration.logical_size[0].max(1.0) as f32,
            configuration.logical_size[1].max(1.0) as f32,
        ));
        // The configured locale selects the catalog; no Rust string literals for
        // user-facing text live here (N01). Fallbacks are recorded, never silent.
        let messages = MessageSource::from_request(&configuration.locale);
        if let Some(reason) = messages.resolution().fallback {
            log::info!(
                "locale {:?} resolved to {} ({:?})",
                messages.resolution().requested,
                messages.locale().tag(),
                reason
            );
        }
        apply_chrome(&ui, &messages);
        // Consume the compact config and the initial viewport: the responsive
        // geometry is derived once here and on every resize (audit U01).
        apply_responsive(&ui, configuration.logical_size, configuration.compact);
        ui.set_status_label(messages.text("status.scaffold", &[]).into());
        ui.set_work_mode(work_mode);

        let document = configuration.document.clone();
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));
        let view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>> = Rc::new(RefCell::new(None));
        let pick_mapper: Rc<RefCell<Option<Rc<dyn CanvasPickMapper>>>> =
            Rc::new(RefCell::new(None));
        let layout_switch: Rc<RefCell<Option<Box<dyn LayoutSwitchSink>>>> =
            Rc::new(RefCell::new(None));
        let selected_kind: Rc<Cell<MeasurementToolKind>> =
            Rc::new(Cell::new(MeasurementToolKind::Distance));
        let measurement_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let selected_annotation_kind: Rc<Cell<AnnotationToolKind>> =
            Rc::new(Cell::new(AnnotationToolKind::Text));
        let annotation_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let annotation_order: Rc<RefCell<Vec<AnnotationId>>> = Rc::new(RefCell::new(Vec::new()));
        let layer_order: Rc<RefCell<Vec<LayerId>>> = Rc::new(RefCell::new(Vec::new()));
        let layout_order: Rc<RefCell<Vec<cad_domain::LayoutId>>> =
            Rc::new(RefCell::new(Vec::new()));
        let messages_slot: Rc<RefCell<MessageSource>> = Rc::new(RefCell::new(messages.clone()));
        let layer_override_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let selection_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let annotation_hidden_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let view_3d: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let orbit_last: Rc<Cell<Option<[f64; 2]>>> = Rc::new(Cell::new(None));

        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_open_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::OpenDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_fit_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::FitDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // The Measure button starts (or restarts) the selected algorithm
            // instead of sending a payload-free guess (audit U04/F06).
            let s = shared.clone();
            let doc = document.clone();
            let kind = selected_kind.clone();
            ui.on_measure_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Measure,
                    &doc,
                    viewport,
                    CommandPayload::MeasureTool(kind.get()),
                ));
            });
        }
        {
            // Selecting a kind immediately starts that tool; this is the explicit
            // user choice of algorithm, not a default. The label is localized, so
            // it is mapped back through the same catalog-built ordering rather
            // than the Chinese-only `from_label`.
            let s = shared.clone();
            let doc = document.clone();
            let kind_slot = selected_kind.clone();
            let messages = messages_slot.clone();
            ui.on_measure_kind_selected(move |name| {
                let messages = messages.borrow().clone();
                let Some(kind) = status::measurement_kind_from_label(&messages, name.as_str())
                else {
                    return;
                };
                kind_slot.set(kind);
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Measure,
                    &doc,
                    viewport,
                    CommandPayload::MeasureTool(kind),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_confirm_measurement_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ConfirmMeasurement,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_cancel_measurement_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelMeasurement,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Layer visibility toggle: the panel sends a row index; the adapter
            // resolves it to the exact LayerId from the pushed model order.
            let s = shared.clone();
            let doc = document.clone();
            let order = layer_order.clone();
            ui.on_layer_visibility_toggled(move |index, visible| {
                let layer = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(layer) = layer {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::ToggleLayer,
                        &doc,
                        viewport,
                        CommandPayload::Layer(layer, visible),
                    ));
                }
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_restore_layers_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::RestoreLayers,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Clearing the selection is a real, read-only Select command with an
            // empty payload; the application records it and mutates nothing.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_clear_selection_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Select,
                    &doc,
                    viewport,
                    CommandPayload::Selection(Vec::new()),
                ));
            });
        }
        {
            // A canvas click becomes one `Measure`/`Points` command when the host
            // installed a world mapper. Without one the click cannot be turned
            // into a real point, so it is reported (not silently swallowed).
            let s = shared.clone();
            let doc = document.clone();
            let mapper = pick_mapper.clone();
            let report = ui_weak.clone();
            let active = measurement_active.clone();
            let annotating = annotation_active.clone();
            let messages = messages_slot.clone();
            ui.on_canvas_pick(move |x, y| {
                // Ordinary navigation clicks must stay silent; only an active
                // capture tool turns a click into a pick.
                let measure = active.get();
                let annotate = annotating.get();
                if !measure && !annotate {
                    return;
                }
                let world = mapper
                    .borrow()
                    .as_ref()
                    .and_then(|mapper| mapper.to_world([x as f64, y as f64]));
                match world {
                    Some(world) => {
                        let (id, payload) = if annotate {
                            (
                                CommandId::AppendAnnotationPoints,
                                CommandPayload::AppendAnnotationPoints(vec![world]),
                            )
                        } else {
                            (CommandId::Measure, CommandPayload::Points(vec![world]))
                        };
                        let _ = s
                            .borrow_mut()
                            .send(command_for(id, &doc, viewport, payload));
                    }
                    None => {
                        // Explicit, never a silent no-op: no mapper (or an
                        // unresolved point) means the pick is not wired on this
                        // host yet.
                        if let Some(ui) = report.upgrade() {
                            ui.set_status_label(
                                messages.borrow().text("status.pick_unwired", &[]).into(),
                            );
                        }
                    }
                }
            });
        }
        {
            // Annotate starts (or restarts) the selected annotation kind instead
            // of sending a payload-free command (audit U04/F07).
            let s = shared.clone();
            let doc = document.clone();
            let kind = selected_annotation_kind.clone();
            ui.on_annotate_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::BeginAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationTool(kind.get()),
                ));
            });
        }
        {
            // Selecting a kind immediately starts that tool; this is the explicit
            // user choice, not a default. Localized labels map back through the
            // catalog-built ordering.
            let s = shared.clone();
            let doc = document.clone();
            let kind_slot = selected_annotation_kind.clone();
            let messages = messages_slot.clone();
            ui.on_annotation_kind_selected(move |name| {
                let messages = messages.borrow().clone();
                let Some(kind) = status::annotation_kind_from_label(&messages, name.as_str())
                else {
                    return;
                };
                kind_slot.set(kind);
                let _ = s.borrow_mut().send(command_for(
                    CommandId::BeginAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationTool(kind),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_confirm_annotation_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ConfirmAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_cancel_annotation_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Text payload for text/leader annotate tools. The shell only sends
            // it when the active kind requires text; the application validates
            // that again.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_annotation_text_edited(move |text| {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::AnnotationText,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationText(text.to_string()),
                ));
            });
        }
        {
            // Row click selects the annotation for edit/delete. The adapter
            // resolves the row index to the exact id from the pushed order.
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_selected(move |index| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SelectAnnotation,
                    &doc,
                    viewport,
                    CommandPayload::SelectAnnotation(id),
                ));
            });
        }
        {
            // Delete the selected annotation row (resolved through the order).
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_delete_requested(move |index| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(id) = id {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::DeleteAnnotationById,
                        &doc,
                        viewport,
                        CommandPayload::DeleteAnnotation(id),
                    ));
                }
            });
        }
        {
            // Annotation visibility toggle: session state only, no transaction.
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_visibility_toggled(move |index, visible| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(id) = id {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::SetAnnotationVisibility,
                        &doc,
                        viewport,
                        CommandPayload::AnnotationVisibility(id, visible),
                    ));
                }
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_undo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Undo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_redo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Redo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Layout switch (F04). The panel sends a row index (-1 = model
            // space); the adapter resolves it to the exact LayoutId from the
            // pushed order. The switch is routed through the shared app command
            // path (`SwitchSpace`), which validates the layout against the
            // drawing and records the active space in the session. A host that
            // installed the legacy `LayoutSwitchSink` is still notified directly
            // (source compatibility); in that case the command is not also sent,
            // so the two paths cannot fight.
            let s = shared.clone();
            let doc = document.clone();
            let order = layout_order.clone();
            let report = ui_weak.clone();
            let messages = messages_slot.clone();
            let layout_switch = layout_switch.clone();
            ui.on_layout_selected(move |index| {
                let space = if index < 0 {
                    Some(cad_representation::SpaceSelection::Model)
                } else {
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| order.borrow().get(i).copied())
                        .map(cad_representation::SpaceSelection::Paper)
                };
                let Some(space) = space else {
                    // No such row in the pushed model: report, do not act.
                    if let Some(ui) = report.upgrade() {
                        ui.set_status_label(
                            messages.borrow().text("layout.switch_unwired", &[]).into(),
                        );
                    }
                    return;
                };
                if let Some(sink) = layout_switch.borrow_mut().as_mut() {
                    sink.select(space);
                    return;
                }
                let space_id = match space {
                    cad_representation::SpaceSelection::Model => SpaceId::Model,
                    cad_representation::SpaceSelection::Paper(id) => SpaceId::Paper(id),
                };
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchSpace,
                    &doc,
                    viewport,
                    CommandPayload::Space(space_id),
                ));
            });
        }
        {
            // 2D/3D toggle (F13/F14). A single command so the session keeps the
            // saved camera for a lossless round trip; the shell reflects the
            // result from the host-pushed `view-3d` property.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_toggle_view_mode_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Switch2d3d,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Standard view (F13). The pushed model is built from
            // `StandardView::ALL`, so the localized label maps back to the exact
            // view without a second ordering list; an unknown label is ignored.
            let s = shared.clone();
            let doc = document.clone();
            let messages = messages_slot.clone();
            ui.on_standard_view_selected(move |name| {
                let messages = messages.borrow().clone();
                if let Some(view) = status::standard_view_from_label(&messages, name.as_str()) {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::StandardView,
                        &doc,
                        viewport,
                        CommandPayload::StandardView(view),
                    ));
                }
            });
        }
        {
            // Explicit orthographic/perspective toggle (F13), distinct from the
            // 2D/3D toggle: it keeps the target and adjusts the projection.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_toggle_projection_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchProjection,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_backend_selected(move |name| {
                let choice = match name.as_str() {
                    "WebGPU" => cad_app::BackendChoice::WebGpu,
                    "WebGL2" => cad_app::BackendChoice::WebGl2,
                    _ => cad_app::BackendChoice::Auto,
                };
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchBackend,
                    &doc,
                    viewport,
                    CommandPayload::Backend(choice),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_export_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ExportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_import_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ImportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_diagnostics_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Diagnostics,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Closing the drawer is a pure presentation action; it emits no
            // command (audit U08: the bar stays simple, the drawer is explicit).
            let ui_weak = ui.as_weak();
            ui.on_diagnostics_closed(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_diagnostics_open(false);
                }
            });
        }
        {
            let input = view_input.clone();
            let s = shared.clone();
            let doc = document.clone();
            let is_3d = view_3d.clone();
            let orbit_last = orbit_last.clone();
            ui.on_pointer_input(move |kind, button, x, y| {
                let x = x as f64;
                let y = y as f64;
                // In 3D mode a left-button drag orbits the view through the
                // shared command path. Other buttons (middle/right) and scroll
                // still route to the host's `ViewInput`, so a host can keep 3D
                // pan on the middle button.
                if is_3d.get() && button == 1 {
                    match kind {
                        0 => {
                            orbit_last.set(Some([x, y]));
                            return;
                        }
                        2 => {
                            if let Some(from) = orbit_last.get() {
                                let (yaw, pitch) = orbit_delta(from, [x, y]);
                                orbit_last.set(Some([x, y]));
                                if yaw != 0.0 || pitch != 0.0 {
                                    let _ = s.borrow_mut().send(command_for(
                                        CommandId::Orbit,
                                        &doc,
                                        viewport,
                                        CommandPayload::Orbit { yaw, pitch },
                                    ));
                                }
                                return;
                            }
                            // No drag started in 3D: fall through to the host.
                        }
                        1 | 3 => {
                            orbit_last.set(None);
                            return;
                        }
                        _ => {}
                    }
                }
                if let Some(input) = input.borrow().as_ref() {
                    input.pointer(kind, button, x, y);
                }
            });
        }
        {
            let input = view_input.clone();
            ui.on_scroll_input(move |dx, dy| {
                if let Some(input) = input.borrow().as_ref() {
                    input.scroll(dx as f64, dy as f64);
                }
            });
        }

        Ok(UiAdapter {
            configuration,
            ui,
            view_input,
            pick_mapper,
            layout_switch,
            selected_kind,
            measurement_active,
            selected_annotation_kind,
            annotation_active,
            annotation_order,
            layer_order,
            layout_order,
            messages: messages_slot,
            layer_override_count,
            selection_count,
            annotation_hidden_count,
            view_3d,
        })
    }

    /// Route canvas input to the CAD view; call once the view exists.
    pub fn set_view_input(&self, input: Rc<dyn ViewInput>) {
        *self.view_input.borrow_mut() = Some(input);
    }

    /// Install the logical-pixel → world converter used for measurement picks.
    ///
    /// Until this is installed, a canvas pick reports "unwired" in the status
    /// line instead of dropping the click. Hosts should derive the point from
    /// the authoritative viewport (`Viewport::screen_to_world`).
    pub fn set_canvas_pick_mapper(&self, mapper: Rc<dyn CanvasPickMapper>) {
        *self.pick_mapper.borrow_mut() = Some(mapper);
    }

    /// Install the sink that switches model/paper space when the layout panel
    /// emits `layout-selected` (F04/U03).
    ///
    /// Until this is installed, clicking a layout reports "layout switch not
    /// wired" in the status line instead of silently doing nothing.
    pub fn set_layout_switch_sink(&self, sink: Box<dyn LayoutSwitchSink>) {
        *self.layout_switch.borrow_mut() = Some(sink);
    }

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle {
            ui: self.ui.as_weak(),
            messages: self.messages.clone(),
            selected_kind: self.selected_kind.clone(),
            measurement_active: self.measurement_active.clone(),
            selected_annotation_kind: self.selected_annotation_kind.clone(),
            annotation_active: self.annotation_active.clone(),
            annotation_order: self.annotation_order.clone(),
            layer_order: self.layer_order.clone(),
            layout_order: self.layout_order.clone(),
            layer_override_count: self.layer_override_count.clone(),
            selection_count: self.selection_count.clone(),
            annotation_hidden_count: self.annotation_hidden_count.clone(),
            view_3d: self.view_3d.clone(),
        }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }

    /// Resize the window so its logical size is `logical_size` at `scale`.
    ///
    /// Mirrors the size back into the recorded configuration and lets a host
    /// that owns the platform surface (Android/web) resize it to the same
    /// physical pixels, so the CAD frame matches the canvas rectangle (U07).
    /// Returns the physical size so the caller can log or forward it.
    pub fn fit_window_to_logical(
        &mut self,
        logical_size: [f64; 2],
        scale: f32,
    ) -> slint::PhysicalSize {
        let physical = self.fitted_physical_size(logical_size, scale);
        self.ui.window().set_size(physical);
        self.configuration.logical_size = logical_size;
        physical
    }

    /// Physical size the window must have to fit `logical_size` at `scale`.
    fn fitted_physical_size(&self, logical_size: [f64; 2], scale: f32) -> slint::PhysicalSize {
        let width = (logical_size[0].max(1.0) as f32 * scale).round() as u32;
        let height = (logical_size[1].max(1.0) as f32 * scale).round() as u32;
        slint::PhysicalSize::new(width.max(1), height.max(1))
    }

    /// The measurement algorithm currently selected in the shell.
    pub fn selected_measurement_kind(&self) -> MeasurementToolKind {
        self.selected_kind.get()
    }

    /// The annotation kind currently selected in the shell (Annotate button and
    /// canvas picks agree with the selector).
    pub fn selected_annotation_kind(&self) -> AnnotationToolKind {
        self.selected_annotation_kind.get()
    }

    /// Show the window and run the platform event loop.
    ///
    /// On Android this is called after `slint::android::init()`; on desktop it
    /// is the winit event loop; on the web it hands over to the browser event
    /// loop (the call may not return — see `apps/app-web`).
    pub fn run(&self) -> CadResult<()> {
        self.ui
            .show()
            .map_err(|e| CadError::Invariant(format!("slint show: {e}")))?;
        slint::run_event_loop().map_err(|e| CadError::Invariant(format!("slint loop: {e}")))?;
        Ok(())
    }

    /// The component, for hosts that need to install a rendering notifier.
    pub fn component(&self) -> &YacrWindow {
        &self.ui
    }
}

/// Ask Slint to render with wgpu so a CAD renderer can share the device.
///
/// Must be called before creating any window. On Android the backend is Skia
/// behind wgpu (`unstable-wgpu-30`), which is what makes a shared texture
/// possible at all; on the web `web::select_backend` chooses WebGPU or WebGL2.
/// See `docs/render-backends.md`.
pub fn select_wgpu_backend() -> CadResult<()> {
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::default())
        .select()
        .map_err(|e| CadError::GpuFailure(format!("slint wgpu backend: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_definition_mentions_a_cad_frame_property() {
        assert!(UI_DEFINITION.contains("cad-frame"));
        assert!(ZH_CN_MESSAGES.contains('{'));
    }

    #[test]
    fn shell_exposes_independent_redo_and_measurement_affordances() {
        // U11: redo must not share the undo flag.
        assert!(UI_DEFINITION.contains("can-redo"));
        assert!(UI_DEFINITION.contains("can-undo"));
        // U03/U04: kind selector + confirm/cancel + step/unit status.
        assert!(UI_DEFINITION.contains("measure-kind-selected"));
        assert!(UI_DEFINITION.contains("confirm-measurement-requested"));
        assert!(UI_DEFINITION.contains("cancel-measurement-requested"));
        assert!(UI_DEFINITION.contains("canvas-pick"));
    }

    #[test]
    fn shell_is_responsive_and_consumes_compact_config() {
        // U01: the arrangement is width-driven, not a single hardcoded row.
        for property in [
            "compact-shell",
            "phone-shell",
            "control-height",
            "touch-target",
            "side-panel-width",
            "side-panel-collapsed",
            "drawer-height",
            "show-floating-nav",
        ] {
            assert!(
                UI_DEFINITION.contains(property),
                "shell must expose responsive {property}"
            );
        }
        // The three arrangements are all present in one component.
        assert!(UI_DEFINITION.contains("side-panel-open"));
        assert!(UI_DEFINITION.contains("tools-open"));
        // A phone drawer entry and a wide collapsible panel entry exist; the
        // desktop bar and phone bar are separate branches, not a squash.
        assert!(UI_DEFINITION.contains("phone-shell && root.tools-open"));
        assert!(UI_DEFINITION.contains("show-floating-nav"));
    }

    #[test]
    fn shell_reaches_every_merged_panel_and_layout_list() {
        // U03: every already-merged panel is reachable from the shell.
        for marker in [
            "layer-rows",
            "property-rows",
            "annotation-rows",
            "diagnostics-rows",
            "layout-rows",
            "layout-selected",
            "layout-empty-label",
        ] {
            assert!(
                UI_DEFINITION.contains(marker),
                "missing panel marker {marker}"
            );
        }
    }

    #[test]
    fn layout_rows_and_active_index_are_pushed_faithfully() {
        let mut state = LayoutPanelState::default();
        state.rows.push(LayoutRowUi {
            id: 7,
            name: "Sheet A".into(),
            supported: true,
            reason: String::new(),
            viewport_count: 2,
        });
        state.rows.push(LayoutRowUi {
            id: 8,
            name: "Sheet B".into(),
            supported: false,
            reason: "unsupported viewport".into(),
            viewport_count: 1,
        });
        state.active_index = Some(1);
        assert_eq!(state.rows[1].name, "Sheet B");
        assert!(!state.rows[1].supported);
        assert_eq!(state.active_index, Some(1));

        // Model space has no active layout row.
        state.active_index = None;
        assert_eq!(state.active_index, None);
    }

    #[test]
    fn shell_exposes_layer_and_property_panels() {
        // F03: layer list with visibility toggles + restore affordance.
        assert!(UI_DEFINITION.contains("layer-rows"));
        assert!(UI_DEFINITION.contains("layer-visibility-toggled"));
        assert!(UI_DEFINITION.contains("restore-layers-requested"));
        // F05: read-only properties + explicit empty/mixed states.
        assert!(UI_DEFINITION.contains("property-rows"));
        assert!(UI_DEFINITION.contains("property-empty-label"));
        assert!(UI_DEFINITION.contains("property-mixed-label"));
        assert!(UI_DEFINITION.contains("clear-selection-requested"));
    }

    #[test]
    fn layout_panel_state_mirrors_descriptors_and_marks_the_active_row() {
        use cad_domain::LayoutId;
        use cad_representation::{LayoutDescriptor, SpaceSelection};

        let descriptors = vec![
            LayoutDescriptor {
                id: LayoutId(1),
                name: "Layout1".into(),
                supported: true,
                reason: String::new(),
                viewport_count: 1,
            },
            LayoutDescriptor {
                id: LayoutId(2),
                name: "Layout2".into(),
                supported: false,
                reason: "viewport scale must be positive".into(),
                viewport_count: 1,
            },
        ];
        let state = LayoutPanelState::from_descriptors(
            &descriptors,
            SpaceSelection::Paper(LayoutId(2)),
            "无布局",
        );
        assert_eq!(state.rows.len(), 2);
        assert!(state.rows[0].supported);
        assert!(!state.rows[1].supported);
        assert_eq!(state.active_index, Some(1));

        // Model space selects no row.
        let model =
            LayoutPanelState::from_descriptors(&descriptors, SpaceSelection::Model, "无布局");
        assert_eq!(model.active_index, None);

        // Empty input yields an empty model, not a fabricated row.
        let empty = LayoutPanelState::from_descriptors(&[], SpaceSelection::Model, "无布局");
        assert!(empty.rows.is_empty());
        assert_eq!(empty.empty_label, "无布局");
    }

    #[test]
    fn layer_panel_state_mirrors_real_rows() {
        use cad_app::layers::{LayerOverrideSet, LayerRow};
        use cad_domain::LayerId;

        let mut overrides = LayerOverrideSet::new();
        overrides.set(LayerId(1), false);
        let rows = vec![
            LayerRow {
                id: LayerId(0),
                name: "0".into(),
                database_visible: true,
                override_visible: None,
                effective_visible: true,
            },
            LayerRow {
                id: LayerId(1),
                name: "WALLS".into(),
                database_visible: true,
                override_visible: Some(false),
                effective_visible: false,
            },
        ];
        let state = LayerPanelState::from_rows(&rows, "无图层");
        assert_eq!(state.rows.len(), 2);
        assert_eq!(state.override_count, 1);
        assert!(!state.rows[1].visible);
        assert!(state.rows[1].overridden);
        assert_eq!(state.empty_label, "无图层");

        // Empty input yields an empty model, not a fabricated row.
        let empty = LayerPanelState::from_rows(&[], "无图层");
        assert!(empty.rows.is_empty());
        assert_eq!(empty.override_count, 0);
    }

    #[test]
    fn property_panel_state_tracks_empty_and_mixed() {
        use cad_app::SelectionProperties;

        let empty = SelectionProperties::default();
        let state = PropertyPanelState::from_properties(&empty, "未选择", |_| String::new());
        assert_eq!(state.count, 0);
        assert!(state.rows.is_empty());
        assert_eq!(state.empty_label, "未选择");

        let mixed = SelectionProperties {
            count: 2,
            rows: Vec::new(),
            mixed_keys: vec!["id", "length"],
            empty: false,
        };
        let state = PropertyPanelState::from_properties(&mixed, "未选择", |keys| {
            format!("多值: {}", keys.join(","))
        });
        assert_eq!(state.count, 2);
        assert_eq!(state.mixed_label, "多值: id,length");
    }

    #[test]
    fn measurement_ui_state_tracks_the_preview() {
        use cad_app::MeasurementTool;
        let mut tool = MeasurementTool::new(MeasurementToolKind::Area);

        // Idle: nothing active, no confirm.
        let idle = MeasurementUiState::from_preview(None, "drawing units");
        assert!(!idle.active);
        assert!(!idle.can_confirm);
        assert_eq!(idle.step_label, "");
        assert_eq!(idle.unit_label, "drawing units");

        // Capturing: active but not confirmable yet.
        tool.push_point(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let capturing = MeasurementUiState::from_preview(Some(&tool.preview()), "m");
        assert!(capturing.active);
        assert!(!capturing.can_confirm);
        assert_eq!(capturing.kind, MeasurementToolKind::Area);
        assert_eq!(
            capturing.kind_index(),
            MeasurementToolKind::Area.index() as i32
        );

        // Ready: confirm enabled.
        tool.push_point(Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        tool.push_point(Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        });
        let ready = MeasurementUiState::from_preview(Some(&tool.preview()), "m");
        assert!(ready.can_confirm);
    }

    #[test]
    fn kind_index_round_trips_through_ui_state() {
        for (index, kind) in MeasurementToolKind::ALL.iter().copied().enumerate() {
            let state = MeasurementUiState {
                kind,
                ..MeasurementUiState::default()
            };
            assert_eq!(state.kind_index() as usize, index);
            assert_eq!(
                MeasurementToolKind::from_index(state.kind_index()),
                Some(kind)
            );
        }
    }

    #[test]
    fn catalog_drives_ui_defaults() {
        // The shell no longer hardcodes its initial labels: they come from the
        // embedded catalog, so both languages stay reachable through config.
        assert_eq!(
            MessageSource::for_locale(Locale::ZhCn).text("file.open", &[]),
            "打开图纸"
        );
        assert_eq!(
            MessageSource::for_locale(Locale::En).text("file.open", &[]),
            "Open drawing"
        );
    }

    #[test]
    fn chrome_is_catalog_driven_not_hardcoded() {
        // Every chrome control binds to a pushed property; the shell holds no
        // literal label. This mirrors `scripts/check-i18n.py` in-crate.
        for property in [
            "fit-label",
            "undo-label",
            "redo-label",
            "export-label",
            "import-label",
            "diagnostics-label",
            "measurement-panel-label",
            "annotation-panel-label",
            "layer-panel-label",
            "property-panel-label",
            "diagnostics-drawer-title",
            "layout-panel-label",
            "view-panel-label",
            "view-2d-label",
            "view-3d-label",
            "view-projection-label",
            "view-ortho-label",
            "view-perspective-label",
            "standard-view-labels",
            "tools-label",
            "drawer-label",
            "nav-label",
        ] {
            assert!(
                UI_DEFINITION.contains(property),
                "shell must expose {property}"
            );
        }
        // A hardcoded CJK literal in the shell is a missing translation.
        assert!(
            !UI_DEFINITION.chars().any(is_cjk),
            "ui/app.slint still contains a hardcoded CJK literal"
        );
    }

    /// True for CJK ideographs and the CJK punctuation used in the chrome.
    fn is_cjk(ch: char) -> bool {
        let code = ch as u32;
        (0x4E00..=0x9FFF).contains(&code) || (0x3000..=0x303F).contains(&code)
    }

    #[test]
    fn diagnostics_panel_state_is_pushable_and_empty_by_default() {
        let state = DiagnosticsPanelState::default();
        assert!(state.is_empty());
        // The drawer needs an explicit empty label; the adapter supplies the
        // catalog one, never a fabricated row.
        assert!(state.rows.is_empty());
    }

    #[test]
    fn shell_reaches_the_view_and_space_controls() {
        // F04/F13/F14: the 2D/3D toggle, standard views and the layout selector
        // are all reachable from the shell and bound to catalog-built models.
        for marker in [
            "view-3d",
            "view-perspective",
            "toggle-view-mode-requested",
            "toggle-projection-requested",
            "standard-view-selected",
            "standard-view-labels",
            "layout-selected",
        ] {
            assert!(
                UI_DEFINITION.contains(marker),
                "shell must expose view/space control {marker}"
            );
        }
    }

    #[test]
    fn orbit_delta_is_proportional_finite_and_signed() {
        // A rightward drag yaws negative; a downward drag pitches positive. The
        // exact sign is a UI choice; the magnitude and finiteness are the
        // contract.
        let (yaw, pitch) = orbit_delta([10.0, 10.0], [30.0, 40.0]);
        assert!(yaw < 0.0);
        assert!(pitch > 0.0);
        assert_eq!(yaw, -20.0 * ORBIT_RADIANS_PER_PIXEL);
        assert_eq!(pitch, 30.0 * ORBIT_RADIANS_PER_PIXEL);
        // No movement is exactly zero, so no command is emitted.
        assert_eq!(orbit_delta([5.0, 5.0], [5.0, 5.0]), (0.0, 0.0));
        // Non-finite input is refused rather than poisoning the camera.
        assert_eq!(orbit_delta([0.0, 0.0], [f64::NAN, 2.0]), (0.0, 0.0));
        assert_eq!(orbit_delta([0.0, 0.0], [1.0, f64::INFINITY]), (0.0, 0.0));
    }

    #[test]
    fn view_state_ui_defaults_are_2d_not_3d() {
        let state = ViewStateUi::default();
        assert!(!state.is_3d);
        assert!(!state.perspective);
    }
}
