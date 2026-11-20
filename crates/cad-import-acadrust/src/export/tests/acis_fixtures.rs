//! Shared fixtures for the ACIS writer tests: authored planar B-reps, the
//! comparison helpers and the export-side body/block builders.
//!
//! Kept in one module so `tests::acis` (writer round trip and refusals) and
//! `tests::acis_export` (export contracts) use the *same* geometry instead of
//! two drifting copies, and so each test file stays inside the 1000-line gate.

use super::*;

use cad_domain::{BrepCurve, BrepFace, BrepLoop, BrepShell, BrepSurface};
use cad_kernel_adapter::{
    tessellate_exchange, SolidExchange, TessellationBudget, TessellationTolerance,
};

use crate::export::acis::brep_to_sat;
use crate::sat_to_brep;

pub(super) fn add(a: Point3, b: Point3) -> Point3 {
    p(a.x + b.x, a.y + b.y, a.z + b.z)
}

pub(super) fn sub(a: Point3, b: Point3) -> Point3 {
    p(a.x - b.x, a.y - b.y, a.z - b.z)
}

pub(super) fn scale(a: Point3, s: f64) -> Point3 {
    p(a.x * s, a.y * s, a.z * s)
}

pub(super) fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub(super) fn cross(a: Point3, b: Point3) -> Point3 {
    p(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

pub(super) fn norm(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub(super) fn unit(a: Point3) -> Point3 {
    scale(a, 1.0 / norm(a))
}

/// Newell's area vector of a polygon; its direction is the polygon's normal.
pub(super) fn newell(points: &[Point3]) -> Point3 {
    let mut normal = p(0.0, 0.0, 0.0);
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        normal = add(
            normal,
            p(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            ),
        );
    }
    normal
}

/// A planar face from an *outward* normal and an unordered ring: the ring is
/// re-wound to be counter-clockwise about the outward normal when needed, so a
/// fixture cannot accidentally author an inside-out (negative-volume) solid.
pub(super) fn planar_face(id: u32, outward: Point3, ring: &[Point3]) -> BrepFace {
    let mut ring = ring.to_vec();
    if dot(newell(&ring), outward) < 0.0 {
        ring.reverse();
    }
    let count = ring.len();
    let edges = (0..count)
        .map(|index| BrepCurve::Line {
            start: ring[index],
            end: ring[(index + 1) % count],
        })
        .collect();
    BrepFace {
        id,
        surface: BrepSurface::Plane {
            origin: ring[0],
            normal: unit(outward),
            u_dir: unit(sub(ring[1], ring[0])),
        },
        reversed: false,
        loops: vec![BrepLoop { edges }],
        tolerance: None,
        domain: None,
    }
}

pub(super) fn single_shell(faces: Vec<BrepFace>) -> BrepData {
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
        tolerance: None,
    }
}

/// An axis-aligned box `l × w × h` with one corner at the origin.
pub(super) fn box_brep(l: f64, w: f64, h: f64) -> BrepData {
    single_shell(vec![
        planar_face(
            0,
            p(0.0, 0.0, -1.0),
            &[
                p(0.0, 0.0, 0.0),
                p(l, 0.0, 0.0),
                p(l, w, 0.0),
                p(0.0, w, 0.0),
            ],
        ),
        planar_face(
            1,
            p(0.0, 0.0, 1.0),
            &[p(0.0, 0.0, h), p(l, 0.0, h), p(l, w, h), p(0.0, w, h)],
        ),
        planar_face(
            2,
            p(0.0, -1.0, 0.0),
            &[
                p(0.0, 0.0, 0.0),
                p(l, 0.0, 0.0),
                p(l, 0.0, h),
                p(0.0, 0.0, h),
            ],
        ),
        planar_face(
            3,
            p(0.0, 1.0, 0.0),
            &[p(0.0, w, 0.0), p(l, w, 0.0), p(l, w, h), p(0.0, w, h)],
        ),
        planar_face(
            4,
            p(-1.0, 0.0, 0.0),
            &[
                p(0.0, 0.0, 0.0),
                p(0.0, w, 0.0),
                p(0.0, w, h),
                p(0.0, 0.0, h),
            ],
        ),
        planar_face(
            5,
            p(1.0, 0.0, 0.0),
            &[p(l, 0.0, 0.0), p(l, w, 0.0), p(l, w, h), p(l, 0.0, h)],
        ),
    ])
}

/// A right triangular prism: right triangle `l × w` in the XY plane, height `h`.
pub(super) fn wedge_brep(l: f64, w: f64, h: f64) -> BrepData {
    single_shell(vec![
        planar_face(
            0,
            p(0.0, 0.0, -1.0),
            &[p(0.0, 0.0, 0.0), p(l, 0.0, 0.0), p(0.0, w, 0.0)],
        ),
        planar_face(
            1,
            p(0.0, 0.0, 1.0),
            &[p(0.0, 0.0, h), p(l, 0.0, h), p(0.0, w, h)],
        ),
        planar_face(
            2,
            p(0.0, -1.0, 0.0),
            &[
                p(0.0, 0.0, 0.0),
                p(l, 0.0, 0.0),
                p(l, 0.0, h),
                p(0.0, 0.0, h),
            ],
        ),
        planar_face(
            3,
            p(-1.0, 0.0, 0.0),
            &[
                p(0.0, 0.0, 0.0),
                p(0.0, w, 0.0),
                p(0.0, w, h),
                p(0.0, 0.0, h),
            ],
        ),
        planar_face(
            4,
            p(w, l, 0.0),
            &[p(l, 0.0, 0.0), p(0.0, w, 0.0), p(0.0, w, h), p(l, 0.0, h)],
        ),
    ])
}

/// A square pyramid: base side `s`, apex height `h` above the base centre.
pub(super) fn pyramid_brep(s: f64, h: f64) -> BrepData {
    let apex = p(s / 2.0, s / 2.0, h);
    let corners = [
        p(0.0, 0.0, 0.0),
        p(s, 0.0, 0.0),
        p(s, s, 0.0),
        p(0.0, s, 0.0),
    ];
    let mut faces = vec![planar_face(0, p(0.0, 0.0, -1.0), &corners)];
    for index in 0..4 {
        let start = corners[index];
        let end = corners[(index + 1) % 4];
        // The Newell normal of the triangle is the outward normal of that side.
        let outward = newell(&[start, end, apex]);
        faces.push(planar_face(index as u32 + 1, outward, &[start, end, apex]));
    }
    single_shell(faces)
}

pub(super) fn diagonal(brep: &BrepData) -> f64 {
    let mut min = p(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = p(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for shell in &brep.shells {
        for face in &shell.faces {
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    if let BrepCurve::Line { start, end } = curve {
                        for point in [start, end] {
                            min = p(min.x.min(point.x), min.y.min(point.y), min.z.min(point.z));
                            max = p(max.x.max(point.x), max.y.max(point.y), max.z.max(point.z));
                        }
                    }
                }
            }
        }
    }
    norm(sub(max, min))
}

/// Assert the parsed B-rep reproduces the authored one: same face/loop/edge
/// counts, exactly equal plane surfaces, and geometry within `1e-9·diagonal`.
pub(super) fn assert_brep_equivalent(expected: &BrepData, actual: &BrepData) {
    let tolerance = 1e-9 * diagonal(expected);
    let close = |a: Point3, b: Point3| norm(sub(a, b)) <= tolerance;

    assert_eq!(actual.shells.len(), expected.shells.len(), "shell count");
    assert!(
        actual.placement.is_none(),
        "the placement is baked, so no transform record is written"
    );
    for (expected_shell, actual_shell) in expected.shells.iter().zip(&actual.shells) {
        assert_eq!(
            actual_shell.faces.len(),
            expected_shell.faces.len(),
            "face count"
        );
        for (expected_face, actual_face) in expected_shell.faces.iter().zip(&actual_shell.faces) {
            let BrepSurface::Plane {
                origin: expected_origin,
                normal: expected_normal,
                u_dir: expected_u,
            } = &expected_face.surface
            else {
                panic!("fixture face {} is not planar", expected_face.id);
            };
            let BrepSurface::Plane {
                origin,
                normal,
                u_dir,
            } = &actual_face.surface
            else {
                panic!("round-tripped face must stay planar");
            };
            assert!(close(*expected_origin, *origin), "plane origin");
            assert!(close(*expected_normal, *normal), "plane normal");
            assert!(close(*expected_u, *u_dir), "plane u direction");
            assert_eq!(actual_face.reversed, expected_face.reversed, "face sense");
            assert_eq!(
                actual_face.loops.len(),
                expected_face.loops.len(),
                "loop count"
            );
            for (expected_loop, actual_loop) in expected_face.loops.iter().zip(&actual_face.loops) {
                assert_eq!(
                    actual_loop.edges.len(),
                    expected_loop.edges.len(),
                    "coedge count"
                );
                for (expected_curve, actual_curve) in
                    expected_loop.edges.iter().zip(&actual_loop.edges)
                {
                    let BrepCurve::Line { start, end } = expected_curve else {
                        panic!("fixture curve is not a line");
                    };
                    let BrepCurve::Line {
                        start: actual_start,
                        end: actual_end,
                    } = actual_curve
                    else {
                        panic!("round-tripped curve must stay a line");
                    };
                    assert!(close(*start, *actual_start), "curve start");
                    assert!(close(*end, *actual_end), "curve end");
                }
            }
        }
    }
}

pub(super) fn tessellated_volume(brep: BrepData) -> f64 {
    let outcome = tessellate_exchange(
        &SolidExchange::Brep(brep),
        TessellationTolerance::default(),
        TessellationBudget::default(),
    );
    let mesh = outcome.mesh().expect("a planar solid tessellates");
    let mut volume = 0.0;
    for triangle in &mesh.mesh.triangles {
        let a = mesh.mesh.vertices[triangle[0] as usize];
        let b = mesh.mesh.vertices[triangle[1] as usize];
        let c = mesh.mesh.vertices[triangle[2] as usize];
        volume += dot(a, cross(b, c));
    }
    volume / 6.0
}

pub(super) fn round_trip(brep: &BrepData) -> BrepData {
    let sat = brep_to_sat(brep, AcisVersion::default()).expect("the planar fixture is writable");
    let text = acadrust::SatWriter::write(&sat);
    let parsed = acadrust::SatDocument::parse(&text).expect("acadrust parses its own SAT text");
    sat_to_brep(&parsed)
}

/// Inclusive axis-aligned bounding box of every boundary point.
pub(super) fn brep_bounds(brep: &BrepData) -> (Point3, Point3) {
    let mut min = p(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = p(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for shell in &brep.shells {
        for face in &shell.faces {
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    if let BrepCurve::Line { start, end } = curve {
                        for point in [start, end] {
                            min = p(min.x.min(point.x), min.y.min(point.y), min.z.min(point.z));
                            max = p(max.x.max(point.x), max.y.max(point.y), max.z.max(point.z));
                        }
                    }
                }
            }
        }
    }
    (min, max)
}

/// Every written boundary point must lie in its own face plane, and every face
/// frame must be an orthonormal pair.
pub(super) fn assert_loops_lie_in_their_planes(brep: &BrepData, tolerance: f64) {
    for shell in &brep.shells {
        for face in &shell.faces {
            let BrepSurface::Plane {
                origin,
                normal,
                u_dir,
            } = &face.surface
            else {
                panic!("every written face is planar");
            };
            assert!(
                (norm(*normal) - 1.0).abs() <= 1e-12,
                "the written plane normal is unit length"
            );
            assert!(
                (norm(*u_dir) - 1.0).abs() <= 1e-12,
                "the written plane u direction is unit length"
            );
            assert!(
                dot(*normal, *u_dir).abs() <= 1e-9,
                "the written plane frame is orthogonal"
            );
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    let BrepCurve::Line { start, end } = curve else {
                        panic!("every written boundary curve is a line");
                    };
                    for point in [start, end] {
                        let distance = dot(*normal, sub(*point, *origin)).abs();
                        assert!(
                            distance <= tolerance,
                            "face {} boundary point {point:?} is {distance} off its plane",
                            face.id
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Export contracts
// ---------------------------------------------------------------------------

pub(super) fn solid_body_with_display(
    exact: BrepData,
    mesh: Mesh,
    edges: Vec<Vec<Point3>>,
) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Solid,
        exact: BodyExact::Exact(Box::new(exact)),
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: Some(0.01),
            precision: Precision::Analytic,
            completeness: Completeness::Complete,
        },
    }))
}

pub(super) fn solid_body(exact: BrepData) -> SemanticGeometry {
    solid_body_with_display(
        exact,
        Mesh {
            vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
            triangles: vec![[0, 1, 2]],
            normals: vec![p(0.0, 0.0, 1.0); 3],
            face_sources: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
        },
        vec![vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)]],
    )
}

// ---------------------------------------------------------------------------
// The AC1024 switch is document-global (model space *and* block definitions)
// ---------------------------------------------------------------------------

pub(super) fn read_document(bytes: &[u8]) -> CadDocument {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .expect("the written DXF parses")
        .read()
        .expect("the written DXF reads")
}

pub(super) fn block_entities(document: &CadDocument, name: &str) -> Vec<EntityType> {
    let record = document
        .block_records
        .get(name)
        .unwrap_or_else(|| panic!("no BLOCK_RECORD named {name}"));
    record
        .entity_handles
        .iter()
        .filter_map(|handle| document.get_entity(*handle).cloned())
        .collect()
}

/// A database with one named block whose single member is an exact planar box,
/// referenced once from model space.
pub(super) fn db_with_blocked_solid() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(1), "BOXBLOCK", p(0.0, 0.0, 0.0))
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "AcDb3dSolid".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(1)),
            geometry: solid_body(box_brep(1.0, 2.0, 3.0)),
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "test".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::translation(p(10.0, 0.0, 0.0)),
            },
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}
