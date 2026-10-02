//! Host → shell state connectors for Android.
//!
//! `cad-ui-slint` owns the derived panel models but cannot read the application
//! state itself: the last leg ("application state → shell") is the host's job
//! (`docs/ui.md` §3, `docs/panels.md` §3.2, `docs/annotation-tools.md`). This
//! module is that leg for Android. It mirrors the same getters the web host uses
//! so the two hosts cannot drift.
//!
//! Everything here is a *push* of real application state:
//!
//! * history availability (undo **and** redo, never shared),
//! * the measurement tool panel,
//! * the layer panel plus its ordered `LayerId`s,
//! * the read-only property panel for the current selection,
//! * the annotation management panel plus its ordered `AnnotationId`s,
//! * the layout (paper-space) panel from the database's real layout table,
//! * the diagnostics drawer from the last import report,
//! * the transient render overlays: selection highlight, measurement preview and
//!   annotation preview.
//!
//! When a getter has nothing to report the panels show their **explicit empty
//! state**; no row is ever fabricated. A missing import report is reported as
//! "unverified", not as a clean document.

use super::*;

use cad_app::input::{apply_canvas_metrics, CanvasMetrics};
use cad_app::render_scene::layout_descriptors;
use cad_import_acadrust::ImportReport;
use cad_representation::SpaceSelection;
use cad_ui_slint::{
    AnnotationPanelState, DiagnosticRowUi, DiagnosticsPanelState, LayerPanelState,
    LayoutPanelState, MeasurementUiState, MessageSource, PropertyPanelState,
};

/// Literal host empty labels. Layer/property/annotation empty-state text has no
/// catalog key: the host supplies it through the pushed panel state (see
/// `docs/panels.md` §3.1/§3.2), so these strings are the documented contract.
const LAYER_EMPTY: &str = "无图层";
const PROPERTY_EMPTY: &str = "未选择";
const ANNOTATION_EMPTY: &str = "无批注";

/// The panel state derived from one controller snapshot, without the handle.
///
/// Kept as a plain value so the derivation is unit-testable on the host (no
/// Slint window required). The shell push is a separate, mechanical step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PanelSnapshot {
    pub history: cad_app::HistoryAvailability,
    pub measurement: MeasurementUiState,
    pub layers: LayerPanelState,
    pub layer_ids: Vec<LayerId>,
    pub properties: PropertyPanelState,
    pub annotations: AnnotationPanelState,
    pub annotation_ids: Vec<AnnotationId>,
    pub layouts: LayoutPanelState,
    pub layout_ids: Vec<LayoutId>,
    pub diagnostics: DiagnosticsPanelState,
    /// Selection rendered as a highlight overlay. Empty when nothing is selected;
    /// never a fabricated hit.
    pub selection: cad_app::SelectionSet,
    /// In-progress measurement preview, or `None` when no measurement tool runs.
    pub measurement_preview: Option<cad_app::MeasurementPreview>,
    /// In-progress annotation preview, or `None` when no annotation tool runs.
    pub annotation_preview: Option<cad_app::AnnotationPreview>,
}

/// Build every panel model from the authoritative controller + view.
///
/// Pure with respect to the UI: it reads the controller and returns values; it
/// never dispatches a command or mutates the document.
pub(crate) fn snapshot(
    controller: &Rc<RefCell<HostController>>,
    messages: &MessageSource,
) -> PanelSnapshot {
    let controller_ref = controller.borrow();

    let history = controller_ref.history_availability();
    let measurement = MeasurementUiState::from_preview(
        controller_ref.measurement_preview().as_ref(),
        controller_ref.unit_label(),
    );
    // Transient overlay inputs (docs/ui.md §2): the selection highlight and the
    // active tool previews are pushed on the same snapshot as the panels, so a
    // command, a pick, an open or a tool confirm/cancel keeps them current.
    let selection = controller_ref.selection().clone();
    let measurement_preview = controller_ref.measurement_preview();
    let (layer_rows, layer_ids) = match controller_ref.layer_rows() {
        Ok(rows) => {
            let ids = rows.iter().map(|row| row.id).collect();
            (rows, ids)
        }
        // A getter error (no document) is an explicit empty panel, never a
        // fabricated row.
        Err(_) => (Vec::new(), Vec::new()),
    };
    let layers = LayerPanelState::from_rows(&layer_rows, LAYER_EMPTY);

    let properties = match controller_ref.selection_properties() {
        Ok(properties) => {
            PropertyPanelState::from_properties(&properties, PROPERTY_EMPTY, |keys| {
                format!("多值: {}", keys.join(","))
            })
        }
        Err(_) => PropertyPanelState::from_properties(
            &cad_app::SelectionProperties::default(),
            PROPERTY_EMPTY,
            |keys| format!("多值: {}", keys.join(",")),
        ),
    };

    let (annotation_rows, annotation_ids, annotation_preview) =
        match controller_ref.annotation_rows() {
            Ok(rows) => {
                let ids = rows.iter().map(|row| row.id).collect();
                (rows, ids, controller_ref.annotation_preview())
            }
            Err(_) => (Vec::new(), Vec::new(), controller_ref.annotation_preview()),
        };
    let annotations = AnnotationPanelState::from_rows(
        &annotation_rows,
        annotation_preview.as_ref(),
        ANNOTATION_EMPTY,
    );

    let drawing = controller_ref.drawing();
    let active_space = controller_ref.session.active_space.clone();
    let (layouts, layout_ids) = match drawing.as_ref() {
        Some(drawing) => {
            let descriptors = layout_descriptors(drawing.as_ref());
            let ids = descriptors.iter().map(|d| d.id).collect();
            let active = cad_ui_slint::CadView::space_selection(&active_space)
                .unwrap_or(SpaceSelection::Model);
            (
                LayoutPanelState::from_descriptors(
                    &descriptors,
                    active,
                    messages.text("layout.empty", &[]),
                ),
                ids,
            )
        }
        None => (
            LayoutPanelState::from_descriptors(
                &[],
                SpaceSelection::Model,
                messages.text("layout.empty", &[]),
            ),
            Vec::new(),
        ),
    };

    let diagnostics = import_diagnostics(controller_ref.last_import_report.as_ref(), messages);

    PanelSnapshot {
        history,
        measurement,
        layers,
        layer_ids,
        properties,
        annotations,
        annotation_ids,
        layouts,
        layout_ids,
        diagnostics,
        selection,
        measurement_preview,
        annotation_preview,
    }
}

/// Push every derived panel model into the Slint shell in one place.
///
/// Also mirrors transient render state into the render bridge: the session's
/// temporary layer overrides (`docs/panels.md` §3.1), the selection highlight and
/// the active measurement/annotation previews (`docs/ui.md` §2). A missing view
/// (before `install_cad_bridge` runs) still pushes the panels; the render-mirror
/// sync is simply skipped.
pub(crate) fn push_panel_state(
    controller: &Rc<RefCell<HostController>>,
    handle: &UiHandle,
    view: &SharedView,
) {
    let messages = MessageSource::from_request("zh-CN");
    let snapshot = snapshot(controller, &messages);

    let _ = handle.set_history_availability(snapshot.history);
    let _ = handle.set_measurement_state(&snapshot.measurement);
    let _ = handle.set_layer_state(&snapshot.layers, &snapshot.layer_ids);
    let _ = handle.set_property_state(&snapshot.properties);
    let _ = handle.set_annotation_state(&snapshot.annotations, &snapshot.annotation_ids);
    let _ = handle.set_layout_state(&snapshot.layouts, &snapshot.layout_ids);
    let _ = handle.set_diagnostics_state(&snapshot.diagnostics);

    if let Some(view) = view.borrow().as_ref() {
        let overrides = controller.borrow().session.layer_overrides.clone();
        view.set_layer_overrides(overrides);
        // Selection → highlight overlay; empty selection clears it. The preview
        // setters take `None` when no tool is running, which cancels the overlay
        // instead of leaving a stale one (docs/ui.md §2).
        view.set_selection_highlight(snapshot.selection);
        view.set_measurement_preview(snapshot.measurement_preview);
        view.set_annotation_preview(snapshot.annotation_preview);
    }
}

/// Build the diagnostics drawer from the last import report.
///
/// `cad_domain::Diagnostic` carries a stable `code`, an object id and a
/// human-facing `message`, but **no severity**: the import path never classified
/// one. Rows therefore use the real code, object and message and the neutral
/// "info" severity word; promoting a row to a warning/error here would invent a
/// severity the core did not produce. The drawer summary uses the report's own
/// completeness label, so a missing/partial document is never shown as clean.
///
/// With no report (`None`) the drawer is explicitly empty and its summary says
/// "unverified": a host that has not imported anything cannot claim a complete
/// document.
pub(crate) fn import_diagnostics(
    report: Option<&ImportReport>,
    messages: &MessageSource,
) -> DiagnosticsPanelState {
    let empty_label = messages.text("diagnostics.empty", &[]);
    match report {
        Some(report) => {
            let info = messages.text("diagnostics.severity.info", &[]);
            let rows = report
                .diagnostics
                .iter()
                .map(|diagnostic| DiagnosticRowUi {
                    code: diagnostic.code.clone(),
                    severity: info.clone(),
                    description: diagnostic.message.clone(),
                    object: diagnostic
                        .object
                        .map(|object| object.0.to_string())
                        .unwrap_or_default(),
                    details: String::new(),
                })
                .collect();
            DiagnosticsPanelState {
                rows,
                summary: report.completeness_label(),
                // The active renderer backend is host state, not import state;
                // the status area owns that label.
                backend: String::new(),
                empty_label,
            }
        }
        None => DiagnosticsPanelState {
            rows: Vec::new(),
            summary: messages.text("diagnostics.summary.unverified", &[("count", "0")]),
            backend: String::new(),
            empty_label,
        },
    }
}

/// Apply a new logical surface size / DPI scale to the authoritative viewport.
///
/// Rotation and resize change the canvas rectangle but must not move the camera:
/// `apply_canvas_metrics` only rewrites `logical_size`/`dpi_scale`, leaving the
/// camera target untouched, so the view centre is preserved (audit U07,
/// `docs/input.md` §3). Degenerate sizes are refused so no NaN reaches the
/// screen→world mapping.
///
/// The Activity does not yet forward its `SurfaceHolder` size callback into this
/// build (see `docs/validation-android.md` §8); this is the pure seam it will
/// call. It is wired at start with the configured logical size.
pub(crate) fn apply_surface_size(
    controller: &Rc<RefCell<HostController>>,
    size_logical: [f64; 2],
    dpi_scale: f64,
) -> CadResult<()> {
    let viewport_id = controller.borrow().viewport_id;
    let canvas = CanvasMetrics::new([0.0, 0.0], size_logical, dpi_scale);
    let mut controller = controller.borrow_mut();
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .ok_or_else(|| CadError::InvalidInput("viewport not found".into()))?;
    apply_canvas_metrics(viewport, &canvas)
}

/// Maps a canvas-local logical pixel to a world point for measurement picks.
///
/// Uses the authoritative viewport and the shell's real CAD surface size, so the
/// point agrees with what is rendered and with [`AndroidViewInput`]'s own picks.
pub(crate) struct AndroidCanvasPickMapper {
    controller: Rc<RefCell<HostController>>,
    handle: SharedHandle,
}

impl AndroidCanvasPickMapper {
    pub(crate) fn new(controller: Rc<RefCell<HostController>>, handle: SharedHandle) -> Self {
        AndroidCanvasPickMapper { controller, handle }
    }
}

impl cad_ui_slint::CanvasPickMapper for AndroidCanvasPickMapper {
    fn to_world(&self, logical: [f64; 2]) -> Option<Point3> {
        // Degenerate / non-finite input must not produce a fabricated world
        // point (audit B17); `screen_to_world` already returns `None` for those.
        if !logical[0].is_finite() || !logical[1].is_finite() {
            return None;
        }
        let (canvas_logical, _scale) = self.handle.borrow().as_ref()?.cad_surface_size()?;
        let controller = self.controller.borrow();
        let viewport = controller
            .application
            .workspace
            .viewports
            .get(&controller.viewport_id)?;
        viewport.screen_to_world(logical, canvas_logical)
    }
}
