//! `WIPEOUTFRAME` display contract (split out of `tests.rs` for the file-size
//! ceiling): the session token is part of `SceneVersion`, so a flip rebuilds the
//! base and toggles the mask boundary frame without touching `cad-db`.

use super::*;
use cad_db::DrawingDatabaseBuilder;
use cad_domain::DatabaseId;

/// `WIPEOUTFRAME` is session state, but it changes what the base draws: the
/// controller's flag is part of `SceneVersion`, so flipping it rebuilds the base
/// (adding/removing the mask boundary frame) without any `cad-db` write.
fn prepare(controller: &mut CadSceneController, db: &DrawingDatabase) {
    controller
        .prepare(
            Some(db),
            DocumentId(73),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
        )
        .unwrap();
}

#[test]
fn wipeout_frames_flag_rebuilds_the_base_and_toggles_the_mask_frame() {
    use cad_db::{DbEntity, DbObject, Layer};
    use cad_domain::{EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId};

    let p = |x: f64, y: f64| Point3 { x, y, z: 0.0 };
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(11));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbWipeout".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mask {
            boundary: vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0)],
            inverted: false,
        },
        draw_order: 0,
    })
    .unwrap();
    let db = b.finish().unwrap();

    let mut controller = CadSceneController::default();
    assert!(controller.set_wipeout_frames(true));
    assert!(
        !controller.set_wipeout_frames(true),
        "an unchanged flag is a no-op"
    );
    prepare(&mut controller, &db);
    let first = controller.ready.as_ref().unwrap();
    assert_eq!(first.base.masks().len(), 1);
    assert_eq!(
        first.base.masks()[0].frame,
        Some(cad_representation::DEFAULT_RENDER_COLOR),
        "ON draws the mask boundary frame"
    );
    let base_revision = first.base_revision;

    assert!(controller.set_wipeout_frames(false));
    prepare(&mut controller, &db);
    let second = controller.ready.as_ref().unwrap();
    assert!(
        second.base_revision > base_revision,
        "a WIPEOUTFRAME flip must rebuild the base"
    );
    assert_eq!(
        second.base.masks()[0].frame,
        None,
        "OFF leaves the fill and drops the frame"
    );
}
