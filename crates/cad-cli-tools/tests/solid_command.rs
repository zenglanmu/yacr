//! ADR-0036 phase C1 integration contract: the seven primitive commands create
//! real exact bodies through the **real** `solidcore` modeler.
//!
//! **Evidence boundary**: this is a Linux x86-64 contract test. It links the
//! pinned `solidcore` revision through `cad-kernel-solidcore` and drives
//! `cad_app::Application` directly (no GPU, no window, no carrier). It is not a
//! product acceptance: Windows/macOS/Android/Web modeler wiring is
//! **NOT VERIFIED**, and nothing here says anything about rendering, pixels or
//! performance.
//!
//! The kernel is only reachable from this test target: `cad-cli-tools` itself
//! links no kernel (the seam is trait-only), and the modeler is injected exactly
//! the way `app-linux` injects it.

use std::sync::Arc;

use cad_app::host::{HostController, SolidBooleanPoll};
use cad_app::{
    Command, CommandId, CommandPayload, ExtendedCommandId, SelectionSet, ToolParameter, ToolValue,
};
use cad_domain::{
    Body3d, Completeness, DocumentId, EntityId, InstancePath, Point3, Precision, SelectionRef,
    SemanticGeometry,
};
use cad_kernel_adapter::SolidModeler;
use cad_kernel_solidcore::SolidCoreModeler;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn command(
    document: DocumentId,
    viewport: cad_domain::ViewportId,
    id: CommandId,
    payload: CommandPayload,
) -> Command {
    Command {
        schema_version: 1,
        id,
        document,
        viewport,
        payload,
    }
}

/// How many stored rows are `AcDb3dSolid` bodies.
fn solid_count(controller: &HostController) -> usize {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .filter(|entity| entity.object.type_key == "AcDb3dSolid")
        .count()
}

fn modeler_controller() -> HostController {
    let mut controller =
        HostController::with_blank_document([800.0, 600.0]).expect("blank document");
    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    controller.application.set_solid_modeler(Some(modeler));
    controller
}

/// Run one command and return the single committed body.
fn run_body(
    controller: &mut HostController,
    ext: ExtendedCommandId,
    points: Vec<Point3>,
) -> Body3d {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ext),
                CommandPayload::Points(points),
            ),
        )
        .unwrap_or_else(|error| panic!("{ext:?} creates a body: {error:?}"));
    assert_eq!(outcome.objects.len(), 1, "{ext:?}");
    let entity = controller.application.workspace.documents[&document]
        .drawing
        .entities()
        .find(|entity| entity.object.id == outcome.objects[0])
        .expect("the committed row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid", "{ext:?}");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("{ext:?} must store a Body3d");
    };
    body.as_ref().clone()
}

/// The point payload of each of the seven primitives (well-formed).
fn primitive_points(ext: ExtendedCommandId) -> Vec<Point3> {
    match ext {
        ExtendedCommandId::Box => vec![p(0.0, 0.0, 0.0), p(2.0, 3.0, 4.0)],
        ExtendedCommandId::Wedge => vec![p(0.0, 0.0, 0.0), p(4.0, 2.0, 0.0), p(0.0, 0.0, 3.0)],
        ExtendedCommandId::Pyramid => vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0), p(1.0, 1.0, 3.0)],
        ExtendedCommandId::Cylinder => vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(0.0, 0.0, 5.0)],
        ExtendedCommandId::Sphere => vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
        ExtendedCommandId::Cone => vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(0.0, 0.0, 5.0)],
        ExtendedCommandId::Torus => vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0), p(5.0, 0.0, 1.0)],
        other => panic!("{other:?} is not one of the seven primitives"),
    }
}

#[test]
fn box_then_cylinder_create_two_bodies_and_one_undo_removes_the_last() {
    let mut controller = modeler_controller();
    let document = controller.document_id;
    let viewport = controller.viewport_id;

    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ExtendedCommandId::Box),
                CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(2.0, 3.0, 4.0)]),
            ),
        )
        .expect("BOX creates a body");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(solid_count(&controller), 1);
    let box_entity = controller.application.workspace.documents[&document]
        .drawing
        .entities()
        .find(|entity| entity.object.id == outcome.objects[0])
        .expect("the box row exists");
    assert_eq!(box_entity.object.type_key, "AcDb3dSolid");
    let SemanticGeometry::Body3d(body) = &box_entity.geometry else {
        panic!("BOX must store a Body3d");
    };
    assert_eq!(body.kind, cad_domain::BodyKind::Solid);
    assert!(
        !body.display.mesh.triangles.is_empty(),
        "the kernel body is tessellated for display"
    );

    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ExtendedCommandId::Cylinder),
                CommandPayload::Points(vec![
                    p(10.0, 0.0, 0.0),
                    p(12.0, 0.0, 0.0),
                    p(10.0, 0.0, 5.0),
                ]),
            ),
        )
        .expect("CYLINDER creates a body");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(solid_count(&controller), 2, "two independent bodies");
    let cylinder_id = outcome.objects[0];

    controller
        .application
        .execute(
            &mut controller.session,
            command(document, viewport, CommandId::Undo, CommandPayload::None),
        )
        .expect("undo");
    assert_eq!(
        solid_count(&controller),
        1,
        "one undo removes the last body"
    );
    assert!(
        !controller.application.workspace.documents[&document]
            .drawing
            .entities()
            .any(|entity| entity.object.id == cylinder_id),
        "the cylinder row is gone"
    );
}

#[test]
fn a_carrier_without_an_injected_modeler_refuses_the_primitives_by_name() {
    let mut controller =
        HostController::with_blank_document([800.0, 600.0]).expect("blank document");
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let error = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ExtendedCommandId::Sphere),
                CommandPayload::Points(vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)]),
            ),
        )
        .expect_err("no modeler, no body");
    assert_eq!(
        error,
        cad_domain::CadError::Unsupported("unsupported-runtime: solid-modeler".into())
    );
    assert_eq!(solid_count(&controller), 0);
    assert!(!controller.application.can_undo(&controller.document_id));
}

#[test]
fn all_seven_primitives_create_a_displayable_body_with_the_expected_precision() {
    // Planar primitives are exact (Analytic + Complete); the four curved ones
    // are drawn as facets at the shape-relative tolerance and say so (Partial).
    let planar = [
        ExtendedCommandId::Box,
        ExtendedCommandId::Wedge,
        ExtendedCommandId::Pyramid,
    ];
    let curved = [
        ExtendedCommandId::Cylinder,
        ExtendedCommandId::Sphere,
        ExtendedCommandId::Cone,
        ExtendedCommandId::Torus,
    ];
    let mut controller = modeler_controller();
    for ext in planar.into_iter().chain(curved) {
        let body = run_body(&mut controller, ext, primitive_points(ext));
        assert!(
            !body.display.mesh.triangles.is_empty(),
            "{ext:?} has a display mesh"
        );
        // A loop-less analytic face (a full sphere / torus) has no boundary
        // curves, so its display is the mesh alone; every other primitive keeps
        // its boundary polylines (`cad-db` accepts both shapes).
        if has_boundary_edges(ext) {
            assert!(!body.display.edges.is_empty(), "{ext:?} has display edges");
        }
        let planar = planarity(ext);
        if planar {
            assert_eq!(body.display.precision, Precision::Analytic, "{ext:?}");
            assert_eq!(body.display.completeness, Completeness::Complete, "{ext:?}");
        } else {
            assert!(
                matches!(body.display.precision, Precision::Approximate { .. }),
                "{ext:?} is a discrete mesh: {:?}",
                body.display.precision
            );
            let Completeness::Partial(reasons) = &body.display.completeness else {
                panic!("{ext:?} must not claim a complete curved display");
            };
            assert!(
                reasons
                    .iter()
                    .any(|reason| reason.contains("discrete mesh") && reason.contains("zoom")),
                "{ext:?}: {reasons:?}"
            );
        }
    }
    assert_eq!(solid_count(&controller), 7);
}

fn has_boundary_edges(ext: ExtendedCommandId) -> bool {
    !matches!(ext, ExtendedCommandId::Sphere | ExtendedCommandId::Torus)
}

fn planarity(ext: ExtendedCommandId) -> bool {
    matches!(
        ext,
        ExtendedCommandId::Box | ExtendedCommandId::Wedge | ExtendedCommandId::Pyramid
    )
}

#[test]
fn a_none_payload_is_refused_before_any_kernel_work() {
    let mut controller = modeler_controller();
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let error = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ExtendedCommandId::Box),
                CommandPayload::None,
            ),
        )
        .expect_err("BOX needs explicit points");
    assert!(
        matches!(&error, cad_domain::CadError::InvalidInput(text) if text.starts_with("box.requires_points:")),
        "{error:?}"
    );
    assert_eq!(solid_count(&controller), 0);
}

#[test]
fn a_wedge_is_the_same_body_whichever_base_corner_is_first() {
    // The contract is "tall face at the minimum-x corner": p0/p1 order must not
    // change the resulting exact B-rep.
    let mut controller = modeler_controller();
    let forward = run_body(
        &mut controller,
        ExtendedCommandId::Wedge,
        vec![p(0.0, 0.0, 0.0), p(4.0, 2.0, 0.0), p(0.0, 0.0, 3.0)],
    );
    let reversed = run_body(
        &mut controller,
        ExtendedCommandId::Wedge,
        vec![p(4.0, 2.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 0.0, 3.0)],
    );
    assert_eq!(forward.exact, reversed.exact);
}

#[test]
fn elev_does_not_move_an_explicitly_placed_solid() {
    let mut controller = modeler_controller();
    controller
        .session
        .drafting_extras
        .set_elev(10.0)
        .expect("a finite elevation is legal");
    let body = run_body(
        &mut controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(2.0, 3.0, 4.0)],
    );
    let (min, max) = cad_kernel_adapter::brep_bounds(body.exact_brep().expect("exact"))
        .expect("a created box has bounds");
    assert_eq!(min.z, 0.0, "ELEV must not lift the body");
    assert_eq!(max.z, 4.0);
}

#[test]
fn a_large_sphere_is_tessellated_at_a_shape_relative_tolerance() {
    // The absolute 0.01 default used to refuse r=500 on budget (and could
    // materialise a ~134M-triangle mesh for r=1e6 before checking it). The
    // shared rule keeps the facet count ~constant, so r=500 creates normally.
    let mut controller = modeler_controller();
    let body = run_body(
        &mut controller,
        ExtendedCommandId::Sphere,
        vec![p(0.0, 0.0, 0.0), p(500.0, 0.0, 0.0)],
    );
    assert!(!body.display.mesh.triangles.is_empty());
    assert!(body.display.mesh.triangles.len() < 200_000);
}

#[test]
fn an_absurdly_large_sphere_stays_bounded() {
    // A regression to an absolute deflection would blow this up (r=1e6 with a
    // 0.01 chord budget asks for ~134M triangles), so a plain `Ok` with a small
    // triangle count is the assertion — not "either succeeds or refuses".
    let mut controller = modeler_controller();
    let body = run_body(
        &mut controller,
        ExtendedCommandId::Sphere,
        vec![p(0.0, 0.0, 0.0), p(1.0e6, 0.0, 0.0)],
    );
    let triangles = body.display.mesh.triangles.len();
    assert!(
        triangles < 20_000,
        "a 1e6-radius sphere must stay shape-relative, got {triangles} triangles"
    );
}

// ---- ADR-0036 phase C2: UNION / SUBTRACT through the real kernel ----------
//
// The command starts a real worker (`tasks::solid`), the controller polls it and
// commits one body in one transaction. **Evidence boundary**: Linux x86-64
// contract only; no GPU, window or carrier, and no pixel/performance claim.

/// Create two overlapping `BOX` bodies (two undo steps) and return their ids.
fn overlapping_boxes(controller: &mut HostController) -> (EntityId, EntityId) {
    run_body(
        controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(2.0, 2.0, 2.0)],
    );
    run_body(
        controller,
        ExtendedCommandId::Box,
        vec![p(1.0, 1.0, 1.0), p(3.0, 3.0, 3.0)],
    );
    let ids: Vec<EntityId> = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .map(|entity| entity.id)
        .collect();
    assert_eq!(ids.len(), 2, "two boxes");
    (ids[0], ids[1])
}

fn select_bodies(controller: &mut HostController, ids: &[EntityId]) {
    let document = controller.document_id;
    controller.session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document,
        entity: *id,
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

/// Start one boolean and assert the command itself wrote nothing.
fn start_boolean(controller: &mut HostController, ext: ExtendedCommandId) {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let solids_before = solid_count(controller);
    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::Extended(ext),
                CommandPayload::None,
            ),
        )
        .unwrap_or_else(|error| panic!("{ext:?} starts: {error:?}"));
    assert!(outcome.objects.is_empty(), "{ext:?} commits only on poll");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "solid.boolean_started"),
        "{ext:?} reports the job start"
    );
    assert_eq!(
        solid_count(controller),
        solids_before,
        "{ext:?} wrote nothing"
    );
}

/// Poll the controller until the worker settles (the real kernel is slower than
/// the fake one, so this waits on the public poll contract, not a fixed sleep).
fn wait_boolean_done(controller: &mut HostController) -> cad_domain::ObjectId {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        match controller.poll_solid_boolean() {
            SolidBooleanPoll::Running => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the real boolean did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            SolidBooleanPoll::Done { object, .. } => return object,
            other => panic!("the real kernel should fold two boxes, got {other:?}"),
        }
    }
}

#[test]
fn union_commits_one_body_through_the_background_job() {
    let mut controller = modeler_controller();
    let (first, second) = overlapping_boxes(&mut controller);
    select_bodies(&mut controller, &[first, second]);
    start_boolean(&mut controller, ExtendedCommandId::Union);
    assert_eq!(solid_count(&controller), 2, "the command wrote nothing yet");

    let object = wait_boolean_done(&mut controller);
    assert_eq!(solid_count(&controller), 1, "one combined body");
    let document = controller.document_id;
    let entity = controller.application.workspace.documents[&document]
        .drawing
        .entities()
        .find(|entity| entity.object.id == object)
        .expect("the combined row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("UNION must commit a Body3d");
    };
    assert_eq!(body.kind, cad_domain::BodyKind::Solid);
    assert!(
        body.exact_brep().is_some(),
        "the result keeps exact geometry"
    );
    assert!(!body.display.mesh.triangles.is_empty());

    // One undo restores both operands.
    controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                controller.viewport_id,
                CommandId::Undo,
                CommandPayload::None,
            ),
        )
        .expect("undo");
    assert_eq!(solid_count(&controller), 2, "one undo restores the group");
    assert!(
        !controller.application.workspace.documents[&document]
            .drawing
            .entities()
            .any(|entity| entity.object.id == object),
        "the combined body is gone after undo"
    );
}

#[test]
fn subtract_commits_one_body_through_the_background_job() {
    let mut controller = modeler_controller();
    let (first, second) = overlapping_boxes(&mut controller);
    select_bodies(&mut controller, &[first, second]);
    start_boolean(&mut controller, ExtendedCommandId::Subtract);

    let object = wait_boolean_done(&mut controller);
    assert_eq!(solid_count(&controller), 1, "first minus second");
    let document = controller.document_id;
    let entity = controller.application.workspace.documents[&document]
        .drawing
        .entities()
        .find(|entity| entity.object.id == object)
        .expect("the difference row exists");
    assert_eq!(entity.object.type_key, "AcDb3dSolid");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("SUBTRACT must commit a Body3d");
    };
    assert_eq!(body.kind, cad_domain::BodyKind::Solid);
    assert!(body.exact_brep().is_some());
}

// ---- ADR-0036 phase C3a: MASSPROP through the real kernel -----------------
//
// MASSPROP is a read: the modeler runs on the command thread against the stored
// exact B-rep. **Evidence boundary**: Linux x86-64 contract only; no GPU,
// window, carrier, pixel or performance claim.

/// Every stored `AcDb3dSolid` row id, in insertion order.
fn solid_ids(controller: &HostController) -> Vec<EntityId> {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entities()
        .filter(|entity| entity.object.type_key == "AcDb3dSolid")
        .map(|entity| entity.id)
        .collect()
}

/// The first `massprop.object` row, or a panic naming what was reported.
fn massprop_object(controller: &mut HostController) -> cad_domain::Diagnostic {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let outcome = controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                viewport,
                CommandId::MassProp,
                CommandPayload::None,
            ),
        )
        .expect("MASSPROP measures an exact solid body");
    outcome
        .diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == "massprop.object")
        .unwrap_or_else(|| panic!("no massprop.object row"))
}

#[test]
fn massprop_reports_a_real_box_volume_area_and_requested_accuracy() {
    let mut controller = modeler_controller();
    run_body(
        &mut controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)],
    );
    let ids = solid_ids(&controller);
    assert_eq!(ids.len(), 1, "one unit box");
    select_bodies(&mut controller, &ids);

    let row = massprop_object(&mut controller);
    assert_eq!(row.object.map(|object| object.0), Some(ids[0].0));
    let text = row.message;
    assert!(text.contains("volume=1.000000"), "{text}");
    assert!(text.contains("surface_area=6.000000"), "{text}");
    assert!(
        text.contains("centroid=(0.500000, 0.500000, 0.500000)"),
        "{text}"
    );
    assert!(
        text.contains("estimated at requested relative accuracy 1e-6, not a proved bound"),
        "{text}"
    );
    assert!(!text.contains("analytic"), "{text}");
    assert_eq!(solid_count(&controller), 1, "a read writes nothing");
}

/// Inverted from `massprop_refuses_a_real_curved_cylinder_as_not_liftable`: a
/// curved body now lifts (P2.1b part 2), so `MASSPROP` measures it against the
/// closed form instead of refusing it.
///
/// The `massprop.body_not_liftable` path is still covered — by the crate that can
/// author a *curved-but-unrepresentable* body (`cad-kernel-solidcore`'s unit
/// tests: a curved face with no `(u, v)` domain, a boundary with no exact pcurve)
/// and by `cad-app`'s fake-modeler test. No CLI command can author such a face,
/// so inventing one here would be a fabricated operand.
#[test]
fn massprop_measures_a_real_curved_cylinder_against_its_closed_form() {
    use std::f64::consts::PI;
    let mut controller = modeler_controller();
    let document = controller.document_id;
    // Radius 2, height 5: V = πr²h = 20π, A = 2πr² + 2πrh = 8π + 20π = 28π.
    run_body(
        &mut controller,
        ExtendedCommandId::Cylinder,
        vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(0.0, 0.0, 5.0)],
    );
    let ids = solid_ids(&controller);
    select_bodies(&mut controller, &ids);
    let revision = controller.application.workspace.documents[&document]
        .drawing
        .revision();

    let row = massprop_object(&mut controller);
    assert_eq!(row.object.map(|object| object.0), Some(ids[0].0));
    let text = row.message;
    let volume = 20.0 * PI;
    let area = 28.0 * PI;
    assert!(
        text.contains(&format!("volume={volume:.6}")),
        "volume {volume:.6}: {text}"
    );
    assert!(
        text.contains(&format!("surface_area={area:.6}")),
        "area {area:.6}: {text}"
    );
    // The axis centroid prints with a signed zero, so only the height and the
    // zero plane are pinned.
    assert!(
        text.contains("0.000000, 0.000000, 2.500000)"),
        "centroid at mid-height: {text}"
    );
    assert!(
        text.contains("estimated at requested relative accuracy 1e-6, not a proved bound"),
        "{text}"
    );
    assert_eq!(
        controller.application.workspace.documents[&document]
            .drawing
            .revision(),
        revision,
        "a read writes nothing"
    );
    assert_eq!(solid_count(&controller), 1, "the cylinder stays");
}

// ---------------------------------------------------------------------------
// ADR-0036 P3a: a real modeler BOX survives a DXF save/open as an exact body.
// ---------------------------------------------------------------------------

/// Signed volume of a closed, outward-wound triangle mesh (divergence formula).
fn mesh_volume(mesh: &cad_domain::Mesh) -> f64 {
    let mut volume = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        volume += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    volume / 6.0
}

/// BOX → DXF bytes → re-import keeps a single `Body3d` whose exact B-rep is the
/// planar box, written as ACIS SAT text in a `3DSOLID`.
///
/// **Evidence boundary**: this is a Linux x86-64 contract test proving our own
/// writer/reader agreement. No third-party CAD application opens the file, and
/// nothing here is GPU, pixel or vendor-compatibility evidence.
#[test]
fn a_modeler_box_survives_a_dxf_round_trip_as_an_exact_solid3d() {
    use cad_import_acadrust::export::{
        export, ExportFormat, ExportLimits, ExportRequest, ExportSpace,
    };
    use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

    let mut controller = modeler_controller();
    let document = controller.document_id;
    let body = run_body(
        &mut controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(1.0, 1.0, 1.0)],
    );
    assert!(body.is_exact(), "the modeler BOX carries an exact B-rep");

    let database = controller.application.workspace.documents[&document]
        .drawing
        .clone();
    let (bytes, report) = export(&ExportRequest {
        database: &database,
        document_id: document,
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports as DXF text");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("3DSOLID"),
        "the exact body is written as a 3DSOLID record"
    );
    let messages: Vec<&str> = report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.converted.acis_solid")
        .map(|entry| entry.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("no third-party CAD reader was used to verify")),
        "the report states the evidence boundary: {messages:?}"
    );
    assert!(
        messages.iter().all(|message| !message.contains("AutoCAD")
            && !message.contains("verified by a CAD application")),
        "the report never claims third-party verification: {messages:?}"
    );

    let imported = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(2),
                database: cad_domain::DatabaseId(2),
                bytes: std::sync::Arc::from(bytes.into_boxed_slice()),
                limits: ImportLimits::default(),
                generation: 0,
                xref: None,
            },
            &|| false,
        )
        .expect("the exported DXF re-imports")
        .database;

    let bodies: Vec<&Body3d> = imported
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Body3d(body) => Some(body.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(bodies.len(), 1, "one body row, not a mesh copy");
    let imported_body = bodies[0];
    assert_eq!(
        imported_body.kind,
        cad_domain::BodyKind::Solid,
        "a 3DSOLID with an exact B-rep lifts back as a Solid"
    );
    assert!(
        imported_body.is_exact(),
        "the SAT payload is the exact planar B-rep, not a display cache"
    );
    let volume = mesh_volume(&imported_body.display.mesh);
    assert!(
        (volume - 1.0).abs() < 1e-3,
        "a 1×1×1 BOX keeps volume 1 through the round trip, got {volume}"
    );
}

// ---------------------------------------------------------------------------
// ADR-0036 P2.0: CHAMFEREDGE through the **real** solidcore modeler.
// ---------------------------------------------------------------------------

/// The stored exact B-rep of one `AcDb3dSolid` row.
fn stored_brep(controller: &HostController, id: EntityId) -> cad_domain::BrepData {
    let entity = controller.application.workspace.documents[&controller.document_id]
        .drawing
        .entity(id)
        .expect("the body row exists");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("entity {} is not a body", id.0);
    };
    body.exact_brep().expect("the body is exact").clone()
}

fn face_count(brep: &cad_domain::BrepData) -> usize {
    brep.shells.iter().map(|shell| shell.faces.len()).sum()
}

fn revision_of(controller: &HostController) -> cad_domain::Revision {
    controller.application.workspace.documents[&controller.document_id]
        .drawing
        .revision()
}

fn set_chamfer_distances(controller: &mut HostController, a: f64, b: f64) {
    controller
        .session
        .set_tool_parameter(ToolParameter::ChamferDistanceA, ToolValue::Number(a))
        .expect("A is a legal set-back");
    controller
        .session
        .set_tool_parameter(ToolParameter::ChamferDistanceB, ToolValue::Number(b))
        .expect("B is a legal set-back");
}

/// Run CHAMFEREDGE with the host-built payload (the real kernel seam; the pick
/// itself is a `cad-host`/`cad-app` contract, not a CLI one).
fn chamfer_edge(
    controller: &mut HostController,
    body: EntityId,
    seed: Point3,
) -> cad_domain::CadResult<cad_app::CommandOutcome> {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    controller.application.execute(
        &mut controller.session,
        command(
            document,
            viewport,
            CommandId::Extended(ExtendedCommandId::ChamferEdge),
            CommandPayload::ChamferSolidEdge {
                body: SelectionRef {
                    document,
                    entity: body,
                    instance: InstancePath::default(),
                    sub_element: None,
                },
                seed,
            },
        ),
    )
}

/// **Evidence boundary**: Linux x86-64 contract only (real `solidcore`, no GPU,
/// no window, no carrier); it is not visual/pixel/performance evidence.
#[test]
fn chamfer_edge_cuts_one_real_box_edge_in_place() {
    let mut controller = modeler_controller();
    let document = controller.document_id;
    run_body(
        &mut controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(2.0, 3.0, 4.0)],
    );
    let ids = solid_ids(&controller);
    assert_eq!(ids.len(), 1);
    let body = ids[0];
    select_bodies(&mut controller, &ids);
    set_chamfer_distances(&mut controller, 0.5, 0.5);

    let before_revision = revision_of(&controller);
    let before_brep = stored_brep(&controller, body);
    let before_faces = face_count(&before_brep);
    let before_undo = controller.application.history[&document].undo_depth();

    // The middle of the top-front edge (`y = 0`, `z = 4`).
    let seed = p(1.0, 0.0, 4.0);
    let outcome = chamfer_edge(&mut controller, body, seed)
        .expect("the real kernel chamfers a planar box edge");
    assert_eq!(outcome.objects, vec![cad_domain::ObjectId(body.0)]);
    assert_eq!(
        revision_of(&controller),
        cad_domain::Revision(before_revision.0 + 1),
        "exactly one revision bump"
    );
    assert_eq!(
        controller.application.history[&document].undo_depth(),
        before_undo + 1,
        "exactly one undo entry"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "chamferedge.applied"),
        "the success is reported"
    );

    let after_brep = stored_brep(&controller, body);
    assert_eq!(
        face_count(&after_brep),
        before_faces + 1,
        "one chamfer face replaces the edge"
    );
    assert_ne!(after_brep, before_brep);

    controller
        .application
        .execute(
            &mut controller.session,
            command(
                document,
                controller.viewport_id,
                CommandId::Undo,
                CommandPayload::None,
            ),
        )
        .expect("undo");
    assert_eq!(
        stored_brep(&controller, body),
        before_brep,
        "one undo restores the exact original B-rep"
    );
}

/// **Evidence boundary**: as above — real-kernel contract, not visual evidence.
#[test]
fn a_real_face_centre_seed_is_solid_edge_not_found_and_writes_nothing() {
    let mut controller = modeler_controller();
    run_body(
        &mut controller,
        ExtendedCommandId::Box,
        vec![p(0.0, 0.0, 0.0), p(2.0, 3.0, 4.0)],
    );
    let ids = solid_ids(&controller);
    select_bodies(&mut controller, &ids);
    set_chamfer_distances(&mut controller, 0.5, 0.5);
    let before_revision = revision_of(&controller);
    let before_undo = controller.application.history[&controller.document_id].undo_depth();

    // The centre of the top face: far from every edge.
    let error = chamfer_edge(&mut controller, ids[0], p(1.0, 1.5, 4.0))
        .expect_err("no edge is near the face centre");
    let cad_domain::CadError::Unsupported(text) = error else {
        panic!("a kernel edge refusal is Unsupported, got {error:?}");
    };
    assert!(text.starts_with("solid.edge_not_found:"), "{text}");
    assert_eq!(
        revision_of(&controller),
        before_revision,
        "a refusal writes nothing"
    );
    assert_eq!(
        controller.application.history[&controller.document_id].undo_depth(),
        before_undo,
        "a refusal records no undo"
    );
    assert_eq!(solid_count(&controller), 1, "the body is unchanged");
}

/// Inverted from `a_real_curved_cylinder_chamfer_is_solid_lift_requires_planar`.
///
/// A curved body now lifts, so the chamfer reaches the **pick** — and stops there:
/// the cylinder's only `Line` edge is the seam the writer reconstructs to close the
/// record, and a seam (both its uses on one face) is not an edge a user can select.
/// The refusal is therefore `solid.edge_not_found`, naming the seam it will not
/// hand over, and the body is unchanged. Cutting a cylinder's rim needs a
/// curve-edge selector, which is P2.2.
///
/// **Evidence boundary**: real-kernel contract on Linux x86-64. The refusal is
/// the modeler's own answer; no GPU/window/carrier is involved.
#[test]
fn a_real_curved_cylinder_chamfer_finds_no_pickable_edge() {
    let mut controller = modeler_controller();
    run_body(
        &mut controller,
        ExtendedCommandId::Cylinder,
        vec![p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(0.0, 0.0, 5.0)],
    );
    let ids = solid_ids(&controller);
    select_bodies(&mut controller, &ids);
    set_chamfer_distances(&mut controller, 0.5, 0.5);
    let before_revision = revision_of(&controller);
    let before_undo = controller.application.history[&controller.document_id].undo_depth();

    // A point on the cylinder's top rim: the edge a cylinder actually has.
    let error = chamfer_edge(&mut controller, ids[0], p(2.0, 0.0, 5.0))
        .expect_err("a cylinder's rim has no pickable straight edge");
    let cad_domain::CadError::Unsupported(text) = error else {
        panic!("a capability refusal is Unsupported, got {error:?}");
    };
    assert!(
        !text.starts_with("solid.lift_requires_planar:"),
        "a curved operand no longer refuses at the lift: {text}"
    );
    assert!(
        text.starts_with("solid.edge_not_found:"),
        "the only Line edge is a seam, which is not pickable: {text}"
    );
    assert!(
        text.contains("seam"),
        "the refusal says the seam is what it skipped: {text}"
    );
    assert_eq!(revision_of(&controller), before_revision);
    assert_eq!(
        controller.application.history[&controller.document_id].undo_depth(),
        before_undo
    );
    assert_eq!(solid_count(&controller), 1, "the cylinder is unchanged");
}
