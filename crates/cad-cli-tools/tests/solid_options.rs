//! ADR-0036 C1 extension, real-kernel contract: the typed **option
//! constructions** (`Cube`, `Center`, `Diameter`, `2P`, `Top radius`) create the
//! bodies their `PrimitiveSpec` names through the **real** `solidcore` modeler.
//!
//! **Evidence boundary**: Linux x86-64 contract test. It links the pinned
//! `solidcore` revision through `cad-kernel-solidcore` and drives
//! `cad_app::Application` directly (no GPU, no window, no carrier). It is not a
//! product acceptance: Windows/macOS/Android/Web modeler wiring is **NOT
//! VERIFIED**, and nothing here says anything about rendering, pixels or
//! performance.
//!
//! Split out of `tests/solid_command.rs` because that file is at the 1000-line
//! code gate; the fixtures are the same (blank document, injected
//! `SolidCoreModeler` + `SolidCoreTessellator`).

use std::sync::Arc;

use cad_app::{Command, CommandId, CommandPayload, ExtendedCommandId};
use cad_domain::{Point3, SemanticGeometry};
use cad_kernel_adapter::SolidModeler;
use cad_kernel_solidcore::{SolidCoreModeler, SolidCoreTessellator};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn modeler_controller() -> cad_app::host::HostController {
    let mut controller =
        cad_app::host::HostController::with_blank_document([800.0, 600.0]).expect("blank document");
    let modeler: Arc<dyn SolidModeler> = Arc::new(SolidCoreModeler::new());
    controller.application.set_solid_modeler(Some(modeler));
    // A primitive body has no NURBS face, so the display mesher stays inert for
    // these contracts; it is injected exactly as `app-linux` composes the carrier.
    controller
        .application
        .set_solid_tessellator(Some(Arc::new(SolidCoreTessellator::new())));
    controller
}

/// The committed body's **display-mesh** bounds.
///
/// `brep_bounds` is a conservative size estimate for the tessellation budget (a
/// cylinder face is sampled as its whole circle's axis box), so it cannot answer
/// "where is this body". The facets are the derived world-space surface and are
/// the right oracle for a placement contract.
fn mesh_bounds(body: &cad_domain::Body3d) -> (Point3, Point3) {
    let mut vertices = body.display.mesh.vertices.iter();
    let first = *vertices.next().expect("a committed body has facets");
    let (mut min, mut max) = (first, first);
    for vertex in vertices {
        min.x = min.x.min(vertex.x);
        min.y = min.y.min(vertex.y);
        min.z = min.z.min(vertex.z);
        max.x = max.x.max(vertex.x);
        max.y = max.y.max(vertex.y);
        max.z = max.z.max(vertex.z);
    }
    (min, max)
}

/// Create one body and return its display-mesh bounds.
fn run_bounds(
    controller: &mut cad_app::host::HostController,
    ext: ExtendedCommandId,
    input: cad_app::SolidPrimitiveCommand,
) -> (Point3, Point3) {
    let document = controller.document_id;
    let viewport = controller.viewport_id;
    let command = Command {
        schema_version: 1,
        id: CommandId::Extended(ext),
        document,
        viewport,
        payload: CommandPayload::Solid(input),
    };
    let outcome = controller
        .application
        .execute(&mut controller.session, command)
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
    assert!(
        body.is_exact(),
        "{ext:?} must keep the modeler's exact B-rep"
    );
    mesh_bounds(body)
}

fn set_elev(controller: &mut cad_app::host::HostController, elevation: f64) {
    controller
        .session
        .drafting_extras
        .set_elev(elevation)
        .expect("a finite elevation is legal");
}

#[test]
fn box_cube_lands_on_the_session_elevation_with_one_edge() {
    let mut controller = modeler_controller();
    set_elev(&mut controller, 10.0);
    let (min, max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Box,
        cad_app::SolidPrimitiveCommand::Cube {
            corner: [2.0, 3.0],
            edge: 4.0,
        },
    );
    assert_eq!((min.x, min.y, min.z), (2.0, 3.0, 10.0));
    assert_eq!((max.x, max.y, max.z), (6.0, 7.0, 14.0));
}

#[test]
fn cylinder_diameter_and_two_point_bases_build_the_same_radius() {
    let mut controller = modeler_controller();
    set_elev(&mut controller, 2.0);
    let (diameter_min, diameter_max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Cylinder,
        cad_app::SolidPrimitiveCommand::CylinderPlan {
            base: cad_app::PlanCircle::Diameter {
                center: [0.0, 0.0],
                diameter: 4.0,
            },
            height: 5.0,
        },
    );
    // The base and top caps are planar, so their z is exact; the lateral
    // surface is faceted at a shape-relative tolerance, so the radial extreme is
    // an inscribed approximation just inside the true radius.
    assert!((diameter_min.x + 2.0).abs() < 0.02, "{diameter_min:?}");
    assert!((diameter_max.x - 2.0).abs() < 0.02, "{diameter_max:?}");
    assert!((diameter_min.z - 2.0).abs() < 1e-9, "{diameter_min:?}");
    assert!((diameter_max.z - 7.0).abs() < 1e-9, "{diameter_max:?}");

    let (two_point_min, two_point_max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Cylinder,
        cad_app::SolidPrimitiveCommand::CylinderPlan {
            base: cad_app::PlanCircle::TwoPoint {
                first: [0.0, 0.0],
                second: [0.0, 4.0],
            },
            height: 5.0,
        },
    );
    assert!((two_point_min.x + 2.0).abs() < 0.02, "{two_point_min:?}");
    assert!((two_point_max.x - 2.0).abs() < 0.02, "{two_point_max:?}");
    assert!(two_point_min.y.abs() < 0.02, "{two_point_min:?}");
    assert!((two_point_max.y - 4.0).abs() < 0.02, "{two_point_max:?}");
}

#[test]
fn cone_top_radius_builds_a_real_frustum() {
    let mut controller = modeler_controller();
    let (min, max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Cone,
        cad_app::SolidPrimitiveCommand::ConePlan {
            base: cad_app::PlanCircle::Radius {
                center: [0.0, 0.0],
                radius: 4.0,
            },
            top_radius: 1.0,
            height: 6.0,
        },
    );
    // The wide base is at z = 0 and the narrow top at z = 6.
    assert!((min.z).abs() < 1e-9, "{min:?}");
    assert!((max.z - 6.0).abs() < 1e-9, "{max:?}");
    // The top face's radius is 1, so the body's xy extent is set by the base.
    assert!((min.x + 4.0).abs() < 0.05, "{min:?}");
    assert!((max.x - 4.0).abs() < 0.05, "{max:?}");

    // AutoCAD's Top radius may also be *wider* than the base (the cone widens
    // upwards); the seam's `radius_top` carries it and the kernel builds it.
    let (wide_min, wide_max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Cone,
        cad_app::SolidPrimitiveCommand::ConePlan {
            base: cad_app::PlanCircle::Radius {
                center: [0.0, 0.0],
                radius: 4.0,
            },
            top_radius: 6.0,
            height: 6.0,
        },
    );
    assert!((wide_min.z).abs() < 1e-9, "{wide_min:?}");
    assert!((wide_max.z - 6.0).abs() < 1e-9, "{wide_max:?}");
    assert!((wide_min.x + 6.0).abs() < 0.05, "{wide_min:?}");
    assert!((wide_max.x - 6.0).abs() < 0.05, "{wide_max:?}");
}

#[test]
fn sphere_two_point_takes_the_3d_diameter_and_ignores_elev() {
    let mut controller = modeler_controller();
    set_elev(&mut controller, 42.0);
    let (min, max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Sphere,
        cad_app::SolidPrimitiveCommand::SphereTwoPoint {
            first: p(0.0, 0.0, 0.0),
            second: p(0.0, 0.0, 10.0),
        },
    );
    assert!((min.z).abs() < 1e-9, "{min:?}");
    assert!((max.z - 10.0).abs() < 1e-9, "{max:?}");
    assert!((min.x + 5.0).abs() < 0.05, "{min:?}");
}

#[test]
fn torus_ring_diameter_and_tube_radius_build_a_ring() {
    let mut controller = modeler_controller();
    set_elev(&mut controller, 1.0);
    let (min, max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Torus,
        cad_app::SolidPrimitiveCommand::TorusPlan {
            ring: cad_app::PlanCircle::Diameter {
                center: [0.0, 0.0],
                diameter: 20.0,
            },
            tube: cad_app::PlanLength::Radius(2.0),
        },
    );
    // Outer radius = major + minor = 10 + 2.
    assert!((min.x + 12.0).abs() < 0.05, "{min:?}");
    assert!((max.x - 12.0).abs() < 0.05, "{max:?}");
    assert!((min.z - (1.0 - 2.0)).abs() < 0.05, "{min:?}");
}

#[test]
fn pyramid_polygon_base_commits_the_ngon_body_through_the_real_modeler() {
    // A regular hexagon of vertex radius 2 at ELEV 1: its vertices sit at
    // `k·60°`, so the display bounds are exactly `x ± 2` and `y ± √3`, and the
    // apex is 3 units above the base plane. The real `solidcore` modeler must
    // lift and tessellate the authored planar B-rep, and the row must be the
    // same exact `AcDb3dSolid` the point forms commit.
    let mut controller = modeler_controller();
    set_elev(&mut controller, 1.0);
    let (min, max) = run_bounds(
        &mut controller,
        ExtendedCommandId::Pyramid,
        cad_app::SolidPrimitiveCommand::PyramidPolygon {
            base: [0.0, 0.0],
            radius: 2.0,
            sides: 6,
            circumscribed: false,
            height: 3.0,
        },
    );
    let root_three = 3.0_f64.sqrt();
    for (actual, expected, axis) in [
        (min.x, -2.0, "min x"),
        (min.y, -root_three, "min y"),
        (min.z, 1.0, "min z"),
        (max.x, 2.0, "max x"),
        (max.y, root_three, "max y"),
        (max.z, 4.0, "max z"),
    ] {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{axis}: {actual} vs {expected}"
        );
    }
}
