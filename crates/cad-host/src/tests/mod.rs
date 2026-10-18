//! UI-free synthetic contracts for the host boundary.
//!
//! These drive `HostCore` directly through the platform contracts and the
//! `EngineClient` surface (no UI handle, no window, no GPU), so they are
//! platform-general. They are not render/run acceptance evidence.
use super::*;
use cad_app::{DrawToolKind, Viewport};
use cad_platform::images::ImageLoadReport;
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use std::collections::BTreeMap;

mod box_select;
mod box_select_edge_cases;
mod capture_candidate;
mod compat;
mod engine_config;
mod file_open_save;
mod gate;
mod hover;
mod navigation;
mod p4_origin_dispatch;
mod p4_trace;
mod queue;
mod select;

// --- synthetic platform contracts -------------------------------------------

/// A fake platform whose file events the test enqueues directly.
#[derive(Default)]
struct FakeFiles {
    events: RefCell<Vec<FileEvent>>,
    source: RefCell<Option<FileTarget>>,
}

impl PlatformFiles for FakeFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Ok(())
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.borrow_mut() = target.cloned();
    }
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

/// A fake platform that synthesizes the save target synchronously, the way the
/// browser host does (no native dialog).
#[derive(Default)]
struct SyncSaveFiles {
    events: RefCell<Vec<FileEvent>>,
    written: RefCell<Vec<(String, Vec<u8>)>>,
}

impl PlatformFiles for SyncSaveFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.events.borrow_mut().push(FileEvent::SaveTarget {
            purpose,
            target: FileTarget {
                display_name: default_name.to_string(),
                opaque: format!("download://{default_name}"),
            },
        });
        Ok(())
    }
    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        self.written
            .borrow_mut()
            .push((target.display_name.clone(), bytes.to_vec()));
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

struct FakeConfig;
impl ConfigStore for FakeConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, _text: &str) -> CadResult<()> {
        Ok(())
    }
}

/// A config store that records every persisted preference projection, so a
/// clear's durable write can be asserted without a UI.
#[derive(Default)]
struct RecordingConfig {
    persisted: RefCell<Vec<String>>,
}

impl ConfigStore for RecordingConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, text: &str) -> CadResult<()> {
        self.persisted.borrow_mut().push(text.to_string());
        Ok(())
    }
}

struct FakeSurface;
impl SurfaceSource for FakeSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        Some(SurfaceMetrics {
            logical: [800.0, 600.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        })
    }
}

fn options() -> HostOptions {
    HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: [800.0, 600.0],
    }
}

fn capabilities(import_worker: bool) -> HostCapabilities {
    HostCapabilities {
        open: true,
        new: true,
        save: true,
        plot: true,
        import_worker,
    }
}

fn core_with_caps(files: Rc<dyn PlatformFiles>, capabilities: HostCapabilities) -> HostCore {
    let platform = HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities,
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap()
}

fn core_with(files: Rc<FakeFiles>, import_worker: bool) -> HostCore {
    core_with_caps(files, capabilities(import_worker))
}

fn core() -> HostCore {
    core_with(Rc::new(FakeFiles::default()), false)
}

/// A core whose session has the idle hover preview explicitly pinned ON (it now
/// defaults on, but the pin keeps these contracts independent of the default)
/// and the dwell disabled (immediate highlight), so the existing aperture/tint
/// contracts are about the pickbox alone. The dwell has its own tests below.
fn hover_core() -> HostCore {
    let core = core();
    {
        let mut controller = core.controller.borrow_mut();
        controller.session.set_hover_preview(true);
        controller.session.set_hover_dwell_ms(0);
    }
    core
}

/// Install a deterministic instant source for the hover dwell and return the
/// millisecond counter the test advances. The seam returns
/// `base_instant + counter`; production uses `Instant::now`.
fn install_hover_clock(core: &mut HostCore) -> Rc<Cell<u64>> {
    let now_ms = Rc::new(Cell::new(0_u64));
    let clock = now_ms.clone();
    let base = std::time::Instant::now();
    core.set_hover_clock_for_test(Rc::new(move || {
        base + std::time::Duration::from_millis(clock.get())
    }));
    now_ms
}

fn core_with_config(config_store: Rc<dyn ConfigStore>) -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store,
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap()
}

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn select_entity(core: &HostCore, entity: cad_domain::EntityId) {
    let document = core.controller.borrow().document_id;
    core.controller
        .borrow_mut()
        .session
        .selection
        .replace([cad_app::selection::model_ref(document, entity)]);
}

fn line_endpoints(
    core: &HostCore,
    entity: cad_domain::EntityId,
) -> (cad_domain::Point3, cad_domain::Point3) {
    let drawing = core.controller.borrow().drawing().unwrap();
    match &drawing.entity(entity).unwrap().geometry {
        cad_domain::SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected LINE geometry, got {other:?}"),
    }
}

fn add_two_lines(core: &HostCore) -> (cad_domain::EntityId, cad_domain::EntityId) {
    let mut controller = core.controller.borrow_mut();
    let first = controller.create_line(p(0.0, 0.0), p(1.0, 0.0)).unwrap();
    let second = controller.create_line(p(0.0, 10.0), p(1.0, 10.0)).unwrap();
    (
        cad_domain::EntityId(first.objects[0].0),
        cad_domain::EntityId(second.objects[0].0),
    )
}

fn run_controller_command(core: &HostCore, id: CommandId, payload: CommandPayload) {
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    controller
        .execute(Command {
            schema_version: 1,
            id,
            document,
            viewport,
            payload,
        })
        .unwrap();
}

fn catalog(status: &StatusMessage) -> (&str, BTreeMap<String, String>) {
    match status {
        StatusMessage::Catalog { key, args } => (
            key.as_str(),
            args.iter()
                .map(|arg| (arg.key.clone(), arg.value.clone()))
                .collect(),
        ),
        other => panic!("expected a catalog status, got {other:?}"),
    }
}

fn touch(
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
    metrics_revision: u64,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Touch,
        phase,
        button: None,
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn mouse(
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
    metrics_revision: u64,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn dispatch_pointer(core: &HostCore, request: InteractionRequest) {
    EngineClient::dispatch(core, EngineRequest::Interaction(request)).unwrap();
}

fn command(core: &HostCore, id: CommandId, payload: CommandPayload) -> Command {
    let c = core.controller.borrow();
    Command {
        schema_version: 1,
        id,
        document: c.document_id,
        viewport: c.viewport_id,
        payload,
    }
}

fn move_cursor_world(x: f64, y: f64) -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::World(p(x, y))))
}

fn install_fake_clock(core: &HostCore, value: Rc<Cell<f64>>) {
    let clock: Rc<dyn Fn() -> f64> = Rc::new(move || value.get());
    core.set_request_clock(clock).unwrap();
}

/// A platform whose `begin_open` re-enters the engine, to prove a service
/// callback cannot jump the synchronous write guard.
#[derive(Default)]
struct NestedProbeFiles {
    core: RefCell<Option<HostCore>>,
    reentries: Rc<Cell<usize>>,
}

impl PlatformFiles for NestedProbeFiles {
    fn begin_open(&self) -> CadResult<()> {
        if let Some(core) = self.core.borrow().as_ref() {
            assert!(matches!(
                EngineClient::dispatch(
                    core,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            assert!(matches!(
                EngineClient::submit(
                    core,
                    EngineClient::request_context(core),
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            assert!(matches!(EngineClient::pump(core), Err(RequestError::Busy)));
            self.reentries.set(self.reentries.get() + 1);
        }
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, _default_name: &str) -> CadResult<()> {
        Ok(())
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
    }
}

fn drive_until_idle(core: &HostCore) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while core.loading() {
        assert!(
            std::time::Instant::now() < deadline,
            "open did not reach a terminal"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Pan the active 2D viewport's camera by `dx` world units (what a drag produces).
fn pan_active_camera(core: &HostCore, dx: f64) {
    let mut c = core.controller.borrow_mut();
    let id = c.viewport_id;
    let viewport = c
        .application
        .workspace
        .viewports
        .get_mut(&id)
        .expect("the host always has an active viewport");
    viewport.camera.target.x += dx;
}

/// The canvas-local logical pixel a world point projects to in the active view.
fn screen_of(core: &HostCore, world: Point3) -> [f64; 2] {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .camera
        .world_to_screen(world, viewport.logical_size)
        .expect("the 2D plan view projects the world point")
}

/// A world point on the demo document's circle rim.
///
/// A circle is pickable on its circumference only (ADR-0012 §8), so the rim is
/// the only honest target for a click on it.
fn demo_circle_rim(core: &HostCore) -> Point3 {
    let drawing = core
        .controller
        .borrow()
        .drawing()
        .expect("the demo document");
    let (center, radius) = drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Circle { center, radius, .. } => Some((*center, *radius)),
            _ => None,
        })
        .expect("the demo document has a circle");
    Point3 {
        x: center.x + radius,
        y: center.y,
        z: center.z,
    }
}

/// The layer of the demo document's circle, for a temporary-hide test.
fn demo_circle_layer(core: &HostCore) -> cad_domain::LayerId {
    let drawing = core
        .controller
        .borrow()
        .drawing()
        .expect("the demo document");
    drawing
        .model_space()
        .iter()
        .find(|entity| {
            matches!(
                &entity.geometry,
                cad_domain::SemanticGeometry::Circle { .. }
            )
        })
        .map(|entity| entity.layer)
        .expect("the demo document has a circle")
}

/// The reference `pick_at_screen` reports at a pixel, computed exactly the way
/// `select_at_mode` computes it.
fn picked_ref_at(core: &HostCore, pixel: [f64; 2]) -> cad_domain::SelectionRef {
    let c = core.controller.borrow();
    let drawing = c.drawing().expect("the demo document");
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    cad_app::picking::pick_at_screen(
        &drawing,
        c.document_id,
        &viewport.camera,
        pixel,
        viewport.logical_size,
        &cad_domain::TolerancePolicy::default(),
        cad_app::BackFacePolicy::Cull,
    )
    .expect("the pixel produces a pick")
    .hit
    .expect("the demo circle rim is pickable")
    .source
}

/// The active viewport's camera, for "a click must not navigate" assertions.
fn active_camera(core: &HostCore) -> cad_app::Camera {
    let c = core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id].camera
}

/// Whether `pick_at_screen` reports no hit at a pixel.
fn picks_nothing(core: &HostCore, pixel: [f64; 2]) -> bool {
    let c = core.controller.borrow();
    let drawing = c.drawing().expect("the demo document");
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    cad_app::picking::pick_at_screen(
        &drawing,
        c.document_id,
        &viewport.camera,
        pixel,
        viewport.logical_size,
        &cad_domain::TolerancePolicy::default(),
        cad_app::BackFacePolicy::Cull,
    )
    .expect("the pixel produces a pick")
    .hit
    .is_none()
}

/// A pointer request with an explicit button.
fn pointer_with_button(
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
    metrics_revision: u64,
    button: PointerButton,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(button),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

/// The click sequence the session emits: the press and release of a click, then
/// the typed `SelectAt` replace-selection for the release position.
fn click_select_at(core: &HostCore, pixel: [f64; 2]) -> cad_domain::SelectionRef {
    let revision = core.request_context().metrics_revision.unwrap();
    dispatch_pointer(
        core,
        mouse(1, cad_app::interaction::PointerPhase::Down, pixel, revision),
    );
    dispatch_pointer(
        core,
        mouse(1, cad_app::interaction::PointerPhase::Up, pixel, revision),
    );
    dispatch_pointer(
        core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: pixel,
                metrics_revision: revision,
            },
            mode: SelectionMode::Replace,
        },
    );
    let selected = core.controller.borrow().session.selection.refs().to_vec();
    assert_eq!(selected.len(), 1, "a click selects exactly one object");
    selected[0].clone()
}

/// The typed `MoveCursor` the session emits for an idle pointer move: the
/// canvas-local hover probe, tagged with the host's current metrics revision.
fn hover_cursor(core: &HostCore, pixel: [f64; 2]) -> InteractionRequest {
    InteractionRequest::MoveCursor(PointInput::CanvasLocal {
        logical_position: pixel,
        metrics_revision: core.request_context().metrics_revision.unwrap(),
    })
}

/// The hovered refs that reached the producer contract (`OverlayInputs`).
fn hovered_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.render.overlays().hovered.refs().to_vec()
}

/// The real database identity of the current document: pointer, revision and
/// entity count. Hover must leave it bit-for-bit untouched.
fn document_identity(core: &HostCore) -> (usize, cad_domain::Revision, usize) {
    let c = core.controller.borrow();
    let drawing = c.drawing().expect("the demo document");
    (
        Arc::as_ptr(&drawing) as usize,
        drawing.revision(),
        drawing.entity_count(),
    )
}
