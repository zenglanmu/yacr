//! Unit tests.

use super::*;

fn entry(id: u128, x: f64, y: f64) -> SpatialEntry {
    SpatialEntry {
        source: SelectionRef {
            document: DocumentId(1),
            entity: EntityId(id),
            instance: InstancePath::default(),
            sub_element: None,
        },
        bounds: Bounds3 {
            min: Point3 {
                x: x - 1.0,
                y: y - 1.0,
                z: -1.0,
            },
            max: Point3 {
                x: x + 1.0,
                y: y + 1.0,
                z: 1.0,
            },
        },
    }
}

#[test]
fn query_returns_only_intersecting_entries() {
    let mut index = GridSpatialIndex::new();
    index
        .rebuild(&[entry(1, 0.0, 0.0), entry(2, 100.0, 100.0)])
        .unwrap();
    let hits = index
        .query_bounds(&Bounds3 {
            min: Point3 {
                x: -5.0,
                y: -5.0,
                z: -5.0,
            },
            max: Point3 {
                x: 5.0,
                y: 5.0,
                z: 5.0,
            },
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity, EntityId(1));
}

#[test]
fn ray_candidates_use_aabb_slab_test() {
    let mut index = GridSpatialIndex::new();
    index
        .rebuild(&[entry(1, 0.0, 0.0), entry(2, 50.0, 0.0)])
        .unwrap();
    let ray = Ray3 {
        origin: Point3 {
            x: -10.0,
            y: 0.0,
            z: 0.0,
        },
        direction: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    };
    let hits = index.ray_candidates(&ray).unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn update_removes_by_source() {
    let mut index = GridSpatialIndex::new();
    let e = entry(1, 0.0, 0.0);
    index.rebuild(std::slice::from_ref(&e)).unwrap();
    index.update(&[], std::slice::from_ref(&e.source)).unwrap();
    assert_eq!(index.entry_count(), 0);
}
