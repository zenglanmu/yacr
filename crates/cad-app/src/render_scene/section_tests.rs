//! A document section plane really clips the assembled model batches.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::section::SectionPlane;
use cad_domain::{DocumentId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId, TaskStamp};

fn document(plane: Option<SectionPlane>) -> cad_db::DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: cad_domain::LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: cad_domain::EntityId(1),
            layer: cad_domain::LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: -1.0,
                },
                end: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    builder.set_section_plane(plane).unwrap();
    builder.finish().unwrap()
}

fn max_world_z(delta: &cad_scene::SceneDelta) -> f64 {
    delta
        .added
        .iter()
        .flat_map(|batch| {
            batch
                .vertices
                .iter()
                .map(move |v| batch.local_origin.z + f64::from(v[2]))
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

fn z_plane(active: bool) -> SectionPlane {
    SectionPlane::new(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    )
    .unwrap()
    .with_active(active)
}

#[test]
fn an_active_section_plane_clips_the_model_batches() {
    let delta = build_scene(
        &document(Some(z_plane(true))),
        TaskStamp::new(DocumentId(1), 0),
    )
    .expect("scene");
    assert!(!delta.added.is_empty());
    assert!(
        max_world_z(&delta) <= 1.0e-5,
        "the above-plane half must be clipped, got {}",
        max_world_z(&delta)
    );
}

#[test]
fn a_paused_or_absent_plane_leaves_the_model_intact() {
    for plane in [None, Some(z_plane(false))] {
        let delta = build_scene(&document(plane), TaskStamp::new(DocumentId(1), 0)).expect("scene");
        assert!(
            (max_world_z(&delta) - 1.0).abs() < 1.0e-5,
            "an inactive/absent plane must not clip, got {}",
            max_world_z(&delta)
        );
    }
}
