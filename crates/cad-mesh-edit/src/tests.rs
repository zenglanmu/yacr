//! Contract tests for the kernel-free mesh edit layer.

use cad_domain::Revision;
use cad_domain::{CadError, Mesh, SubElementId};

use crate::*;

fn p(x: f64, y: f64, z: f64) -> cad_domain::Point3 {
    point(x, y, z)
}

/// A closed outward-wound 2×2×2 cube. The two top triangles carry the source key
/// `top`; every other triangle carries no source id.
fn cube() -> Mesh {
    let vertices = vec![
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(0.0, 0.0, 2.0),
        p(2.0, 0.0, 2.0),
        p(2.0, 2.0, 2.0),
        p(0.0, 2.0, 2.0),
    ];
    let triangles = vec![
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
    ];
    let mut face_sources = vec![None; triangles.len()];
    let top = SubElementId {
        source_key: "top".into(),
        topology_revision: Revision(0),
    };
    face_sources[2] = Some(top.clone());
    face_sources[3] = Some(top);
    Mesh {
        vertices,
        triangles,
        normals: Vec::new(),
        face_sources,
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

fn with_top_hole() -> Mesh {
    let mut mesh = cube();
    // Remove the two top triangles (indices 2 and 3).
    mesh.triangles.remove(3);
    mesh.triangles.remove(2);
    mesh.face_sources.remove(3);
    mesh.face_sources.remove(2);
    mesh
}

fn signed_volume(mesh: &Mesh) -> f64 {
    let mut six = 0.0;
    for triangle in &mesh.triangles {
        let a = mesh.vertices[triangle[0] as usize];
        let b = mesh.vertices[triangle[1] as usize];
        let c = mesh.vertices[triangle[2] as usize];
        six += a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x)
            + a.z * (b.x * c.y - b.y * c.x);
    }
    six / 6.0
}

fn error_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a typed refusal, got {other:?}"),
    }
}

#[test]
fn welding_merges_stl_like_duplicate_vertices() {
    let mut mesh = cube();
    // Explode to one vertex record per corner, like an STL.
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for triangle in &mesh.triangles {
        let start = vertices.len() as u32;
        for corner in triangle {
            vertices.push(mesh.vertices[*corner as usize]);
        }
        triangles.push([start, start + 1, start + 2]);
    }
    mesh.vertices = vertices;
    mesh.triangles = triangles;
    // The exploded cube's face_sources no longer align with the new order; the
    // welding path treats an empty list as no identity.
    mesh.face_sources.clear();
    let welded = weld(&mesh).expect("an unwelded cube welds");
    assert_eq!(welded.vertices.len(), 8);
    assert_eq!(welded.triangle_count(), 12);
    assert_eq!(classify(&welded).class, MeshClass::ClosedManifold);
}

#[test]
fn boundary_loops_close_a_hole_loop() {
    let mesh = with_top_hole();
    let welded = weld(&mesh).expect("weld");
    assert_eq!(classify(&welded).class, MeshClass::OpenShell);
    let loops = boundary_loops(&welded).expect("a simple rim");
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), 4);
}

#[test]
fn a_face_region_is_one_planar_disk_with_an_outward_normal() {
    let welded = weld(&cube()).expect("weld");
    let region = face_region(&welded, "top").expect("top is a planar disk");
    assert_eq!(region.triangles.len(), 2);
    assert_eq!(region.boundary.len(), 4);
    assert!((region.area - 4.0).abs() < 1e-9);
    assert!(region.normal.z > 0.99);
}

#[test]
fn a_mesh_without_source_ids_refuses_a_face_region() {
    let mut mesh = cube();
    mesh.face_sources.clear();
    let error = face_region(&weld(&mesh).unwrap(), "top").expect_err("no identity");
    assert!(error_text(error).contains("mesh.face_identity_unavailable"));
}

#[test]
fn moving_the_top_face_outward_adds_solid_volume_and_stays_closed() {
    let mesh = cube();
    let moved = move_faces(&mesh, &["top"], p(0.0, 0.0, 1.0)).expect("move the top face");
    assert!(is_closed(&moved).expect("classify"));
    assert_eq!(
        classify(&weld(&moved).unwrap()).class,
        MeshClass::ClosedManifold
    );
    // Cube 8 + one 2×2 face swept by 1.
    assert!((signed_volume(&moved) - 12.0).abs() < 1e-9);
    // The cap keeps the source id; the generated side walls do not.
    let top = moved
        .face_sources
        .iter()
        .filter(|source| source.as_ref().is_some_and(|id| id.source_key == "top"))
        .count();
    assert!(top >= 2);
}

#[test]
fn moving_inward_keeps_a_closed_product() {
    let mesh = cube();
    let moved = move_faces(&mesh, &["top"], p(0.0, 0.0, -1.0)).expect("move inward");
    assert!(is_closed(&moved).expect("classify"));
    assert!((signed_volume(&moved) - 4.0).abs() < 1e-9);
}

#[test]
fn press_pull_uses_the_region_normal() {
    let mesh = cube();
    let extruded = extrude_faces(&mesh, &["top"], 1.0).expect("press/pull out");
    assert!((signed_volume(&extruded) - 12.0).abs() < 1e-9);
}

#[test]
fn an_open_mesh_refuses_a_face_move() {
    let mesh = with_top_hole();
    let error = move_faces(&mesh, &["top"], p(0.0, 0.0, 1.0)).expect_err("open shell");
    assert!(error_text(error).contains("mesh.edit_requires_closed"));
}

#[test]
fn a_geometry_derived_edge_key_resolves_and_moves_both_endpoints() {
    let mesh = cube();
    let key = cad_domain::mesh_edge_key(p(0.0, 0.0, 2.0), p(2.0, 0.0, 2.0));
    let welded = weld(&mesh).expect("weld");
    assert_eq!(
        edge_for_key(&welded, &key).expect("the top edge exists"),
        (4, 5)
    );
    let moved = move_edges(&mesh, &[key.as_str()], p(0.0, 0.0, 1.0)).expect("move edge");
    assert_eq!(
        classify(&weld(&moved).unwrap()).class,
        MeshClass::ClosedManifold
    );
    assert!(signed_volume(&moved) > 8.0);
}

#[test]
fn an_unknown_edge_key_is_refused_by_name() {
    let error =
        move_edges(&cube(), &["mesh:edge:deadbeef"], p(0.0, 0.0, 1.0)).expect_err("no such edge");
    assert!(error_text(error).contains("mesh.edge_not_found"));
}

#[test]
fn a_picked_edge_key_is_stable_across_the_same_positions() {
    let mesh = cube();
    let key = cad_domain::mesh_edge_key(p(0.0, 0.0, 2.0), p(2.0, 0.0, 2.0));
    let again = cad_domain::mesh_edge_key(p(2.0, 0.0, 2.0), p(0.0, 0.0, 2.0));
    assert_eq!(key, again, "endpoint order does not change the key");
    let moved = move_edges(&mesh, &[key.as_str()], p(0.0, 0.0, 1.0)).expect("move");
    assert!(
        edge_for_key(&weld(&moved).unwrap(), &key).is_err(),
        "moving the edge changes its key"
    );
}

#[test]
fn subdividing_a_closed_cube_keeps_it_closed_and_preserves_face_sources() {
    let mesh = cube();
    let refined = subdivide(&mesh, None).expect("subdivide");
    assert_eq!(refined.triangles.len(), 48);
    assert_eq!(
        classify(&weld(&refined).unwrap()).class,
        MeshClass::ClosedManifold
    );
    let top = refined
        .face_sources
        .iter()
        .filter(|source| source.as_ref().is_some_and(|id| id.source_key == "top"))
        .count();
    assert_eq!(top, 8, "each of the two top triangles becomes four");
}

#[test]
fn smoothing_a_closed_cube_stays_closed() {
    let mesh = cube();
    let smoothed = smooth(&mesh, 1, 0.5).expect("smooth");
    assert_eq!(
        classify(&weld(&smoothed).unwrap()).class,
        MeshClass::ClosedManifold
    );
    assert!(signed_volume(&smoothed) > 0.0);
}

#[test]
fn filling_the_removed_top_hole_closes_the_cube() {
    let mesh = with_top_hole();
    let result = fill_holes(&mesh).expect("fill the one top hole");
    assert_eq!(result.filled, 1);
    assert_eq!(result.refused, 0);
    assert_eq!(result.holes[0].triangles, 2);
    assert!(result.topology.class.is_closed());
    assert_eq!(result.topology.boundary_edges, 0);
    assert!((signed_volume(&result.mesh) - 8.0).abs() < 1e-9);
}

#[test]
fn filling_two_separate_holes_closes_both_loops() {
    let mut mesh = cube();
    // Remove both the top and bottom pairs: two independent boundary loops.
    for index in [3usize, 2, 1, 0] {
        mesh.triangles.remove(index);
        mesh.face_sources.remove(index);
    }
    let result = fill_holes(&mesh).expect("two simple loops fill independently");
    assert_eq!(result.filled, 2);
    assert_eq!(result.refused, 0);
    assert!(result.topology.class.is_closed());
    assert!((signed_volume(&result.mesh) - 8.0).abs() < 1e-9);
}

/// A square tube: outer 2×2, inner 1×1, open at the top and bottom. The two
/// annulus rims must be capped without covering the through-hole.
fn square_tube() -> Mesh {
    let vertices = vec![
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
        p(0.0, 0.0, 2.0),
        p(2.0, 0.0, 2.0),
        p(2.0, 2.0, 2.0),
        p(0.0, 2.0, 2.0),
        p(0.5, 0.5, 0.0),
        p(1.5, 0.5, 0.0),
        p(1.5, 1.5, 0.0),
        p(0.5, 1.5, 0.0),
        p(0.5, 0.5, 2.0),
        p(1.5, 0.5, 2.0),
        p(1.5, 1.5, 2.0),
        p(0.5, 1.5, 2.0),
    ];
    let triangles = vec![
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
        [9, 8, 12],
        [9, 12, 13],
        [11, 10, 14],
        [11, 14, 15],
        [8, 11, 15],
        [8, 15, 12],
        [10, 9, 13],
        [10, 13, 14],
    ];
    Mesh {
        vertices,
        triangles: triangles.clone(),
        normals: Vec::new(),
        face_sources: vec![None; triangles.len()],
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

/// A flat 5×5 plate with two disjoint 1×1 holes at cells (1,1) and (3,3).
fn plate_with_two_holes() -> Mesh {
    let size = 6usize;
    let mut vertices = Vec::new();
    for j in 0..size {
        for i in 0..size {
            vertices.push(p(i as f64, j as f64, 0.0));
        }
    }
    let index = |i: usize, j: usize| (j * size + i) as u32;
    let mut triangles = Vec::new();
    for j in 0..size - 1 {
        for i in 0..size - 1 {
            if (i, j) == (1, 1) || (i, j) == (3, 3) {
                continue;
            }
            let a = index(i, j);
            let b = index(i + 1, j);
            let c = index(i + 1, j + 1);
            let d = index(i, j + 1);
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    Mesh {
        vertices,
        triangles: triangles.clone(),
        normals: Vec::new(),
        face_sources: vec![None; triangles.len()],
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

#[test]
fn nested_boundary_loops_fill_the_annulus_between_them() {
    let mesh = plate_with_two_holes();
    let result = fill_holes(&mesh).expect("the three loops are reported");
    assert_eq!(result.filled, 3);
    assert_eq!(result.refused, 0);
    assert_eq!(result.topology.boundary_edges, 0);
    assert!(
        result
            .holes
            .iter()
            .all(|hole| hole.filled && hole.code.is_none()),
        "the outer loop and both nested holes are closed by one annulus patch"
    );
    let outer = result
        .holes
        .iter()
        .find(|hole| hole.triangles > 0)
        .expect("the outer loop owns the patch");
    assert!(outer.detail.contains("nested hole loop"));
    let nested = result
        .holes
        .iter()
        .filter(|hole| hole.triangles == 0 && hole.filled)
        .count();
    assert_eq!(nested, 2);
}

/// A flat 7×7 plate with a 3×3 hole that contains a 1×1 island: the outer
/// frame and the island are separate material regions, and the moat between
/// them must stay open.
fn plate_with_island() -> Mesh {
    let size = 8usize;
    let mut vertices = Vec::new();
    for j in 0..size {
        for i in 0..size {
            vertices.push(p(i as f64, j as f64, 0.0));
        }
    }
    let index = |i: usize, j: usize| (j * size + i) as u32;
    let removed = [
        (2usize, 2usize),
        (3, 2),
        (4, 2),
        (2, 3),
        (4, 3),
        (2, 4),
        (3, 4),
        (4, 4),
    ];
    let mut triangles = Vec::new();
    for j in 0..size - 1 {
        for i in 0..size - 1 {
            if removed.contains(&(i, j)) {
                continue;
            }
            let a = index(i, j);
            let b = index(i + 1, j);
            let c = index(i + 1, j + 1);
            let d = index(i, j + 1);
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    Mesh {
        vertices,
        triangles: triangles.clone(),
        normals: Vec::new(),
        face_sources: vec![None; triangles.len()],
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

#[test]
fn a_nested_island_is_filled_separately_from_the_frame_annulus() {
    let mesh = plate_with_island();
    let result = fill_holes(&mesh).expect("the nested loops are reported");
    assert_eq!(
        result.filled, 3,
        "outer frame, its hole, and the island close"
    );
    assert_eq!(result.refused, 0);
    assert_eq!(result.topology.boundary_edges, 0);
    let patch_owners = result
        .holes
        .iter()
        .filter(|hole| hole.triangles > 0)
        .count();
    let nested_holes = result
        .holes
        .iter()
        .filter(|hole| hole.filled && hole.triangles == 0)
        .count();
    assert_eq!(
        patch_owners, 2,
        "the frame annulus and the island each get a patch"
    );
    assert_eq!(
        nested_holes, 1,
        "only the frame hole is owned by the outer patch"
    );
    assert!(result
        .holes
        .iter()
        .any(|hole| hole.filled && hole.detail.contains("nested hole")));
}

#[test]
fn a_square_tube_caps_both_annulus_rims_without_covering_the_hole() {
    let mesh = square_tube();
    let result = fill_holes(&mesh).expect("two annulus rims fill");
    assert_eq!(result.filled, 4, "two outer and two nested loops close");
    assert_eq!(result.refused, 0);
    assert_eq!(
        classify(&weld(&result.mesh).unwrap()).class,
        MeshClass::ClosedManifold
    );
    assert_eq!(result.topology.boundary_edges, 0);
    // 2×2×2 outside minus a 1×1×2 through-hole.
    assert!((signed_volume(&result.mesh) - 6.0).abs() < 1e-9);
}

#[test]
fn a_closed_mesh_refuses_hole_filling_by_name() {
    let error = fill_holes(&cube()).expect_err("no holes");
    assert!(error_text(error).contains("mesh.no_holes"));
}

#[test]
fn a_non_planar_hole_is_filled_by_projection_and_reported() {
    let mut mesh = with_top_hole();
    // Lift one top-rim vertex out of the plane.
    mesh.vertices[4].z += 1.0;
    let result = fill_holes(&mesh).expect("a simple projected loop fills");
    assert_eq!(result.filled, 1);
    assert_eq!(result.refused, 0);
    assert!(result.holes[0].non_planar);
    assert!(result.holes[0].detail.contains("non-planar"));
    assert!(result.topology.class.is_closed());
    require_filled(&result).expect("one hole was filled");
}

#[test]
fn a_degenerate_hole_is_reported_and_left_open() {
    let mesh = Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: vec![None],
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let result = fill_holes(&mesh).expect("the report is still a product");
    assert_eq!(result.filled, 0);
    assert_eq!(result.refused, 1);
    assert_eq!(result.holes[0].code, Some("mesh.hole_degenerate"));
    let error = require_filled(&result).expect_err("nothing was filled");
    assert!(error_text(error).contains("mesh.fill_failed"));
}

#[test]
fn a_zero_distance_press_pull_is_refused() {
    let error = extrude_faces(&cube(), &["top"], 0.0).expect_err("zero distance");
    assert!(error_text(error).contains("mesh.edit_zero_distance"));
}
