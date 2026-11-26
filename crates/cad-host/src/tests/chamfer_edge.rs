//! ADR-0036 P2.0 host contracts for `CHAMFEREDGE`.
//!
//! `CHAMFEREDGE`'s one pick is a **pixel** pick: the captured seed is the
//! `PickHit.point` the ordinary pick funnel resolved, and the target body is the
//! current selection. These tests drive the real `HostCore` request path (no UI,
//! no GPU) and pin that shape against a perspective view, where the historical
//! `world_point` route (`Viewport::screen_to_work_plane`) would hard-stamp a
//! fabricated `z` — or error outright.
//!
//! Synthetic contract over `HostCore`; not visual/GPU evidence.

use super::*;
use cad_app::{AppMode, Camera, DrawToolKind, Projection};
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    Body3d, BodyDisplay, BodyExact, BodyKind, Completeness, DatabaseId, EntityId, InstancePath,
    LayerId, Mesh, ObjectId, Precision, Registration, Revision, SemanticGeometry, SpaceId,
    SubElementId,
};
use cad_kernel_adapter::{
    ModelerCapabilities, ModelerPrimitives, SolidModeler, SolidOutcome, SolidRequest, SolidResult,
    SolidUnsupportedReason,
};

const MESH_BODY: u128 = 96;

/// A 10×10×10 mesh row whose top faces carry a source key (P-MESH-2).
fn mesh_body(id: u128) -> DbEntity {
    let corners = [
        p3(0.0, 0.0, 0.0),
        p3(10.0, 0.0, 0.0),
        p3(10.0, 10.0, 0.0),
        p3(0.0, 10.0, 0.0),
        p3(0.0, 0.0, 10.0),
        p3(10.0, 0.0, 10.0),
        p3(10.0, 10.0, 10.0),
        p3(0.0, 10.0, 10.0),
    ];
    let indices: [[u32; 3]; 12] = [
        [0, 3, 2],
        [0, 2, 1],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let mut face_sources = vec![None; indices.len()];
    let top = SubElementId {
        source_key: "top".into(),
        topology_revision: Revision(0),
    };
    face_sources[2] = Some(top.clone());
    face_sources[3] = Some(top);
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbSubDMesh".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mesh(Mesh {
            vertices: corners.to_vec(),
            triangles: indices.to_vec(),
            normals: Vec::new(),
            face_sources,
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
        }),
        draw_order: 0,
    }
}

const BODY_A: u128 = 94;
const BODY_B: u128 = 95;

/// A 10×10×10 box display cache with outward-wound triangles and a face source
/// per triangle (a body click therefore resolves a real face sub-element).
fn box_body(id: u128, x0: f64) -> DbEntity {
    let corners = [
        p3(x0, 0.0, 0.0),
        p3(x0 + 10.0, 0.0, 0.0),
        p3(x0 + 10.0, 10.0, 0.0),
        p3(x0, 10.0, 0.0),
        p3(x0, 0.0, 10.0),
        p3(x0 + 10.0, 0.0, 10.0),
        p3(x0 + 10.0, 10.0, 10.0),
        p3(x0, 10.0, 10.0),
    ];
    let indices: [[u32; 3]; 12] = [
        [0, 3, 2],
        [0, 2, 1],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let face_sources: Vec<Option<SubElementId>> = (0..indices.len())
        .map(|triangle| {
            Some(SubElementId {
                source_key: format!("face-{}", triangle / 2),
                topology_revision: Revision(0),
            })
        })
        .collect();
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDb3dSolid".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Body3d(Box::new(Body3d {
            kind: BodyKind::Solid,
            // The capture path never reads the exact B-rep; the display cache is
            // what the pick ray tests. An empty exact B-rep is enough for the
            // target validation (no kernel call happens in these contracts).
            exact: BodyExact::Exact(Box::default()),
            display: BodyDisplay {
                mesh: Mesh {
                    vertices: corners.to_vec(),
                    triangles: indices.to_vec(),
                    normals: Vec::new(),
                    face_sources,
                    colors: Vec::new(),
                    uvs: Vec::new(),
                    texture: None,
                    pbr: None,
                },
                edges: Vec::new(),
                tolerance: Some(0.01),
                precision: Precision::Approximate {
                    error_bound: Some(0.01),
                },
                completeness: Completeness::Partial(vec!["host fixture mesh".into()]),
            },
        })),
        draw_order: id as i64,
    }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// Install a drawing with the given body rows and turn snapping off (these
/// contracts are about the pick, not about snaps).
fn install_bodies(core: &HostCore, bodies: Vec<DbEntity>) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for body in bodies {
        builder.insert_entity(body).unwrap();
    }
    let drawing = Arc::new(builder.finish().unwrap());
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    controller
        .application
        .workspace
        .documents
        .get_mut(&document)
        .expect("the host always has a document")
        .drawing = drawing;
    controller.set_snap_enabled(false);
}

/// Frame the `x ∈ [x0, x0 + 10]` box in a perspective view from the front-right
/// above, so its top-front edge (`y = 0`, `z = 10`) is a visible silhouette edge.
fn perspective_view(core: &HostCore) {
    let mut controller = core.controller.borrow_mut();
    let id = controller.viewport_id;
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&id)
        .expect("the host always has an active viewport");
    viewport.camera = Camera {
        eye: p3(16.0, -20.0, 18.0),
        target: p3(5.0, 5.0, 5.0),
        up: p3(0.0, 0.0, 1.0),
        projection: Projection::perspective(0.9).expect("a legal fov"),
    };
    viewport.camera.validate().expect("the perspective camera");
}

fn select_body(core: &HostCore, entity: u128, sub_element: Option<SubElementId>) {
    let document = core.controller.borrow().document_id;
    core.controller
        .borrow_mut()
        .session
        .selection
        .replace([SelectionRef {
            document,
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element,
        }]);
}

fn start_chamfer_edge(core: &HostCore) {
    // The host's `BeginDraw` validation needs an injected modeler (tested below);
    // these capture contracts start the session tool directly so they can pin
    // the pixel path itself.
    core.controller
        .borrow_mut()
        .session
        .begin_draw(DrawToolKind::ChamferEdge)
        .expect("the session starts the one-pick capture");
}

/// A modeler that exists only to satisfy the tool-start capability check. The
/// gate must never reach `execute`, so answering anything real here would be a
/// test bug.
struct FakeModeler;

impl SolidModeler for FakeModeler {
    fn registration(&self) -> Registration {
        Registration {
            type_key: "test.fake-solid".into(),
            version: 1,
            priority: 1,
            entity_types: vec!["3DSOLID".into()],
            capabilities: vec!["chamfer".into()],
        }
    }

    fn capabilities(&self) -> ModelerCapabilities {
        ModelerCapabilities {
            primitives: ModelerPrimitives::default(),
            booleans: false,
            mass_properties: false,
            chamfer: true,
            offset_face: false,
            extrude_profile: false,
            fillet: false,
            shell: false,
            nurbs: false,
        }
    }

    fn execute(
        &self,
        request: &SolidRequest<'_>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CadResult<SolidResult> {
        Ok(SolidResult {
            stamp: request.stamp.clone(),
            outcome: SolidOutcome::Unsupported {
                reason: SolidUnsupportedReason {
                    code: "test.modeler_not_called",
                    operation: request.operation.family(),
                    detail: "the tool-start gate must not reach the modeler".into(),
                },
            },
        })
    }
}

fn inject_fake_modeler(core: &HostCore) {
    core.controller
        .borrow_mut()
        .application
        .set_solid_modeler(Some(Arc::new(FakeModeler)));
}

fn metrics_revision(core: &HostCore) -> u64 {
    core.request_context()
        .metrics_revision
        .expect("the host publishes metrics")
}

fn click_logical(core: &HostCore, logical: [f64; 2]) -> CadResult<()> {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::CanvasLocal {
            logical_position: logical,
            metrics_revision: metrics_revision(core),
        })),
    )
    .map(|_| ())
    .map_err(|error| match error {
        RequestError::Execution(error) => error,
        other => panic!("unexpected request error: {other:?}"),
    })
}

fn screen_of(core: &HostCore, world: Point3) -> [f64; 2] {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .camera
        .world_to_screen(world, viewport.logical_size)
        .expect("the perspective view projects the point")
}

fn captured_points(core: &HostCore) -> Vec<Point3> {
    core.controller
        .borrow()
        .session
        .draw_preview()
        .expect("the capture is still active")
        .points
}

// ---- the crux -------------------------------------------------------------

#[test]
fn chamfer_edge_captures_the_pixel_hit_point_not_the_work_plane() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    perspective_view(&core);
    select_body(&core, BODY_A, None);
    start_chamfer_edge(&core);

    // The middle of the top-front edge (`y = 0`, `z = 10`).
    let edge = p3(5.0, 0.0, 10.0);
    let pixel = screen_of(&core, edge);
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        Some(DrawToolKind::ChamferEdge)
    );
    click_logical(&core, pixel).expect("the body edge is picked");

    let points = captured_points(&core);
    assert_eq!(points.len(), 1, "one step, one captured seed");
    let seed = points[0];
    let (work_plane_z, expected_z) = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        (viewport.work_plane.origin.z, edge.z)
    };
    assert!(
        (seed.z - expected_z).abs() < 1e-6,
        "the seed must be the real edge point (z = {expected_z}), got {seed:?}"
    );
    assert!(
        (seed.z - work_plane_z).abs() > 1.0,
        "the seed z must differ from the work plane's fabricated z by far more than any \
         tolerance (work plane z = {work_plane_z}, seed z = {})",
        seed.z
    );
    // The old route is what this replaces: in a perspective view the work-plane
    // mapping cannot even produce a point.
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    assert!(
        viewport
            .camera
            .screen_to_plan_world(pixel, viewport.logical_size)
            .is_none(),
        "a perspective view has no work-plane point, which is exactly why the old path fails"
    );
}

#[test]
fn hovering_in_a_perspective_view_does_not_error_or_feed_a_z_plane_point() {
    let core = hover_core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    perspective_view(&core);
    select_body(&core, BODY_A, None);
    start_chamfer_edge(&core);

    let pixel = screen_of(&core, p3(5.0, 0.0, 10.0));
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision: metrics_revision(&core),
        })),
    )
    .expect("a perspective cursor move is the hover probe, not a work-plane error");

    let preview = core
        .controller
        .borrow()
        .session
        .draw_preview()
        .expect("the capture is still active");
    assert!(
        preview.points.is_empty(),
        "hovering must not capture a point"
    );
    assert_eq!(
        preview.cursor, None,
        "no fabricated work-plane cursor is fed into the tool"
    );
}

// ---- pick matching --------------------------------------------------------

#[test]
fn a_click_on_another_body_is_refused_and_the_capture_stays_armed() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0), box_body(BODY_B, 30.0)]);
    // A perspective view of the first box; the second is far to the right, still
    // projected by the same camera.
    let mut controller = core.controller.borrow_mut();
    let id = controller.viewport_id;
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&id)
        .unwrap();
    viewport.camera = Camera::top_view_2d();
    viewport.camera.projection = Projection::Orthographic { scale: 1.0 };
    viewport.logical_size = [800.0, 600.0];
    drop(controller);
    // Select A **through a face ref**: the collapse rule must accept it.
    select_body(
        &core,
        BODY_A,
        Some(SubElementId {
            source_key: "face-0".into(),
            topology_revision: Revision(0),
        }),
    );
    start_chamfer_edge(&core);

    let other_pixel = screen_of(&core, p3(35.0, 5.0, 10.0));
    let error = click_logical(&core, other_pixel).expect_err("that is the other body");
    let CadError::InvalidInput(text) = error else {
        panic!("expected an InvalidInput refusal, got {error:?}");
    };
    assert!(
        text.starts_with("chamferedge.pick_not_selected_body:"),
        "{text}"
    );
    assert!(
        captured_points(&core).is_empty(),
        "a refused pick consumes no capture step"
    );

    let own_pixel = screen_of(&core, p3(5.0, 5.0, 10.0));
    click_logical(&core, own_pixel).expect("the selected body is picked");
    assert_eq!(captured_points(&core).len(), 1);
}

#[test]
fn a_click_on_empty_space_is_pick_missed_and_changes_nothing() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    perspective_view(&core);
    select_body(&core, BODY_A, None);
    start_chamfer_edge(&core);

    let error = click_logical(&core, [5.0, 5.0]).expect_err("the top-left corner is empty sky");
    let CadError::InvalidInput(text) = error else {
        panic!("expected an InvalidInput refusal, got {error:?}");
    };
    assert!(text.starts_with("chamferedge.pick_missed:"), "{text}");
    assert!(captured_points(&core).is_empty());
}

#[test]
fn a_second_click_names_the_ready_state_without_running_another_pick() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    perspective_view(&core);
    select_body(&core, BODY_A, None);
    start_chamfer_edge(&core);

    let pixel = screen_of(&core, p3(5.0, 0.0, 10.0));
    click_logical(&core, pixel).expect("the first pick captures the seed");
    let error = click_logical(&core, [5.0, 5.0]).expect_err("the one step is complete");
    let CadError::InvalidInput(text) = error else {
        panic!("expected the ready refusal, got {error:?}");
    };
    assert!(
        text.contains("is ready; press Enter to confirm"),
        "the completed capture names the real next action: {text}"
    );
    assert!(
        !text.contains("does not take an entity pick"),
        "a second click must not be refused as if the step still wanted a pick: {text}"
    );
    assert_eq!(
        captured_points(&core).len(),
        1,
        "the refused second click changes no capture state"
    );
}

// ---- fail-fast at tool start ---------------------------------------------

#[test]
fn a_carrier_without_a_modeler_refuses_at_tool_start_without_a_click() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    select_body(&core, BODY_A, None);

    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::ChamferEdge)),
    )
    .expect_err("no modeler on this carrier");
    match error {
        RequestError::Execution(CadError::Unsupported(reason)) => {
            assert_eq!(reason, "unsupported-runtime: solid-modeler");
        }
        other => panic!("expected the named modeler refusal, got {other:?}"),
    }
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        None,
        "a refused start arms no capture"
    );
}

#[test]
fn an_empty_selection_refuses_at_tool_start_even_with_a_modeler_slot_checked_first() {
    // The selection half of the tool-start gate is shared with the command
    // (`cad_app::chamfer_edge_target`); here the no-modeler gate fires first on
    // this synthetic carrier, so the selection rule itself is pinned in
    // `cad-app`'s contracts. This test only proves the gate runs before the
    // session owns the pointer.
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    let before = core.controller.borrow().session.selection.refs().to_vec();
    assert!(before.is_empty());
    let _ = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::ChamferEdge)),
    );
    assert_eq!(core.controller.borrow().session.draw_kind(), None);
    assert!(matches!(
        core.controller.borrow().session.tool,
        cad_app::ToolState::Idle
    ));
}

#[test]
fn the_chamfer_edge_kind_is_work_only_so_a_viewer_tool_start_is_refused() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    // A modeler exists and the selection **is** a valid exact solid, so the only
    // possible refusal is the Work-only authorization — the same
    // `PermissionDenied` a typed `CHAMFEREDGE` gets from the command path.
    inject_fake_modeler(&core);
    select_body(&core, BODY_A, None);
    core.controller
        .borrow_mut()
        .session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::ChamferEdge)),
    )
    .expect_err("a Viewer cannot chamfer");
    match error {
        RequestError::Execution(CadError::PermissionDenied) => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
    assert_eq!(core.controller.borrow().session.draw_kind(), None);

    // The authorization comes **first**, so an invalid selection cannot change
    // the refusal code: the tool start and the command path agree on
    // `PermissionDenied` even when the selection would otherwise be
    // `chamferedge.selection_changed`.
    select_body(&core, BODY_B, None);
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::ChamferEdge)),
    )
    .expect_err("still permission, not the selection code");
    match error {
        RequestError::Execution(CadError::PermissionDenied) => {}
        other => panic!("expected PermissionDenied before the selection check, got {other:?}"),
    }
    assert_eq!(core.controller.borrow().session.draw_kind(), None);
}

// ---- PRESSPULL: the same one-pick shape -----------------------------------
//
// PRESSPULL reuses the CHAMFEREDGE capture shape (a pixel pick as the kernel's
// face seed over the selected exact body), so it is pinned here against the same
// perspective fixture: the route must not fall back to the work-plane projection,
// and a pick on another body must be refused without consuming the capture.

fn start_press_pull(core: &HostCore) {
    core.controller
        .borrow_mut()
        .session
        .begin_draw(DrawToolKind::PressPull)
        .expect("the session starts the one-pick capture");
}

#[test]
fn press_pull_captures_the_pixel_hit_point_and_refuses_another_body() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0), box_body(BODY_B, 20.0)]);
    perspective_view(&core);
    select_body(&core, BODY_A, None);
    start_press_pull(&core);

    // A click on the other body is no pick for this capture: named refusal, and
    // the capture stays armed with zero points.
    let other = screen_of(&core, p3(25.0, 5.0, 10.0));
    let error = click_logical(&core, other).expect_err("another body is not the target");
    let text = match error {
        CadError::InvalidInput(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    };
    assert!(
        text.starts_with("presspull.pick_not_selected_body:"),
        "{text}"
    );
    assert!(captured_points(&core).is_empty());

    // The selected body's top face centre is captured as the hit point itself.
    let top = p3(5.0, 5.0, 10.0);
    click_logical(&core, screen_of(&core, top)).expect("the selected body's face is picked");
    let points = captured_points(&core);
    assert_eq!(points.len(), 1);
    assert!(
        (points[0].x - top.x).abs() < 1e-6
            && (points[0].y - top.y).abs() < 1e-6
            && (points[0].z - top.z).abs() < 1e-6,
        "the seed is the pixel hit point, got {:?}",
        points[0]
    );
}

#[test]
fn press_pull_starts_on_a_mesh_face_without_a_solid_modeler() {
    let core = core();
    install_bodies(&core, vec![mesh_body(MESH_BODY)]);
    perspective_view(&core);
    select_body(
        &core,
        MESH_BODY,
        Some(SubElementId {
            source_key: "top".into(),
            topology_revision: Revision(0),
        }),
    );
    // No exact-solid modeler is installed: a mesh face edit is kernel-free.
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::PressPull)),
    )
    .expect("a mesh face starts the kernel-free capture");
    assert_eq!(
        core.controller.borrow().session.draw_kind(),
        Some(DrawToolKind::PressPull)
    );
    let top = p3(5.0, 5.0, 10.0);
    click_logical(&core, screen_of(&core, top)).expect("the mesh face is picked");
    let points = captured_points(&core);
    assert_eq!(points.len(), 1);
    assert!((points[0].z - top.z).abs() < 1e-6);
}

#[test]
fn press_pull_start_refuses_a_carrier_without_a_modeler_by_name() {
    let core = core();
    install_bodies(&core, vec![box_body(BODY_A, 0.0)]);
    select_body(&core, BODY_A, None);
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::PressPull)),
    )
    .expect_err("no modeler means no face move");
    match error {
        RequestError::Execution(CadError::Unsupported(text)) => {
            assert!(text.contains("solid-modeler"), "{text}")
        }
        other => panic!("expected the named capability refusal, got {other:?}"),
    }
    assert_eq!(core.controller.borrow().session.draw_kind(), None);
}
