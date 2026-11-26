//! Contract tests for the kernel-independent mesh seam: validation, topology
//! classification and capability probes.

use cad_domain::Point3;

use super::*;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A unit cube, wound CCW from outside.
fn cube() -> MeshData {
    MeshData::from_parts(
        vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ],
        vec![
            [0, 2, 1],
            [0, 3, 2],
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
        ],
    )
}

#[test]
fn a_closed_cube_is_closed_manifold() {
    let report = inspect_topology(&cube());
    assert_eq!(report.topology, MeshTopology::ClosedManifold);
    assert_eq!(report.boundary_edges, 0);
    assert_eq!(report.non_manifold_edges, 0);
    assert_eq!(report.edges, 18);
}

#[test]
fn dropping_a_triangle_makes_a_named_open_shell() {
    let mut open = cube();
    open.triangles.pop();
    let report = inspect_topology(&open);
    assert_eq!(report.topology, MeshTopology::OpenShell);
    assert_eq!(report.boundary_edges, 3);
}

#[test]
fn a_duplicated_triangle_makes_non_manifold_edges() {
    let mut doubled = cube();
    doubled.triangles.push([0, 2, 1]);
    let report = inspect_topology(&doubled);
    assert_eq!(report.topology, MeshTopology::NonManifold);
    assert_eq!(report.non_manifold_edges, 3);
}

#[test]
fn an_empty_mesh_is_not_closed() {
    let report = inspect_topology(&MeshData::new());
    assert_eq!(report.topology, MeshTopology::OpenShell);
    assert_eq!(report.edges, 0);
    assert!(!report.topology.is_closed());
}

#[test]
fn validation_rejects_empty_out_of_range_and_repeated_indices() {
    assert!(MeshData::new().validate().is_err());

    let mut out_of_range = cube();
    out_of_range.triangles[0][0] = 99;
    assert!(out_of_range.validate().is_err());

    let mut repeated = cube();
    let first = repeated.triangles[0][0];
    repeated.triangles[0][1] = first;
    assert!(repeated.validate().is_err());

    let mut non_finite = cube();
    non_finite.vertices[0].x = f64::NAN;
    assert!(non_finite.validate().is_err());
}

#[test]
fn capabilities_cover_the_declared_families() {
    let all = MeshModelerCapabilities::all();
    let left = cube();
    let right = cube();
    assert!(all.supports(&MeshOperation::Boolean {
        op: MeshBooleanOp::Union,
        left: &left,
        right: &right,
    }));
    assert!(all.supports(&MeshOperation::Repair {
        mesh: &left,
        mode: MeshRepairMode::Orientation,
        rule: MeshWindingRule::Positive,
    }));
    assert!(
        !MeshModelerCapabilities::default().supports(&MeshOperation::Boolean {
            op: MeshBooleanOp::Union,
            left: &left,
            right: &right,
        })
    );
}

#[test]
fn an_unwelded_cube_is_closed_when_positions_are_welded() {
    let mut unwelded = MeshData::new();
    for triangle in &cube().triangles {
        let start = unwelded.vertices.len() as u32;
        for index in triangle {
            unwelded.vertices.push(cube().vertices[*index as usize]);
        }
        unwelded.triangles.push([start, start + 1, start + 2]);
    }
    assert_eq!(
        inspect_topology(&unwelded).topology,
        MeshTopology::OpenShell
    );
    let welded = inspect_topology_welded(&unwelded);
    assert_eq!(welded.topology, MeshTopology::ClosedManifold);
    assert_eq!(welded.boundary_edges, 0);
    assert!(welded.welded_vertices > 0);
}
