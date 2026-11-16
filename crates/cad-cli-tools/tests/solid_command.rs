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

use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandPayload, ExtendedCommandId};
use cad_domain::{Body3d, Completeness, DocumentId, Point3, Precision, SemanticGeometry};
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
