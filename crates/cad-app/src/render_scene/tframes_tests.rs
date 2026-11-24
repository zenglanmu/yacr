//! `TFRAMES` display contract (split out of `tests.rs` for the file-size
//! ceiling): the session token is part of `SceneVersion`, so a flip rebuilds the
//! base and toggles the image/wipeout frames without touching `cad-db`.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};
use cad_domain::{
    DatabaseId, EntityId, ImageAdjust, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId,
};
use cad_resources::{image_resource_key, DecodedImage, DecodedImageCache, ResourceLimits};
use std::sync::Arc;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// One textured raster image plus a WIPEOUT mask, so both frames this build
/// models are present in one scene.
fn db_with_image_and_mask() -> (DrawingDatabase, Arc<DecodedImageCache>) {
    use cad_db::{DbEntity, DbObject};
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
            type_key: "AcDbRasterImage".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Image {
            origin: p(0.0, 0.0),
            u: p(1.0, 0.0),
            v: p(0.0, 1.0),
            pixels: [2.0, 2.0],
            file: Some("logo.png".into()),
            clip: None,
            visible: true,
            adjust: ImageAdjust::default(),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(2),
            type_key: "AcDbWipeout".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(2),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Mask {
            boundary: vec![p(4.0, 0.0), p(8.0, 0.0), p(8.0, 4.0)],
            inverted: false,
        },
        draw_order: 1,
    })
    .unwrap();
    let mut cache = DecodedImageCache::new();
    let key = image_resource_key("logo.png").expect("a usable image path");
    cache
        .insert(
            key,
            DecodedImage {
                width: 1,
                height: 1,
                rgba: Arc::from(vec![255u8, 255, 255, 255]),
            },
            &ResourceLimits::default(),
        )
        .unwrap();
    (b.finish().unwrap(), Arc::new(cache))
}

fn prepare(
    controller: &mut CadSceneController,
    db: &Arc<DrawingDatabase>,
    cache: &Arc<DecodedImageCache>,
) {
    controller
        .prepare_shared_with_overlays_and_images(
            Some(db.clone()),
            DocumentId(73),
            None,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &OverlayInputs::default(),
            Some(cache.clone()),
        )
        .unwrap();
}

/// One `Lines` batch that names the raster image — the frame the provider adds
/// when `frames_visible` is on.
fn image_frame_batches(controller: &CadSceneController) -> usize {
    let Some(scene) = &controller.ready else {
        return 0;
    };
    scene
        .base
        .batches()
        .filter(|batch| {
            batch.topology == cad_scene::RenderTopology::Lines
                && batch
                    .sources
                    .iter()
                    .any(|source| source.entity == EntityId(1))
        })
        .count()
}

#[test]
fn tframes_hides_image_and_wipeout_frames_and_rebuilds_the_base() {
    let (db, cache) = db_with_image_and_mask();
    let db = Arc::new(db);
    let mut controller = CadSceneController::default();
    // The default keeps `TFRAMES` ON (AutoCAD shows frames by default) and
    // `WIPEOUTFRAME` ON in `DisplaySettings`' session mapping.
    controller.set_display_settings(DisplaySettings {
        wipeout_frames: true,
        frames_visible: true,
        ..DisplaySettings::default()
    });
    prepare(&mut controller, &db, &cache);
    let first = controller.ready.as_ref().unwrap();
    assert_eq!(first.base.images().len(), 1, "the texture is drawn");
    assert_eq!(
        image_frame_batches(&controller),
        1,
        "TFRAMES ON adds the image boundary frame"
    );
    assert_eq!(
        first.base.masks()[0].frame,
        Some(cad_representation::DEFAULT_RENDER_COLOR),
        "TFRAMES ON keeps the WIPEOUT frame WIPEOUTFRAME allows"
    );
    let base_revision = first.base_revision;

    // `TFRAMES OFF` hides both frames even though `WIPEOUTFRAME` still says ON.
    assert!(controller.set_display_settings(DisplaySettings {
        wipeout_frames: true,
        frames_visible: false,
        ..DisplaySettings::default()
    }));
    prepare(&mut controller, &db, &cache);
    let second = controller.ready.as_ref().unwrap();
    assert!(
        second.base_revision > base_revision,
        "a TFRAMES flip must rebuild the base"
    );
    assert_eq!(
        image_frame_batches(&controller),
        0,
        "TFRAMES OFF removes the image frame"
    );
    assert_eq!(
        second.base.masks()[0].frame,
        None,
        "TFRAMES OFF also hides the WIPEOUT frame"
    );
    assert_eq!(second.base.images().len(), 1, "the texture still draws");

    // A repeated sync of the same settings is free: the flag lives in
    // `SceneVersion`, so no second rebuild happens.
    let after = second.base_revision;
    prepare(&mut controller, &db, &cache);
    assert_eq!(
        controller.ready.as_ref().unwrap().base_revision,
        after,
        "an unchanged TFRAMES token schedules nothing"
    );
}
