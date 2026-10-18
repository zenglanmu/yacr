//! Contract tests for the render session facade, split into cohesive
//! submodules. Shared fixtures live here; each submodule pulls them in
//! with `use super::*;`.

use super::*;
use cad_app::render_scene as controller;
use cad_app::render_scene::{
    build_scene, build_scene_with_overrides, build_scene_with_space, layout_descriptors,
};
use cad_app::BackendChoice;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, PaperViewport};
use cad_domain::TaskStamp;
use cad_domain::{
    Completeness, DatabaseId, EntityId, InstancePath, LayerId, LayoutId, Mesh, ObjectId, Point3,
    Revision,
};
use cad_domain::{SelectionRef, SemanticGeometry, SpaceId};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line_entity(id: u128, space: SpaceId, a: Point3, b: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line { start: a, end: b },
        draw_order: id as i64,
    }
}

fn db_with_layout() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Model line through the viewport anchor.
    b.insert_entity(line_entity(1, SpaceId::Model, p(9.0, 20.0), p(11.0, 20.0)))
        .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Layout1".into(),
        viewports: vec![PaperViewport {
            clip: vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)],
            model_to_paper: cad_domain::Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    b.finish().unwrap()
}

fn session_with_published_base() -> RenderSessionHandle {
    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing));
    session.prepare().unwrap();
    session
}

#[cfg(not(target_arch = "wasm32"))]
fn headless_gpu() -> cad_render_wgpu::headless::HeadlessGpu {
    let gpu = cad_render_wgpu::headless::create_headless_gpu(BackendPreference::Auto)
        .expect("hardware test requires a real wgpu adapter (P620/Vulkan)");
    eprintln!(
        "cad-engine hardware test adapter: backend={} name={} device_type={} driver={}",
        gpu.adapter.backend, gpu.adapter.name, gpu.adapter.device_type, gpu.adapter.driver
    );
    gpu
}

#[cfg(not(target_arch = "wasm32"))]
fn renderer_batch_count(session: &RenderSessionHandle) -> usize {
    session
        .0
        .borrow()
        .runtime
        .renderer
        .as_ref()
        .expect("attached renderer")
        .batch_count()
}

#[cfg(not(target_arch = "wasm32"))]
fn runtime_applied_base(session: &RenderSessionHandle) -> Option<u64> {
    session.0.borrow().runtime.applied_base_for_test()
}

/// Cumulative GPU bytes uploaded by the renderer. Read through a throwaway
/// render so the engine's own public surface is unchanged; the render itself
/// uploads nothing.
#[cfg(not(target_arch = "wasm32"))]
fn cumulative_upload_bytes(session: &RenderSessionHandle) -> u64 {
    let mut state = session.0.borrow_mut();
    let renderer = state.runtime.renderer.as_mut().expect("attached renderer");
    let target = cad_render_wgpu::RenderTarget::new(32, 32);
    let camera = camera2d_from_params(cad_app::Camera2dParams {
        center: p(0.0, 0.0),
        world_per_px: 1.0,
    });
    renderer
        .render(camera, &target)
        .expect("measurement render")
        .uploaded_bytes
}

#[cfg(not(target_arch = "wasm32"))]
fn db_with_lines(count: usize) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for i in 0..count {
        let id = i as u128 + 1;
        b.insert_entity(line_entity(
            id,
            SpaceId::Model,
            p(i as f64, 0.0),
            p(i as f64 + 1.0, 0.0),
        ))
        .unwrap();
    }
    b.finish().unwrap()
}

#[cfg(not(target_arch = "wasm32"))]
fn uploads_committed(session: &RenderSessionHandle) -> bool {
    let state = session.0.borrow();
    match state.controller.ready.as_ref() {
        Some(ready) => state
            .runtime
            .scene_committed(ready.revision, ready.base_revision),
        None => false,
    }
}

/// Drive the chunked `apply_ready_updates` until the current ready scene is
/// fully GPU-committed. Uses the real chunk limits (no budget weakening).
#[cfg(not(target_arch = "wasm32"))]
fn drive_uploads(session: &RenderSessionHandle) {
    for _ in 0..100_000 {
        session.apply_ready_updates().unwrap();
        if uploads_committed(session) {
            return;
        }
    }
    panic!("chunked GPU uploads did not commit within the bound");
}

#[cfg(not(target_arch = "wasm32"))]
fn model_entity(id: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(0),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// A filled axis-aligned quad (two triangles, CCW in a Y-up world — the winding
/// the renderer's quad tests use) as an `AcDbMesh` entity.
#[cfg(not(target_arch = "wasm32"))]
fn quad_entity(id: u128, cx: f64, cy: f64, half: f64) -> DbEntity {
    let vertices = vec![
        Point3 {
            x: cx - half,
            y: cy - half,
            z: 0.0,
        },
        Point3 {
            x: cx - half,
            y: cy + half,
            z: 0.0,
        },
        Point3 {
            x: cx + half,
            y: cy + half,
            z: 0.0,
        },
        Point3 {
            x: cx + half,
            y: cy - half,
            z: 0.0,
        },
    ];
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(Mesh {
            vertices,
            triangles: vec![[0, 2, 1], [0, 3, 2]],
            normals: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                };
                4
            ],
            face_sources: vec![None; 2],
            colors: Vec::new(),
        }),
        draw_order: id as i64,
    }
}

/// Two well-separated filled quads (entities 1 and 2) so a selected tint can be
/// sampled at a known pixel per entity.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_two_quads() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(quad_entity(1, -40.0, 0.0, 15.0)).unwrap();
    b.insert_entity(quad_entity(2, 40.0, 0.0, 15.0)).unwrap();
    b.finish().unwrap()
}

/// The pixel containing world point `(wx, wy)` under the default session camera
/// (centre `(0, 0)`, orthographic scale `1.0`).
#[cfg(not(target_arch = "wasm32"))]
fn world_to_pixel_2d(width: u32, height: u32, world: [f64; 2]) -> (u32, u32) {
    let sx = 2.0 / f64::from(width);
    let sy = 2.0 / f64::from(height);
    let px = ((sx * world[0] + 1.0) * 0.5 * f64::from(width)).floor() as i64;
    let py = ((1.0 - sy * world[1]) * 0.5 * f64::from(height)).floor() as i64;
    (
        px.clamp(0, i64::from(width) - 1) as u32,
        py.clamp(0, i64::from(height) - 1) as u32,
    )
}

/// Render the current scene through the session and read the frame back.
#[cfg(not(target_arch = "wasm32"))]
fn session_frame(session: &RenderSessionHandle, size: [f64; 2]) -> Vec<u8> {
    session.render(size, 1.0).unwrap().expect("frame");
    session
        .0
        .borrow_mut()
        .runtime
        .renderer
        .as_mut()
        .expect("attached renderer")
        .read_target_rgba()
        .expect("read back frame")
        .pixels
}

/// `(flags_upload_count, instance_stream_uploads, mesh_chunk_uploads)`.
#[cfg(not(target_arch = "wasm32"))]
fn renderer_counters(session: &RenderSessionHandle) -> (u64, u64, u64) {
    let state = session.0.borrow();
    let renderer = state.runtime.renderer.as_ref().expect("attached renderer");
    (
        renderer.flags_upload_count(),
        renderer.instance_stream_uploads(),
        renderer.mesh_chunk_uploads(),
    )
}

/// A settle timeout long enough that no assertion below can trip it.
const NO_SETTLE: Duration = Duration::from_secs(3600);

/// One camera-only pan step that moves the viewport world bounds.
fn pan_step(session: &RenderSessionHandle, dx: f64) {
    let mut view = session.view_snapshot();
    view.camera.target.x += dx;
    assert!(
        session.apply_view_snapshot(view).unwrap(),
        "pan step applies"
    );
}

/// One camera-only zoom step (2D orthographic `scale`), which also moves the
/// viewport world bounds and is therefore covered by the same deferral.
fn zoom_step(session: &RenderSessionHandle, factor: f64) {
    let mut view = session.view_snapshot();
    let scale = match view.camera.projection {
        cad_app::Projection::Orthographic { scale } => scale,
        _ => panic!("the 2D fixture must use an orthographic camera"),
    };
    view.camera.projection = cad_app::Projection::Orthographic {
        scale: scale * factor,
    };
    assert!(
        session.apply_view_snapshot(view).unwrap(),
        "zoom step applies"
    );
}

/// Settle deterministically: the zero timeout is the kill-switch that flushes a
/// pending refresh on the next pump.
fn settle_camera_refresh(session: &RenderSessionHandle) {
    session.set_grid_settle_timeout(Duration::ZERO);
    session.poll_preparation().expect("poll");
    assert!(
        !session.grid_refresh_pending(),
        "the settle commit must clear the deferred refresh"
    );
}

/// A single filled quad centred on the origin, so its centre pixel stays the
/// target centre under any 2D orthographic zoom about the origin.
#[cfg(not(target_arch = "wasm32"))]
fn db_with_one_quad() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(quad_entity(1, 0.0, 0.0, 15.0)).unwrap();
    b.finish().unwrap()
}

/// The renderer's resolved entity-flag keys (the retained request's live tint).
#[cfg(not(target_arch = "wasm32"))]
fn entity_flag_keys(session: &RenderSessionHandle) -> Vec<u32> {
    session
        .0
        .borrow()
        .runtime
        .renderer
        .as_ref()
        .expect("attached renderer")
        .entity_flag_keys()
}

mod gpu_base;
mod gpu_flags;
mod gpu_selection;
mod navigation;
mod scene_preparation;
mod settle;
mod spaces_camera;
