use cad_domain::*;

use super::bounds::{cached_local_pick_bounds, local_pick_bounds, PickBoundsCache};
use super::hit::segment_hit;
use super::math::{is_finite_point, length3};
use super::*;

#[cfg(test)]
#[test]
fn borrowed_picking_matches_owned_single_hit() {
    let source = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    };
    let items = vec![PickItem {
        source: source.clone(),
        geometry: SemanticGeometry::Line {
            start: p(-1.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        transform: Transform3::identity(),
        geometry_source: GeometrySource::Analytic,
    }];
    let ray = Ray3 {
        origin: p(0.0, 0.0, 1.0),
        direction: p(0.0, 0.0, -1.0),
    };
    let options = PickOptions::new(0.1).unwrap();
    let owned = pick_closest(&ray, &items, &options).unwrap();
    let borrowed = pick_closest_borrowed(
        &ray,
        items
            .iter()
            .map(|item| (&item.source, &item.geometry, &item.transform)),
        &options,
    )
    .unwrap();
    assert_eq!(borrowed, owned);
    assert!(borrowed.hit.is_some());
}
use cad_domain::{DocumentId, EntityId, InstancePath, Revision, SubElementId};

#[test]
fn borrowed_pick_matches_owning_hits_ties_and_unsupported_reports() {
    let source = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    };
    let mut items = vec![PickItem {
        source,
        geometry: SemanticGeometry::Line {
            start: p(-1.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        transform: Transform3::identity(),
        geometry_source: GeometrySource::Analytic,
    }];
    let mut second = items[0].clone();
    second.source.entity = EntityId(2);
    items.push(second);
    items.push(item(
        3,
        SemanticGeometry::Opaque {
            type_key: "PROXY".into(),
            version: 1,
            payload: vec![1],
        },
    ));
    let ray = Ray3 {
        origin: p(0.0, 0.0, 1.0),
        direction: p(0.0, 0.0, -1.0),
    };
    let options = PickOptions::new(0.1).unwrap();
    let owned = pick_closest(&ray, &items, &options).unwrap();
    let borrowed = pick_closest_borrowed(
        &ray,
        items
            .iter()
            .map(|item| (&item.source, &item.geometry, &item.transform)),
        &options,
    )
    .unwrap();
    assert_eq!(owned, borrowed);
    assert_eq!(borrowed.skipped.len(), 1);
    assert_eq!(borrowed.hit.unwrap().source.entity, EntityId(1));
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ray(origin: Point3, direction: Point3) -> Ray3 {
    Ray3 { origin, direction }
}

fn source(id: u128, instance: Vec<u128>) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath(instance.into_iter().map(EntityId).collect()),
        sub_element: None,
    }
}

fn item(id: u128, geometry: SemanticGeometry) -> PickItem {
    PickItem {
        source: source(id, Vec::new()),
        geometry,
        transform: Transform3::identity(),
        geometry_source: GeometrySource::Analytic,
    }
}

fn local_pick_bounds_reference(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
    let mut points = Vec::new();
    let mut radius = 0.0;
    match geometry {
        SemanticGeometry::Line { start, end } => points.extend([*start, *end]),
        SemanticGeometry::Point(point) => points.push(*point),
        SemanticGeometry::Shape { position, .. } => points.push(*position),
        SemanticGeometry::Mesh(_) => return None,
        SemanticGeometry::Polyline {
            points: vertices,
            bulges,
            ..
        } if bulges.iter().all(|bulge| *bulge == 0.0) => points.extend(vertices.iter().copied()),
        SemanticGeometry::Mask { boundary, .. } => points.extend(boundary.iter().copied()),
        SemanticGeometry::Circle {
            center, radius: r, ..
        }
        | SemanticGeometry::Arc {
            center, radius: r, ..
        } => {
            points.push(*center);
            radius = r.abs();
        }
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            ..
        } => {
            points.push(*center);
            radius = length3(*major_axis) * (1.0 + ratio.abs());
        }
        _ => return None,
    }
    if points.is_empty() || !radius.is_finite() || points.iter().any(|p| !is_finite_point(*p)) {
        return None;
    }
    let mut min = points[0];
    let mut max = points[0];
    for point in points.iter().skip(1) {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    Some((
        p(min.x - radius, min.y - radius, min.z - radius),
        p(max.x + radius, max.y + radius, max.z + radius),
    ))
}

#[test]
fn allocation_free_pick_bounds_match_the_vec_reference() {
    let cases = vec![
        SemanticGeometry::Line {
            start: p(-2.0, 1.0, 3.0),
            end: p(4.0, -1.0, 2.0),
        },
        SemanticGeometry::Point(p(1.0, 2.0, 3.0)),
        SemanticGeometry::Shape {
            shape_name: "shape".into(),
            code: 1,
            position: p(3.0, 2.0, 1.0),
            size: 1.0,
            rotation: 0.0,
            font: None,
        },
        SemanticGeometry::Polyline {
            points: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
            bulges: vec![0.0],
            closed: false,
        },
        SemanticGeometry::Polyline {
            points: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
            bulges: vec![1.0],
            closed: false,
        },
        SemanticGeometry::Mask {
            boundary: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
            inverted: false,
        },
        SemanticGeometry::Circle {
            center: p(1.0, 2.0, 3.0),
            normal: p(0.0, 0.0, 1.0),
            radius: -4.0,
        },
        SemanticGeometry::Arc {
            center: p(1.0, 2.0, 3.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 4.0,
            start: 0.0,
            sweep: 1.0,
        },
        SemanticGeometry::Ellipse {
            center: p(1.0, 2.0, 3.0),
            normal: p(0.0, 0.0, 1.0),
            major_axis: p(2.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: 1.0,
        },
        SemanticGeometry::Polyline {
            points: vec![p(f64::NAN, 0.0, 0.0)],
            bulges: vec![],
            closed: false,
        },
        SemanticGeometry::Mesh(Mesh {
            vertices: vec![p(0.0, 0.0, 0.0)],
            triangles: vec![],
            normals: vec![],
            face_sources: vec![],
            colors: vec![],
        }),
        SemanticGeometry::Opaque {
            type_key: "x".into(),
            version: 1,
            payload: vec![],
        },
    ];
    for geometry in &cases {
        assert_eq!(
            local_pick_bounds(geometry),
            local_pick_bounds_reference(geometry)
        );
    }
}

#[test]
fn bounded_and_unculled_pick_reports_are_identical() {
    let mut first_tie = item(
        10,
        SemanticGeometry::Line {
            start: p(-1.0, 0.0, 3.0),
            end: p(1.0, 0.0, 3.0),
        },
    );
    first_tie.source.instance = InstancePath(vec![EntityId(101)]);
    let mut second_tie = first_tie.clone();
    second_tie.source.instance = InstancePath(vec![EntityId(102)]);
    let mut transformed = item(11, SemanticGeometry::Point(p(0.0, 0.0, 5.0)));
    transformed.transform.matrix[0][3] = 0.05;
    let items = vec![
        item(
            1,
            SemanticGeometry::Circle {
                center: p(100.0, 0.0, 20.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 2.0,
            },
        ),
        first_tie,
        second_tie,
        transformed,
        item(
            12,
            SemanticGeometry::Mask {
                boundary: vec![p(-1.0, 0.0, 7.0), p(1.0, 0.0, 7.0)],
                inverted: false,
            },
        ),
        item(
            13,
            SemanticGeometry::Opaque {
                type_key: "UNKNOWN".into(),
                version: 1,
                payload: vec![],
            },
        ),
        item(
            14,
            SemanticGeometry::Mesh(Mesh {
                vertices: vec![p(0.0, 0.0, 10.0), p(1e9, 0.0, 10.0), p(0.0, 1e9, 10.0)],
                triangles: vec![[0, 1, 2]],
                normals: vec![],
                face_sources: vec![],
                colors: vec![],
            }),
        ),
    ];
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.1)
        .unwrap()
        .with_back_faces(BackFacePolicy::DoubleSided);
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    let bounded = pick_closest(&ray, &items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
    let exhaustive = pick_closest(&ray, &items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    assert_eq!(bounded, exhaustive);
    assert_eq!(
        bounded.hit.unwrap().source.instance,
        InstancePath(vec![EntityId(101)])
    );
}

#[test]
fn bounded_compound_cull_skips_children_and_shared_placements_reuse_local_bounds() {
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.01).unwrap();
    let far = SemanticGeometry::Compound(vec![
        SemanticGeometry::Polyline {
            points: vec![p(100.0, -2.0, 5.0), p(100.0, 2.0, 5.0)],
            bulges: vec![0.0],
            closed: false,
        },
        SemanticGeometry::Circle {
            center: p(100.0, 0.0, 10.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 1.0,
        },
    ]);
    let far_source = source(20, Vec::new());
    let identity = Transform3::identity();
    PICK_NARROW_CALLS.with(|count| count.set(0));
    PICK_BOUNDS_COMPUTED.with(|count| count.set(0));
    CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
    assert!(
        pick_closest_borrowed(&ray, [(&far_source, &far, &identity)], &options,)
            .unwrap()
            .hit
            .is_none()
    );
    PICK_BOUNDS_COMPUTED.with(|count| assert_eq!(count.get(), 1));
    PICK_NARROW_CALLS.with(|count| assert_eq!(count.get(), 0));
    CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));

    // The same immutable geometry is offered at two placements in one
    // query. First placement culls the compound; the second hits and must
    // reuse bounds for the root and both descendants.
    let shared = SemanticGeometry::Compound(vec![
        SemanticGeometry::Compound(vec![SemanticGeometry::Polyline {
            points: vec![p(-1.0, 0.0, 2.0), p(1.0, 0.0, 2.0)],
            bulges: vec![0.0],
            closed: false,
        }]),
        SemanticGeometry::Circle {
            center: p(0.0, 0.0, 4.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 0.5,
        },
    ]);
    let mut aggregate_cache = PickBoundsCache::new();
    assert!(cached_local_pick_bounds(&shared, &mut aggregate_cache).is_some());
    assert_eq!(
        aggregate_cache.len(),
        2,
        "only the two compounds are cached"
    );
    assert!(cached_local_pick_bounds(&shared, &mut aggregate_cache).is_some());
    assert_eq!(aggregate_cache.len(), 2, "a repeated aggregate is reused");
    let mut displaced = Transform3::identity();
    displaced.matrix[0][3] = 100.0;
    let source_a = source(21, vec![1]);
    let source_b = source(21, vec![2]);
    PICK_NARROW_CALLS.with(|count| count.set(0));
    PICK_BOUNDS_COMPUTED.with(|count| count.set(0));
    CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
    let report = pick_closest_borrowed(
        &ray,
        [
            (&source_a, &shared, &displaced),
            (&source_b, &shared, &identity),
        ],
        &options,
    )
    .unwrap();
    assert!(report.hit.is_some());
    PICK_BOUNDS_COMPUTED.with(|count| assert_eq!(count.get(), 2));
    PICK_NARROW_CALLS.with(|count| assert_eq!(count.get(), 4));
    CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));
}

#[test]
fn cached_compound_picks_match_unculled_shared_nested_and_mixed_reports() {
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.1).unwrap();
    let shared = SemanticGeometry::Compound(vec![SemanticGeometry::Compound(vec![
        SemanticGeometry::Polyline {
            points: vec![p(-1.0, 0.0, 2.0), p(1.0, 0.0, 2.0)],
            bulges: vec![0.0],
            closed: false,
        },
    ])]);
    let mut displaced = Transform3::identity();
    displaced.matrix[1][3] = 0.05;
    let mut far = Transform3::identity();
    far.matrix[0][3] = 100.0;
    let source_far = source(30, vec![200]);
    let source_offset = source(30, vec![201]);
    let source_first_tie = source(30, vec![202]);
    let source_second_tie = source(30, vec![203]);
    let identity = Transform3::identity();
    let opaque_mixed = SemanticGeometry::Compound(vec![
        SemanticGeometry::Line {
            start: p(100.0, -1.0, 4.0),
            end: p(100.0, 1.0, 4.0),
        },
        SemanticGeometry::Opaque {
            type_key: "UNKNOWN".into(),
            version: 1,
            payload: vec![],
        },
    ]);
    let source_opaque = source(31, vec![]);
    let items = [
        (&source_far, &shared, &far),
        (&source_offset, &shared, &displaced),
        (&source_first_tie, &shared, &identity),
        (&source_second_tie, &shared, &identity),
        (&source_opaque, &opaque_mixed, &identity),
    ];
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    let bounded = pick_closest_borrowed(&ray, items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
    let exhaustive = pick_closest_borrowed(&ray, items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    assert_eq!(bounded, exhaustive);
    assert_eq!(
        bounded.hit.as_ref().unwrap().source.instance,
        InstancePath(vec![EntityId(202)])
    );
    assert_eq!(bounded.skipped.len(), 1);
    assert_eq!(bounded.skipped[0].reason, "compound-unsupported");
}

#[test]
fn cached_mixed_mesh_compound_preserves_face_identity_vs_unculled_pick() {
    let geometry = SemanticGeometry::Compound(vec![
        SemanticGeometry::Mesh(two_face_mesh()),
        SemanticGeometry::Opaque {
            type_key: "UNKNOWN".into(),
            version: 1,
            payload: vec![],
        },
    ]);
    let source = source(40, vec![401]);
    let transform = Transform3::identity();
    let ray = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6)
        .unwrap()
        .with_back_faces(BackFacePolicy::DoubleSided);
    let items = [(&source, &geometry, &transform)];
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    let bounded = pick_closest_borrowed(&ray, items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
    let exhaustive = pick_closest_borrowed(&ray, items, &options).unwrap();
    BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
    assert_eq!(bounded, exhaustive);
    assert_eq!(
        bounded.hit.as_ref().unwrap().source.sub_element,
        Some(face("face-a"))
    );
}

#[test]
fn conservative_bounds_cull_far_curves_but_preserve_nested_unsupported_children() {
    CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.01).unwrap();
    let far_circle = SemanticGeometry::Circle {
        center: p(100.0, 0.0, 10.0),
        normal: p(0.0, 0.0, 1.0),
        radius: 20.0,
    };
    let compound = SemanticGeometry::Compound(vec![
        far_circle,
        SemanticGeometry::Opaque {
            type_key: "UNKNOWN".into(),
            version: 1,
            payload: Vec::new(),
        },
    ]);
    assert_eq!(
        hit_geometry(&ray, &compound, &Transform3::identity(), &options).unwrap(),
        PickOutcome::Unsupported("compound-unsupported")
    );
    CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));
}

#[test]
fn conservative_bounds_keep_tolerance_hits_and_fall_back_for_bulged_polylines() {
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.1).unwrap();
    let near = SemanticGeometry::Circle {
        center: p(0.15, 0.0, 3.0),
        normal: p(0.0, 0.0, 1.0),
        radius: 0.05,
    };
    assert!(matches!(
        hit_geometry(&ray, &near, &Transform3::identity(), &options).unwrap(),
        PickOutcome::Hit(_)
    ));
    let bulged = SemanticGeometry::Polyline {
        points: vec![p(100.0, 0.0, 0.0), p(101.0, 0.0, 0.0)],
        bulges: vec![1.0],
        closed: false,
    };
    assert!(local_pick_bounds(&bulged).is_none());
    assert!(local_pick_bounds(&SemanticGeometry::Opaque {
        type_key: "UNKNOWN".into(),
        version: 1,
        payload: Vec::new()
    })
    .is_none());
}

#[test]
fn conservative_bounds_account_for_transforms_and_mesh_vertices() {
    let mut transform = Transform3::identity();
    transform.matrix[0][3] = 20.0;
    let point = SemanticGeometry::Point(p(0.0, 0.0, 2.0));
    let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(0.01).unwrap();
    assert_eq!(
        hit_geometry(&ray, &point, &transform, &options).unwrap(),
        PickOutcome::Miss
    );

    let mesh = SemanticGeometry::Mesh(Mesh {
        vertices: vec![p(30.0, 0.0, 1.0)],
        triangles: Vec::new(),
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
    assert_eq!(
        hit_geometry(&ray, &mesh, &Transform3::identity(), &options).unwrap(),
        PickOutcome::Miss
    );
}

#[test]
fn huge_mesh_preserves_barycentric_slack_outside_tolerance_bounds() {
    // The existing triangle test allows u down to -1e-12. Across an edge
    // 1e9 units long, this accepts a point 5e-7 units outside the triangle,
    // even though the pick tolerance is only 1e-9.
    let mesh = SemanticGeometry::Mesh(Mesh {
        vertices: vec![p(0.0, 0.0, 1.0), p(1e9, 0.0, 1.0), p(0.0, 1e9, 1.0)],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    });
    assert!(local_pick_bounds(&mesh).is_none());
    let ray = ray(p(-5e-7, 1.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-9)
        .unwrap()
        .with_back_faces(BackFacePolicy::DoubleSided);
    assert!(matches!(
        hit_geometry(&ray, &mesh, &Transform3::identity(), &options).unwrap(),
        PickOutcome::Hit(_)
    ));
}

#[test]
fn segment_hit_and_miss_with_tolerance() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    // A segment crossing the ray (distance 0).
    let hit = segment_hit(&r, p(5.0, -1.0, 0.0), p(5.0, 1.0, 0.0), 1e-6);
    assert!(hit.is_some());
    // A segment running parallel to the ray, offset by 0.001: beyond a
    // 1e-6 tolerance it is a miss.
    let miss = segment_hit(&r, p(5.0, 0.001, -1.0), p(5.0, 0.001, 1.0), 1e-6);
    assert!(miss.is_none());
}

#[test]
fn tolerance_scales_the_segment_hit_window() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let a = p(5.0, 0.25, 0.0);
    let b = p(5.0, 0.25, 1.0);
    // Distance 0.25: missed with a 0.1 tolerance, hit with 0.5.
    assert!(segment_hit(&r, a, b, 0.1).is_none());
    assert!(segment_hit(&r, a, b, 0.5).is_some());
}

#[test]
fn segment_behind_the_origin_is_a_miss() {
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    // The segment is behind the ray origin.
    assert!(segment_hit(&r, p(-5.0, -1.0, 0.0), p(-5.0, 1.0, 0.0), 10.0).is_none());
}

#[test]
fn mesh_triangle_hit_respects_back_face_policy() {
    // Triangle in the z=5 plane, counter-clockwise seen from -z, so its
    // outward normal points -z (toward a ray that travels +z).
    let v0 = p(-1.0, -1.0, 5.0);
    let v1 = p(0.0, 1.0, 5.0);
    let v2 = p(1.0, -1.0, 5.0);
    // A ray travelling +z from the origin sees the front face.
    let front = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    assert!(ray_triangle(&front, v0, v1, v2, BackFacePolicy::Cull).is_some());
    // A ray travelling -z from beyond the plane sees the back face.
    let back = ray(p(0.0, 0.0, 10.0), p(0.0, 0.0, -1.0));
    assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::Cull).is_none());
    assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::DoubleSided).is_some());
}

#[test]
fn mesh_pick_returns_the_closest_triangle() {
    let near = p(-1.0, -1.0, 2.0);
    let nb = p(0.0, 1.0, 2.0);
    let nc = p(1.0, -1.0, 2.0);
    let far = p(-1.0, -1.0, 5.0);
    let fb = p(0.0, 1.0, 5.0);
    let fc = p(1.0, -1.0, 5.0);
    let mesh = Mesh {
        vertices: vec![near, nb, nc, far, fb, fc],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let r = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let options = PickOptions::new(1e-6).unwrap();
    match hit_geometry(
        &r,
        &SemanticGeometry::Mesh(mesh),
        &Transform3::identity(),
        &options,
    )
    .unwrap()
    {
        PickOutcome::Hit(hit) => {
            assert!((hit.distance - 2.0).abs() < 1e-9, "closest triangle wins");
            assert!((hit.point.z - 2.0).abs() < 1e-9);
        }
        other => panic!("expected a hit, got {other:?}"),
    }
}

#[test]
fn closest_of_many_is_returned_not_the_first() {
    let items = vec![
        item(
            1,
            SemanticGeometry::Line {
                start: p(50.0, -1.0, 0.0),
                end: p(50.0, 1.0, 0.0),
            },
        ),
        item(
            2,
            SemanticGeometry::Line {
                start: p(5.0, -1.0, 0.0),
                end: p(5.0, 1.0, 0.0),
            },
        ),
    ];
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let options = PickOptions::new(1e-6).unwrap();
    let report = pick_closest(&r, &items, &options).unwrap();
    let hit = report.hit.expect("a hit");
    assert_eq!(hit.source.entity, EntityId(2), "closest, not first");
    assert!((hit.distance - 5.0).abs() < 1e-9);
}

#[test]
fn instance_path_is_carried_on_the_hit() {
    let mut a = item(
        20,
        SemanticGeometry::Line {
            start: p(5.0, -1.0, 0.0),
            end: p(5.0, 1.0, 0.0),
        },
    );
    a.source = source(20, vec![3]);
    let mut b = item(
        20,
        SemanticGeometry::Line {
            start: p(5.0, -1.0, 0.0),
            end: p(5.0, 1.0, 0.0),
        },
    );
    b.source = source(20, vec![4]);
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let options = PickOptions::new(1e-6).unwrap();
    let report = pick_closest(&r, &[a, b], &options).unwrap();
    let hit = report.hit.expect("a hit");
    assert_eq!(hit.source.entity, EntityId(20));
    assert_eq!(hit.source.instance, InstancePath(vec![EntityId(3)]));
}

#[test]
fn opaque_geometry_is_skipped_with_a_reason() {
    let items = vec![item(
        9,
        SemanticGeometry::Opaque {
            type_key: "ACAD_PROXY_ENTITY".into(),
            version: 1,
            payload: vec![1, 2, 3],
        },
    )];
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let options = PickOptions::new(1e-6).unwrap();
    let report = pick_closest(&r, &items, &options).unwrap();
    assert!(report.hit.is_none());
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, "opaque");
}

#[test]
fn insert_is_not_silently_claimed_as_a_hit() {
    let items = vec![item(
        3,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        },
    )];
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let options = PickOptions::new(1e-6).unwrap();
    let report = pick_closest(&r, &items, &options).unwrap();
    assert!(report.hit.is_none());
    assert_eq!(report.skipped[0].reason, "insert-not-expanded");
}

#[test]
fn degenerate_ray_and_tolerance_are_refused() {
    let items = vec![item(
        1,
        SemanticGeometry::Line {
            start: p(5.0, -1.0, 0.0),
            end: p(5.0, 1.0, 0.0),
        },
    )];
    let good_options = PickOptions::new(1e-6).unwrap();
    let zero = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0));
    assert!(pick_closest(&zero, &items, &good_options).is_err());
    let nan = ray(p(0.0, 0.0, 0.0), p(f64::NAN, 0.0, 0.0));
    assert!(pick_closest(&nan, &items, &good_options).is_err());
    let good = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    // A zero/NaN tolerance must be refused by `validate` and by `new`.
    let zero_tol = PickOptions {
        tolerance: 0.0,
        back_faces: BackFacePolicy::Cull,
    };
    let nan_tol = PickOptions {
        tolerance: f64::NAN,
        back_faces: BackFacePolicy::Cull,
    };
    assert!(zero_tol.validate().is_err());
    assert!(nan_tol.validate().is_err());
    assert!(pick_closest(&good, &items, &zero_tol).is_err());
    assert!(pick_closest(&good, &items, &nan_tol).is_err());
    assert!(PickOptions::new(0.0).is_err());
    assert!(PickOptions::new(-1.0).is_err());
}

#[test]
fn transform_is_applied_before_testing() {
    let mut item = item(
        1,
        SemanticGeometry::Line {
            start: p(0.0, -1.0, 0.0),
            end: p(0.0, 1.0, 0.0),
        },
    );
    item.transform = Transform3::translation(p(5.0, 0.0, 0.0));
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let report = pick_closest(&r, &[item], &PickOptions::new(1e-6).unwrap()).unwrap();
    let hit = report.hit.expect("translated line is hit at x=5");
    assert!((hit.distance - 5.0).abs() < 1e-9);
}

fn face(key: &str) -> SubElementId {
    SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    }
}

/// Two coplanar triangles at z = 2, side by side along x, each carrying a
/// distinct stable face source.
fn two_face_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            p(-2.0, -1.0, 2.0),
            p(-1.0, 1.0, 2.0),
            p(0.0, -1.0, 2.0),
            p(0.0, -1.0, 2.0),
            p(1.0, 1.0, 2.0),
            p(2.0, -1.0, 2.0),
        ],
        triangles: vec![[0, 1, 2], [3, 4, 5]],
        normals: Vec::new(),
        face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
        colors: Vec::new(),
    }
}

#[test]
fn mesh_face_hit_resolves_the_stable_sub_element() {
    let options = PickOptions::new(1e-6).unwrap();
    let mesh = SemanticGeometry::Mesh(two_face_mesh());
    let transform = Transform3::identity();

    let left = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    match hit_geometry(&left, &mesh, &transform, &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert_eq!(
                hit.sub_element.as_ref().map(|s| s.source_key.as_str()),
                Some("face-a")
            );
            assert!(hit.sub_element_reason.is_none());
            assert!((hit.point.z - 2.0).abs() < 1e-9);
        }
        other => panic!("expected a hit, got {other:?}"),
    }

    let right = ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    match hit_geometry(&right, &mesh, &transform, &options).unwrap() {
        PickOutcome::Hit(hit) => assert_eq!(
            hit.sub_element.as_ref().map(|s| s.source_key.as_str()),
            Some("face-b"),
            "each triangle maps to its own face source"
        ),
        other => panic!("expected a hit, got {other:?}"),
    }
}

#[test]
fn mesh_face_without_a_source_is_unresolved_with_a_reason() {
    let options = PickOptions::new(1e-6).unwrap();
    let transform = Transform3::identity();
    let r = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));

    // No `face_sources` at all.
    let mut absent = two_face_mesh();
    absent.face_sources = Vec::new();
    match hit_geometry(&r, &SemanticGeometry::Mesh(absent), &transform, &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert!(hit.sub_element.is_none());
            assert_eq!(hit.sub_element_reason, Some(REASON_FACE_SOURCES_ABSENT));
        }
        other => panic!("expected a hit, got {other:?}"),
    }

    // The hit triangle's entry is explicitly `None`.
    let mut missing = two_face_mesh();
    missing.face_sources = vec![None, Some(face("face-b"))];
    match hit_geometry(&r, &SemanticGeometry::Mesh(missing), &transform, &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert!(hit.sub_element.is_none());
            assert_eq!(hit.sub_element_reason, Some(REASON_FACE_SOURCE_MISSING));
        }
        other => panic!("expected a hit, got {other:?}"),
    }

    // A short `face_sources` list puts the hit triangle out of range.
    let short_right = ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let mut short = two_face_mesh();
    short.face_sources = vec![Some(face("face-a"))];
    match hit_geometry(
        &short_right,
        &SemanticGeometry::Mesh(short),
        &transform,
        &options,
    )
    .unwrap()
    {
        PickOutcome::Hit(hit) => {
            assert!(hit.sub_element.is_none());
            assert_eq!(
                hit.sub_element_reason,
                Some(REASON_FACE_SOURCE_OUT_OF_RANGE)
            );
        }
        other => panic!("expected a hit, got {other:?}"),
    }
}

#[test]
fn pick_closest_puts_the_resolved_sub_element_on_the_hit_source() {
    let item = PickItem {
        source: source(7, Vec::new()),
        geometry: SemanticGeometry::Mesh(two_face_mesh()),
        transform: Transform3::identity(),
        geometry_source: GeometrySource::DirectMesh,
    };
    let r = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let report = pick_closest(&r, &[item], &PickOptions::new(1e-6).unwrap()).unwrap();
    let hit = report.hit.expect("a hit");
    assert_eq!(hit.source.entity, EntityId(7));
    assert_eq!(
        hit.source
            .sub_element
            .as_ref()
            .map(|s| s.source_key.as_str()),
        Some("face-a")
    );
    assert!(hit.sub_element_reason.is_none());
}

#[test]
fn two_faces_of_one_mesh_are_distinct_identities() {
    let options = PickOptions::new(1e-6).unwrap();
    let mesh = two_face_mesh();
    let left = PickItem {
        source: source(7, Vec::new()),
        geometry: SemanticGeometry::Mesh(mesh.clone()),
        transform: Transform3::identity(),
        geometry_source: GeometrySource::DirectMesh,
    };
    let right = PickItem {
        source: source(7, Vec::new()),
        geometry: SemanticGeometry::Mesh(mesh),
        transform: Transform3::identity(),
        geometry_source: GeometrySource::DirectMesh,
    };
    let a = pick_closest(
        &ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0)),
        std::slice::from_ref(&left),
        &options,
    )
    .unwrap()
    .hit
    .unwrap();
    let b = pick_closest(
        &ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0)),
        std::slice::from_ref(&right),
        &options,
    )
    .unwrap()
    .hit
    .unwrap();
    assert_ne!(
        a.source.sub_element, b.source.sub_element,
        "two faces of one mesh select differently"
    );
    assert_ne!(a.source, b.source);
}

#[test]
fn line_hits_have_no_sub_element_and_no_reason() {
    let items = vec![item(
        1,
        SemanticGeometry::Line {
            start: p(5.0, -1.0, 0.0),
            end: p(5.0, 1.0, 0.0),
        },
    )];
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let hit = pick_closest(&r, &items, &PickOptions::new(1e-6).unwrap())
        .unwrap()
        .hit
        .unwrap();
    assert!(hit.source.sub_element.is_none());
    assert!(
        hit.sub_element_reason.is_none(),
        "lines are not face-addressable"
    );
}

#[test]
fn circle_is_tessellated_and_marked_approximate() {
    let circle = SemanticGeometry::Circle {
        center: p(10.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        radius: 2.0,
    };
    let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
    let options = PickOptions::new(0.05).unwrap();
    match hit_geometry(&r, &circle, &Transform3::identity(), &options).unwrap() {
        PickOutcome::Hit(hit) => {
            assert!(matches!(hit.precision, Precision::Approximate { .. }));
            assert!((hit.point.x - 8.0).abs() < 0.1, "near side of the circle");
        }
        other => panic!("expected a hit, got {other:?}"),
    }
}
