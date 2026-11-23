//! Unit tests for the face triangulation and material grouping.
//!
//! Split out of `convert.rs` to keep that file inside the code-size limit.

use super::*;

const IDENTITY_WORLD: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn mesh_with(vertices: Vec<[f64; 3]>, faces: Vec<skpwright::MeshFace>) -> skpwright::Mesh {
    skpwright::Mesh {
        vertices,
        faces,
        ..Default::default()
    }
}

fn face(outer: Vec<u32>) -> skpwright::MeshFace {
    skpwright::MeshFace {
        outer,
        ..Default::default()
    }
}

fn quad() -> (skpwright::Mesh, skpwright::MeshFace) {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    (mesh_with(vertices, vec![]), face(vec![0, 1, 2, 3]))
}

fn triangle_area(a: Point3, b: Point3, c: Point3) -> f64 {
    let ab = [b.x - a.x, b.y - a.y, b.z - a.z];
    let ac = [c.x - a.x, c.y - a.y, c.z - a.z];
    let n = cross(ab, ac);
    (dot(n, n)).sqrt() * 0.5
}

fn total_area(out: &GroupBuffers) -> f64 {
    out.triangles
        .iter()
        .map(|t| {
            triangle_area(
                out.vertices[t[0] as usize],
                out.vertices[t[1] as usize],
                out.vertices[t[2] as usize],
            )
        })
        .sum()
}

/// Every triangle's geometric normal must agree with the stored vertex
/// normal, or back-face culling would hide geometry SketchUp shows.
fn assert_winding_matches_normals(out: &GroupBuffers) {
    for triangle in &out.triangles {
        let a = out.vertices[triangle[0] as usize];
        let b = out.vertices[triangle[1] as usize];
        let c = out.vertices[triangle[2] as usize];
        let geometric = cross(
            [b.x - a.x, b.y - a.y, b.z - a.z],
            [c.x - a.x, c.y - a.y, c.z - a.z],
        );
        let normal = out.normals[triangle[0] as usize];
        let normal = [normal.x, normal.y, normal.z];
        assert!(
            dot(geometric, normal) > 0.0,
            "triangle winding disagrees with its vertex normal"
        );
    }
}

#[test]
fn a_quad_becomes_two_triangles_with_the_face_normal() {
    let (mesh, face) = quad();
    let mut out = GroupBuffers::default();
    emit_face(&mesh, &face, &IDENTITY_WORLD, None, &mut out).expect("a planar quad fills");

    assert_eq!(out.triangles.len(), 2);
    assert!(
        (total_area(&out) - 1.0).abs() < 1e-9,
        "area must be preserved"
    );
    for normal in &out.normals {
        assert!((normal.z - 1.0).abs() < 1e-9, "flat face normal is +Z");
    }
    assert_winding_matches_normals(&out);
}

#[test]
fn a_face_with_a_hole_keeps_the_hole_open() {
    // 4x4 outer square with a 2x2 hole: 12 m² of fill, nothing inside it.
    let vertices = vec![
        [-2.0, -2.0, 0.0],
        [2.0, -2.0, 0.0],
        [2.0, 2.0, 0.0],
        [-2.0, 2.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let mesh = mesh_with(vertices, vec![]);
    let mut with_hole = face(vec![0, 1, 2, 3]);
    with_hole.holes = vec![vec![4, 5, 6, 7]];

    let mut out = GroupBuffers::default();
    emit_face(&mesh, &with_hole, &IDENTITY_WORLD, None, &mut out).expect("donut fills");

    assert!(
        (total_area(&out) - 12.0).abs() < 1e-6,
        "outer minus hole area, got {}",
        total_area(&out)
    );
    for triangle in &out.triangles {
        let pts: Vec<Point3> = triangle.iter().map(|i| out.vertices[*i as usize]).collect();
        let cx = (pts[0].x + pts[1].x + pts[2].x) / 3.0;
        let cy = (pts[0].y + pts[1].y + pts[2].y) / 3.0;
        assert!(
            !(cx > -0.999 && cx < 0.999 && cy > -0.999 && cy < 0.999),
            "a triangle lies inside the hole at ({cx}, {cy})"
        );
    }
    assert_winding_matches_normals(&out);
}

#[test]
fn a_mirroring_instance_flips_winding_and_normal() {
    let (mesh, face) = quad();
    let mirrored_world = [
        -1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    assert!(determinant3(&mirrored_world) < 0.0);
    let mut out = GroupBuffers::default();
    emit_face(&mesh, &face, &mirrored_world, None, &mut out).expect("a mirrored quad fills");

    for normal in &out.normals {
        assert!(
            (normal.z + 1.0).abs() < 1e-9,
            "mirroring negates the normal"
        );
    }
    assert_winding_matches_normals(&out);
    let xs: Vec<f64> = out.vertices.iter().map(|p| p.x).collect();
    assert!(
        xs.iter().all(|x| *x <= 0.0),
        "mirrored vertices are on -X: {xs:?}"
    );
}

#[test]
fn a_translated_instance_moves_its_vertices() {
    let (mesh, face) = quad();
    let world = [
        1.0, 0.0, 0.0, 10.0, //
        0.0, 1.0, 0.0, 20.0, //
        0.0, 0.0, 1.0, 30.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    let mut out = GroupBuffers::default();
    emit_face(&mesh, &face, &world, None, &mut out).expect("a translated quad fills");
    for vertex in &out.vertices {
        assert!((vertex.x - 10.0).abs() < 1e-9 || (vertex.x - 11.0).abs() < 1e-9);
        assert!((vertex.y - 20.0).abs() < 1e-9 || (vertex.y - 21.0).abs() < 1e-9);
        assert!((vertex.z - 30.0).abs() < 1e-9);
    }
}

#[test]
fn an_invisible_tag_and_a_hidden_face_are_pruned() {
    let mut visible = face(vec![0, 1, 2, 3]);
    visible.layer = 5;
    let mut on_invisible_tag = face(vec![0, 1, 2, 3]);
    on_invisible_tag.layer = 6;
    let mut hidden = face(vec![0, 1, 2, 3]);
    hidden.hidden = true;
    let mesh = mesh_with(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        vec![visible, on_invisible_tag, hidden],
    );
    let mut stats = SkpStats::default();
    let visible_tag = |slot: u16| slot != 6;
    let groups = group_faces(&mesh, 0, &visible_tag, &mut stats);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[&0].len(), 1);
    assert_eq!(stats.hidden_faces_skipped, 1);
    assert_eq!(stats.invisible_layer_faces_skipped, 1);
}

#[test]
fn an_inherited_material_paints_only_unpainted_faces() {
    let mut own = face(vec![0, 1, 2, 3]);
    own.front_material = Some(7);
    let inherited = face(vec![0, 1, 2, 3]);
    let mesh = mesh_with(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        vec![own, inherited],
    );
    let mut stats = SkpStats::default();
    let groups = group_faces(&mesh, 3, &|_| true, &mut stats);

    assert_eq!(groups[&7].len(), 1, "the face's own material wins");
    assert_eq!(
        groups[&3].len(),
        1,
        "the rest inherit the instance material"
    );
}

#[test]
fn a_degenerate_face_is_reported_instead_of_approximated() {
    // Three collinear vertices: the filler refuses rather than inventing fill.
    let mesh = mesh_with(
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        vec![],
    );
    let mut degenerate = face(vec![0, 1, 2]);
    degenerate.normal = [0.0, 0.0, 1.0];
    let mut out = GroupBuffers::default();
    let result = emit_face(&mesh, &degenerate, &IDENTITY_WORLD, None, &mut out);

    assert_eq!(result, Err(FaceSkip::Untriangulable));
    assert!(out.vertices.is_empty() && out.triangles.is_empty());
}

#[test]
fn a_ring_index_outside_the_run_is_malformed() {
    let mesh = mesh_with(vec![[0.0, 0.0, 0.0]], vec![]);
    let mut out = GroupBuffers::default();
    let result = emit_face(&mesh, &face(vec![0, 1, 2]), &IDENTITY_WORLD, None, &mut out);
    assert_eq!(result, Err(FaceSkip::Malformed));
}
