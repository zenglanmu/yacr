//! Real-kernel contract tests for the `MeshModeler` seam (ADR-0061, P-MESH-0).
//!
//! These run the pinned `manifold-rust` kernel in-process: no mock, no golden
//! file, no GPU. They prove the seam's status mapping and the boolean/repair
//! arithmetic on synthetic closed cubes; they are not carrier, performance or
//! real-SketchUp evidence.

use cad_domain::{CadError, DocumentId, Revision, TaskStamp};
use cad_kernel_adapter::{
    codes, MeshBooleanOp, MeshData, MeshModeler, MeshOperation, MeshOperationFamily, MeshOutcome,
    MeshRepairMode, MeshRequest, MeshTopology, MeshWindingRule,
};
use manifold_rust::linalg::Vec3;
use manifold_rust::manifold::Manifold;

use crate::{mesh_from_kernel, volume_of, ManifoldMeshModeler};

fn stamp() -> TaskStamp {
    TaskStamp {
        document: DocumentId(1),
        generation: 1,
        object_revision: Revision(0),
        dependency_version: 0,
        configuration_version: 0,
    }
}

fn cube(size: f64, at: [f64; 3]) -> MeshData {
    let body =
        Manifold::cube(Vec3::new(size, size, size), true).translate(Vec3::new(at[0], at[1], at[2]));
    mesh_from_kernel(&body, MeshOperationFamily::Boolean)
        .expect("the kernel's own cube must import")
}

fn run(operation: MeshOperation<'_>) -> MeshOutcome {
    let request = MeshRequest {
        stamp: stamp(),
        operation,
    };
    ManifoldMeshModeler
        .execute(&request, &|| false)
        .expect("the synthetic requests must not be rejected as invalid input")
        .outcome
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

/// Signed volume from the neutral buffers, independent of the kernel's own
/// (absolute) `volume()` reading. Positive means outward winding.
fn signed_volume(mesh: &MeshData) -> f64 {
    let mut six_volume = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        six_volume += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    six_volume / 6.0
}

#[test]
fn overlapping_cubes_union_subtract_and_intersect_by_volume() {
    let left = cube(2.0, [0.0, 0.0, 0.0]);
    let right = cube(2.0, [1.0, 0.0, 0.0]);

    let union = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Union,
        left: &left,
        right: &right,
    });
    let mesh = union.mesh().expect("union must produce a mesh");
    assert_close(volume_of(mesh).expect("union re-imports"), 12.0);
    assert_eq!(union.topology(), Some(MeshTopology::ClosedManifold));

    let subtract = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Subtract,
        left: &left,
        right: &right,
    });
    let mesh = subtract.mesh().expect("subtract must produce a mesh");
    assert_close(volume_of(mesh).expect("subtract re-imports"), 4.0);

    let intersect = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Intersect,
        left: &left,
        right: &right,
    });
    let mesh = intersect.mesh().expect("intersect must produce a mesh");
    assert_close(volume_of(mesh).expect("intersect re-imports"), 4.0);
    assert_eq!(intersect.topology(), Some(MeshTopology::ClosedManifold));
}

#[test]
fn disjoint_intersection_is_a_named_failure_not_an_empty_success() {
    let left = cube(1.0, [0.0, 0.0, 0.0]);
    let right = cube(1.0, [10.0, 0.0, 0.0]);
    let outcome = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Intersect,
        left: &left,
        right: &right,
    });
    assert!(matches!(outcome, MeshOutcome::Failed { .. }));
    assert!(!outcome.is_success());
    assert!(outcome.mesh().is_none());
    assert_eq!(
        outcome.diagnostics().first().map(|row| row.code.as_str()),
        Some(codes::MESH_FAILED)
    );
}

#[test]
fn an_open_operand_is_refused_with_the_stable_not_closed_code() {
    let closed = cube(1.0, [0.0, 0.0, 0.0]);
    let mut open = closed.clone();
    open.triangles.pop();
    let outcome = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Union,
        left: &open,
        right: &closed,
    });
    let reason = outcome
        .unsupported()
        .expect("open input must be Unsupported");
    assert_eq!(reason.code, codes::MESH_NOT_CLOSED);
    assert!(outcome.mesh().is_none());
}

#[test]
fn repair_orientation_flips_an_inside_out_cube() {
    let mut inside_out = cube(2.0, [0.0, 0.0, 0.0]);
    for triangle in &mut inside_out.triangles {
        triangle.swap(0, 1);
    }
    assert!(signed_volume(&inside_out) < 0.0);

    let outcome = run(MeshOperation::Repair {
        mesh: &inside_out,
        mode: MeshRepairMode::Orientation,
        rule: MeshWindingRule::Positive,
    });
    let MeshOutcome::Partial { mesh, repair, .. } = outcome else {
        panic!("repair must be a Partial product, got {outcome:?}");
    };
    assert_eq!(repair.mode, MeshRepairMode::Orientation);
    assert_eq!(repair.input.topology, MeshTopology::ClosedManifold);
    assert_eq!(repair.output.topology, MeshTopology::ClosedManifold);
    assert!(repair.is_closed());
    assert!(signed_volume(&mesh) > 0.0);
    assert_close(signed_volume(&mesh), 8.0);
}

#[test]
fn rebuild_solid_returns_a_closed_product() {
    let cube = cube(2.0, [0.0, 0.0, 0.0]);
    let outcome = run(MeshOperation::Repair {
        mesh: &cube,
        mode: MeshRepairMode::Rebuild,
        rule: MeshWindingRule::Nonzero,
    });
    let MeshOutcome::Partial { mesh, repair, .. } = outcome else {
        panic!("rebuild must be a Partial product, got {outcome:?}");
    };
    assert_eq!(repair.mode, MeshRepairMode::Rebuild);
    assert_eq!(repair.rule, MeshWindingRule::Nonzero);
    assert!(repair.is_closed());
    assert_close(volume_of(&mesh).expect("rebuilt mesh re-imports"), 8.0);
}

#[test]
fn pre_cancellation_wins_before_validation() {
    let left = cube(1.0, [0.0, 0.0, 0.0]);
    let right = left.clone();
    let request = MeshRequest {
        stamp: stamp(),
        operation: MeshOperation::Boolean {
            op: MeshBooleanOp::Union,
            left: &left,
            right: &right,
        },
    };
    let error = ManifoldMeshModeler
        .execute(&request, &|| true)
        .expect_err("a pre-cancelled request must not produce a product");
    assert!(matches!(error, CadError::Cancelled));
}

#[test]
fn an_empty_operand_is_invalid_input_not_a_refusal_outcome() {
    let left = MeshData::new();
    let right = cube(1.0, [0.0, 0.0, 0.0]);
    let request = MeshRequest {
        stamp: stamp(),
        operation: MeshOperation::Boolean {
            op: MeshBooleanOp::Union,
            left: &left,
            right: &right,
        },
    };
    assert!(matches!(
        ManifoldMeshModeler.execute(&request, &|| false),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn registration_and_capabilities_match_the_pin() {
    let modeler = ManifoldMeshModeler;
    let registration = modeler.registration();
    assert_eq!(registration.type_key, crate::TYPE_KEY);
    assert!(registration
        .capabilities
        .iter()
        .any(|row| row == "manifold-rust.version=0.19.6"));
    let capabilities = modeler.capabilities();
    assert!(capabilities.boolean);
    assert!(capabilities.robust_boolean);
    assert!(capabilities.repair_orientation);
    assert!(capabilities.rebuild_solid);
}

/// Expand every triangle corner into its own vertex record, like an STL or a
/// per-face SketchUp group.
fn unwelded(mesh: &MeshData) -> MeshData {
    let mut vertices = Vec::with_capacity(mesh.triangles.len() * 3);
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    for triangle in &mesh.triangles {
        let start = vertices.len() as u32;
        for index in triangle {
            vertices.push(mesh.vertices[*index as usize]);
        }
        triangles.push([start, start + 1, start + 2]);
    }
    MeshData::from_parts(vertices, triangles)
}

#[test]
fn an_unwelded_closed_cube_booleans_after_position_welding() {
    let left = unwelded(&cube(2.0, [0.0, 0.0, 0.0]));
    let right = unwelded(&cube(2.0, [1.0, 0.0, 0.0]));
    let outcome = run(MeshOperation::Boolean {
        op: MeshBooleanOp::Union,
        left: &left,
        right: &right,
    });
    let mesh = outcome
        .mesh()
        .expect("an unwelded but geometrically closed cube must boolean");
    assert_close(volume_of(mesh).expect("union re-imports"), 12.0);
    assert_eq!(outcome.topology(), Some(MeshTopology::ClosedManifold));
}
