//! Shared UI boundary: a real Slint shell compositing a wgpu-produced CAD frame.
//!
//! Spec v2.0 §5: a single presentation coordinator (Slint) owns the window and
//! surface; the CAD renderer draws into a texture that Slint composites. The UI
//! never creates GPU objects and never draws CAD entities itself.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::{Command, CommandId, CommandPayload, MeasurementToolKind};
use cad_domain::{CadError, CadResult, DocumentId, Point3, ViewportId};

/// Source of the shared shell, kept for packaging/documentation tooling.
pub const UI_DEFINITION: &str = include_str!("../ui/app.slint");
/// Default (Chinese) UI strings; UI text is externalised per spec §3.5.
pub const ZH_CN_MESSAGES: &str = include_str!("../i18n/zh-CN.json");

slint::include_modules!();

pub mod bridge;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub use bridge::{install as install_cad_bridge, CadView, IncomingDocument};

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

    pub fn set_backend_index(&self, index: i32) -> CadResult<()> {
        self.with(|ui| ui.set_backend_index(index))
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
        ui.set_open_label("打开".into());
        ui.set_measure_label("测量".into());
        ui.set_annotate_label("批注".into());
        ui.set_status_label("就绪".into());
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
            // A canvas click becomes one `Measure`/`Points` command when the host
            // installed a world mapper. Without one the click cannot be turned
            // into a real point, so it is reported (not silently swallowed).
            let s = shared.clone();
            let doc = document.clone();
            let mapper = pick_mapper.clone();
            let report = ui_weak.clone();
            let active = measurement_active.clone();
            ui.on_canvas_pick(move |x, y| {
                // Ordinary navigation clicks must stay silent; only an active
                // measurement turns a click into a pick.
                if !active.get() {
                    return;
                }
                let world = mapper
                    .borrow()
                    .as_ref()
                    .and_then(|mapper| mapper.to_world([x as f64, y as f64]));
                match world {
                    Some(world) => {
                        let _ = s.borrow_mut().send(command_for(
                            CommandId::Measure,
                            &doc,
                            viewport,
                            CommandPayload::Points(vec![world]),
                        ));
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
            let s = shared.clone();
            let doc = document.clone();
            ui.on_annotate_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CreateAnnotation,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
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
        }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }

    /// The measurement algorithm currently selected in the shell.
    pub fn selected_measurement_kind(&self) -> MeasurementToolKind {
        self.selected_kind.get()
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
}
