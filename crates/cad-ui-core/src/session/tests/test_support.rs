//! Shared fixtures and helpers for the session tests.
//!
//! Test-only; nothing here ships in a release build.
//! Items are `pub(super)` so the sibling test modules can glob-import them.

use super::super::*;

#[allow(unused_imports)]
pub(super) use std::cell::{Cell, RefCell};
#[allow(unused_imports)]
pub(super) use std::rc::Rc;
#[allow(unused_imports)]
pub(super) use std::sync::Arc;

#[allow(unused_imports)]
pub(super) use crate::protocol::{
    InputKind, ProbeInput, BUTTON_MIDDLE, BUTTON_PRIMARY, BUTTON_SECONDARY,
};
#[allow(unused_imports)]
pub(super) use cad_app::input::DRAG_THRESHOLD_LOGICAL_PX;
#[allow(unused_imports)]
pub(super) use cad_app::interaction::{
    InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, PointerKind, PointerPhase,
    SelectionMode,
};
#[allow(unused_imports)]
pub(super) use cad_app::{
    BackFacePolicy, Camera, CommandId, ImportProgressSnapshot, ImportTerminal,
};
#[allow(unused_imports)]
pub(super) use cad_domain::{
    CadError, CadResult, DocumentId, EntityId, InstancePath, Point3, SelectionRef,
    SemanticGeometry, TolerancePolicy,
};
#[allow(unused_imports)]
pub(super) use cad_engine::protocol::{
    EngineClient, EngineRequest, RawDiagnostic, RawDiagnosticParameter, StatusMessage,
};
#[allow(unused_imports)]
pub(super) use cad_engine::render::RenderSessionHandle;
#[allow(unused_imports)]
pub(super) use cad_host::{HostCore, HostOptions, HostPlatform, HostResources};
#[allow(unused_imports)]
pub(super) use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, HostCapabilities, PlatformFiles,
    SavePurpose, SurfaceMetrics, SurfaceSource,
};
pub(super) struct FakeFiles {
    pub(super) events: RefCell<Vec<FileEvent>>,
}

impl PlatformFiles for FakeFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, _name: &str) -> CadResult<()> {
        Err(CadError::Unsupported("save unsupported".into()))
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Err(CadError::Unsupported("write unsupported".into()))
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }
}

pub(super) struct FakeConfig;
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

pub(super) struct FakeSurface;
impl SurfaceSource for FakeSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        Some(SurfaceMetrics {
            logical: [800.0, 600.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        })
    }
}

/// A `SurfaceSource` whose metrics a test mutates, standing in for a live
/// host surface (each app host owns its own platform surface).
#[derive(Clone, Default)]
pub(super) struct MutableSurface {
    metrics: Rc<Cell<Option<SurfaceMetrics>>>,
}

impl MutableSurface {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn set_metrics(&self, metrics: SurfaceMetrics) {
        self.metrics.set(Some(metrics));
    }
}

impl SurfaceSource for MutableSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        self.metrics.get()
    }
}

pub(super) struct FakeResources;
impl cad_host::HostResources for FakeResources {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Ok(())
    }
}

/// A strategy that always refuses, so `CadSession::new` must propagate.
pub(super) struct FailingResources;
impl cad_host::HostResources for FailingResources {
    fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Err(CadError::Unsupported("no font resolver".into()))
    }
    fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
        Err(CadError::Unsupported("no image resolver".into()))
    }
}

pub(super) fn platform(files: Rc<FakeFiles>) -> HostPlatform {
    HostPlatform {
        files,
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(FakeResources),
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: false,
            plot: false,
            import_worker: false,
        },
    }
}

pub(super) fn options() -> HostOptions {
    HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: true,
        drawing: None,
        size: [800.0, 600.0],
    }
}

pub(super) fn session(files: Rc<FakeFiles>) -> CadSession {
    CadSession::new(options(), platform(files), RenderSessionHandle::default())
        .expect("session builds")
}

/// A session with the idle hover preview explicitly enabled through the public
/// command path (it is off by default) and the dwell disabled, so the existing
/// hover contracts observe the immediate highlight.
pub(super) fn hover_enabled_session(files: Rc<FakeFiles>) -> CadSession {
    let session = session(files);
    session
        .command("HOVERPREVIEW")
        .expect("HOVERPREVIEW is admitted");
    session
        .core
        .controller
        .borrow_mut()
        .session
        .set_hover_dwell_ms(0);
    assert!(
        session.core.controller.borrow().hover_preview_enabled(),
        "the preview command enabled the session flag"
    );
    session
}

/// A session whose surface source is mutable, so a test can change the
/// reported CAD rect exactly like a frontend resize handler.
pub(super) fn session_with_surface(files: Rc<FakeFiles>, surface: MutableSurface) -> CadSession {
    let mut host_platform = platform(files);
    host_platform.surface = Rc::new(surface);
    CadSession::new(options(), host_platform, RenderSessionHandle::default())
        .expect("session builds")
}

/// The active viewport's logical canvas size (what `Camera::zoom_at` anchors
/// against).
pub(super) fn canvas_logical_size(session: &CadSession) -> [f64; 2] {
    let c = session.core.controller.borrow();
    c.application
        .workspace
        .viewports
        .get(&c.viewport_id)
        .expect("an active viewport")
        .logical_size
}

pub(super) fn probe(kind: InputKind) -> ProbeInput {
    ProbeInput {
        sequence: 1,
        kind,
        x: None,
        y: None,
        dx: None,
        dy: None,
        width: None,
        height: None,
        dpr: None,
        button: None,
    }
}

pub(super) fn move_input(x: f32, y: f32) -> ProbeInput {
    let mut input = probe(InputKind::PointerMove);
    input.x = Some(x);
    input.y = Some(y);
    input
}

/// A pointer input of `kind` at a canvas-local logical position.
pub(super) fn pointer_at(
    kind: InputKind,
    sequence: u64,
    position: [f64; 2],
    button: Option<i32>,
) -> ProbeInput {
    let mut input = probe(kind);
    input.sequence = sequence;
    input.x = Some(position[0] as f32);
    input.y = Some(position[1] as f32);
    input.button = button;
    input
}

/// The press state of a fresh primary press at `down`.
pub(super) fn primary_press(down: [f64; 2]) -> PointerPress {
    PointerPress {
        button: PointerButton::Primary,
        down,
        dragged: false,
    }
}

/// The canvas-local logical pixel a world point projects to in the session's
/// active view (the same projection the host picks with).
pub(super) fn screen_of(session: &CadSession, world: cad_domain::Point3) -> [f64; 2] {
    let c = session.core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .camera
        .world_to_screen(world, viewport.logical_size)
        .expect("the 2D plan view projects the world point")
}

/// The session's active viewport camera.
pub(super) fn viewport_camera(session: &CadSession) -> cad_app::Camera {
    let c = session.core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id].camera
}

/// A world point on the demo document's circle rim (a circle is pickable on
/// its circumference only, per ADR-0012 §8).
pub(super) fn demo_circle_rim(session: &CadSession) -> cad_domain::Point3 {
    let c = session.core.controller.borrow();
    let drawing = c.drawing().expect("the demo document");
    let (center, radius) = drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Circle { center, radius, .. } => Some((*center, *radius)),
            _ => None,
        })
        .expect("the demo document has a circle");
    cad_domain::Point3 {
        x: center.x + radius,
        y: center.y,
        z: center.z,
    }
}

/// The ref the click-selection pick path reports at a pixel in this session.
pub(super) fn picked_ref_at(session: &CadSession, pixel: [f64; 2]) -> cad_domain::SelectionRef {
    let c = session.core.controller.borrow();
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
    .expect("the pixel is pickable")
    .source
}

/// Whether the pick path reports no hit at a pixel in this session.
pub(super) fn picks_nothing(session: &CadSession, pixel: [f64; 2]) -> bool {
    let c = session.core.controller.borrow();
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

/// The hovered refs that reached the producer contract (`OverlayInputs`).
pub(super) fn hovered_refs(session: &CadSession) -> Vec<cad_domain::SelectionRef> {
    session.core.render.overlays().hovered.refs().to_vec()
}
