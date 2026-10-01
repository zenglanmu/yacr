//! Shared UI boundary: a real Slint shell compositing a wgpu-produced CAD frame.
//!
//! Spec v2.0 §5: a single presentation coordinator (Slint) owns the window and
//! surface; the CAD renderer draws into a texture that Slint composites. The UI
//! never creates GPU objects and never draws CAD entities itself.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::{AnnotationToolKind, Command, CommandId, CommandPayload, MeasurementToolKind};
use cad_domain::{AnnotationId, CadError, CadResult, DocumentId, LayerId, Point3, ViewportId};

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
#[cfg(target_arch = "wasm32")]
pub mod web;
pub use bridge::{
    build_scene, build_scene_with_fonts, build_scene_with_overrides, build_scene_with_space,
    fit_camera, install as install_cad_bridge, layout_descriptors, BridgeCamera, CadView,
    IncomingDocument,
};
pub use i18n::{Locale, LocaleResolution, Message, MessageCatalog, MessageSource};

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

/// Cloneable handle the host uses to push state into the UI.
#[derive(Clone)]
pub struct UiHandle {
    ui: Weak<YacrWindow>,
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
        self.with(|ui| {
            ui.set_layer_rows(model);
            ui.set_layer_override_count(override_count);
            ui.set_layer_empty_label(empty.into());
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
        self.with(|ui| {
            ui.set_property_rows(model);
            ui.set_selection_count(count);
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
        let empty = state.empty_label.clone();
        let step = state.tool_step_label.clone();
        self.with(|ui| {
            ui.set_annotation_rows(model);
            ui.set_annotation_hidden_count(hidden);
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

    /// Re-apply the catalog for `locale` and update the UI labels.
    ///
    /// Returns the resolution actually applied; callers can log a fallback. Full
    /// UI-chrome translation (every button, HTML lang, preference persistence) is
    /// the ui3d/host workstream's job — this is the catalog/core hook.
    pub fn set_locale(&self, locale: &str) -> CadResult<LocaleResolution> {
        let messages = MessageSource::from_request(locale);
        let updated = messages.clone();
        self.with(|ui| {
            ui.set_open_label(updated.text("file.open", &[]).into());
        })?;
        Ok(messages.resolution().clone())
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
        ui.set_open_label(messages.text("file.open", &[]).into());
        ui.set_status_label(messages.text("status.scaffold", &[]).into());
        ui.set_work_mode(work_mode);

        let document = configuration.document.clone();
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));
        let view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>> = Rc::new(RefCell::new(None));
        let pick_mapper: Rc<RefCell<Option<Rc<dyn CanvasPickMapper>>>> =
            Rc::new(RefCell::new(None));
        let selected_kind: Rc<Cell<MeasurementToolKind>> =
            Rc::new(Cell::new(MeasurementToolKind::Distance));
        let measurement_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let selected_annotation_kind: Rc<Cell<AnnotationToolKind>> =
            Rc::new(Cell::new(AnnotationToolKind::Text));
        let annotation_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let annotation_order: Rc<RefCell<Vec<AnnotationId>>> = Rc::new(RefCell::new(Vec::new()));
        let layer_order: Rc<RefCell<Vec<LayerId>>> = Rc::new(RefCell::new(Vec::new()));

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
            // user choice of algorithm, not a default.
            let s = shared.clone();
            let doc = document.clone();
            let kind_slot = selected_kind.clone();
            ui.on_measure_kind_selected(move |name| {
                let Some(kind) = MeasurementToolKind::from_label(name.as_str()) else {
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
                            ui.set_status_label("取点未接线：宿主未提供画布→世界映射".into());
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
            // user choice, not a default.
            let s = shared.clone();
            let doc = document.clone();
            let kind_slot = selected_annotation_kind.clone();
            ui.on_annotation_kind_selected(move |name| {
                let Some(kind) = AnnotationToolKind::from_label(name.as_str()) else {
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
            let input = view_input.clone();
            ui.on_pointer_input(move |kind, button, x, y| {
                if let Some(input) = input.borrow().as_ref() {
                    input.pointer(kind, button, x as f64, y as f64);
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
            selected_kind,
            measurement_active,
            selected_annotation_kind,
            annotation_active,
            annotation_order,
            layer_order,
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

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle {
            ui: self.ui.as_weak(),
            selected_kind: self.selected_kind.clone(),
            measurement_active: self.measurement_active.clone(),
            selected_annotation_kind: self.selected_annotation_kind.clone(),
            annotation_active: self.annotation_active.clone(),
            annotation_order: self.annotation_order.clone(),
            layer_order: self.layer_order.clone(),
        }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
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
}
