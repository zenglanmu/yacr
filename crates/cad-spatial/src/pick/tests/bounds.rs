//! Local pick-bounds computation, conservative culling, and cache reuse.

use cad_domain::*;

use super::super::bounds::{cached_local_pick_bounds, local_pick_bounds, PickBoundsCache};
use super::super::math::{is_finite_point, length3};
use super::super::*;
use super::common::*;

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
fn hatch_local_pick_bounds_are_the_boundary_vertices() {
    // ADR 0018 A.2/E: the hatch keeps its precise local bound rather than
    // degrading to the item AABB (the previous wildcard returned `None`).
    let hatch = hatch_semantics();
    let (min, max) = local_pick_bounds(&hatch).expect("precise hatch bounds");
    assert!((min.x).abs() < 1e-9 && (min.y).abs() < 1e-9, "{min:?}");
    assert!((max.x - 10.0).abs() < 1e-9, "{max:?}");
    assert!((max.y - 10.0).abs() < 1e-9, "{max:?}");
}

#[test]
fn hatch_cached_pick_bounds_match_the_precise_leaf_bound() {
    let hatch = hatch_semantics();
    let precise = local_pick_bounds(&hatch).expect("precise hatch bounds");
    let mut cache = PickBoundsCache::new();
    let cached = cached_local_pick_bounds(&hatch, &mut cache).expect("cached hatch bounds");
    assert_eq!(cached, precise);
}

#[test]
fn hatch_cached_pick_bounds_are_reused_like_a_compound() {
    // Cache parity with compound/dimension (P6a-1 follow-up): a hatch is cached
    // on its own key, so a shared placement computes its boundary bound once.
    let hatch = hatch_semantics();
    let mut cache = PickBoundsCache::new();
    PICK_BOUNDS_COMPUTED.with(|count| count.set(0));
    let first = cached_local_pick_bounds(&hatch, &mut cache).expect("cached hatch bounds");
    let second = cached_local_pick_bounds(&hatch, &mut cache).expect("cached hatch bounds");
    assert_eq!(first, second);
    assert_eq!(cache.len(), 1, "the hatch is cached on its own key");
    PICK_BOUNDS_COMPUTED
        .with(|count| assert_eq!(count.get(), 1, "a repeated hatch reuses its cached bound"));
}
