//! Whole-query behaviour of `pick_closest` / `pick_closest_borrowed`.

use cad_domain::*;

use super::super::*;
use super::common::*;

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
