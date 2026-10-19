//! Stable sub-element (mesh face) resolution on hits.

use cad_domain::*;

use super::super::*;
use super::common::*;

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
