//! Block-attribute value edit invalidation (ADR-0055 M2).
//!
//! An `INSERT`-owned attribute value is part of the reference's display, so a
//! value edit is a geometry update of the `INSERT`. A change that adds a drawn
//! fragment changes the batch count and must fall back to a full rebuild: the
//! overwrite path only replaces the same number of resident batches.

use super::*;
use crate::layers::LayerOverrideSet;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    AttributeAppearance, AttributeDefinition, AttributeFlags, AttributeText, AttributeValue,
    BlockId, DatabaseId, DocumentId, EntityId, LayerId, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, StyleId, TransactionId, Transform3,
};
use cad_representation::{FontEngine, SpaceSelection};
use std::sync::Arc;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// An `INSERT`-owned attribute value is part of the reference's display, so a
/// value edit is a geometry update of the `INSERT`. A change that adds a drawn
/// fragment changes the batch count and must fall back to a full rebuild: the
/// overwrite path only replaces the same number of resident batches.
#[test]
fn an_attribute_value_edit_that_changes_the_fragment_count_full_rebuilds() {
    fn attribute_text(y: f64) -> AttributeText {
        AttributeText {
            position: p(1.0, y),
            height: 1.0,
            style: StyleId(0),
            font: Some("osifont.ttf".into()),
            ..AttributeText::default()
        }
    }

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(7),
            entities: vec![EntityId(71)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(71),
                type_key: "AcDbAttributeDefinition".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(71),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(7)),
            geometry: SemanticGeometry::AttributeDefinition(Box::new(AttributeDefinition {
                tag: "A".into(),
                prompt: "A?".into(),
                default_value: "A0".into(),
                flags: AttributeFlags::default(),
                order: 0,
                text: attribute_text(0.0),
                source_handle: None,
            })),
            draw_order: 0,
        })
        .unwrap();
    let value = |tag: &str, text: &str, y: f64, invisible: bool| AttributeValue {
        tag: tag.into(),
        value: text.into(),
        flags: AttributeFlags {
            invisible,
            ..AttributeFlags::default()
        },
        text: attribute_text(y),
        layer: LayerId(0),
        appearance: AttributeAppearance::default(),
        source_handle: None,
    };
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: BlockId(7),
                transform: Transform3::translation(p(10.0, 0.0)),
                attributes: vec![value("A", "one", 0.0, false), value("B", "two", 2.0, true)],
            },
            draw_order: 0,
        })
        .unwrap();
    let before = Arc::new(builder.finish().unwrap());

    let mut engine = FontEngine::new();
    engine
        .register(
            "osifont.ttf",
            Arc::from(include_bytes!("../../../../fonts/osifont.ttf").as_slice()),
        )
        .unwrap();
    let fonts = Some(Arc::new(engine));
    let mut controller = CadSceneController::default();
    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(before.clone()),
            DocumentId(1),
            fonts.clone(),
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            None,
        )
        .unwrap();
    let (base, base_revision, range) = {
        let ready = controller.ready.as_ref().unwrap();
        (
            ready.base.clone(),
            ready.base_revision,
            controller
                .entity_batch_range(EntityId(1))
                .expect("the visible attribute text owns resident batches"),
        )
    };
    assert!(
        !range.is_empty(),
        "the base really draws the visible attribute of the reference"
    );

    // Reveal the invisible second value: the reference now emits one more text
    // fragment, which cannot overwrite the same resident batch range.
    let mut after = before.as_ref().clone();
    let mut edited = after.entity(EntityId(1)).unwrap().clone();
    if let SemanticGeometry::Insert { attributes, .. } = &mut edited.geometry {
        attributes[1].flags.invisible = false;
    }
    let mut transaction = after
        .begin_drawing_transaction("test: attribute value", TransactionId(11))
        .unwrap();
    transaction.update_entity(edited).unwrap();
    let changes = transaction.commit().unwrap();
    let after = Arc::new(after);
    let receipt = DrawingEditReceipt::from_commit(before.clone(), after.clone(), &changes)
        .expect("a single INSERT geometry update is classified");
    assert_eq!(
        receipt.delta,
        CommitDelta::Update(EntityId(1)),
        "the value lives on the INSERT, so the edit is one entity update"
    );

    controller
        .prepare_shared_with_overlays_and_images_and_edit(
            Some(after.clone()),
            DocumentId(1),
            fonts,
            &LayerOverrideSet::new(),
            SpaceSelection::Model,
            &OverlayInputs::default(),
            None,
            Some(&receipt),
        )
        .unwrap();
    let ready = controller.ready.as_ref().unwrap();
    assert!(
        !ready.base.same_as(&base),
        "a fragment-count change must full-rebuild, not overwrite a partial range"
    );
    assert!(
        ready.overwrites.is_empty(),
        "a full rebuild never publishes a partial in-place overwrite"
    );
    assert_eq!(
        ready.base_revision,
        base_revision + 1,
        "the full rebuild advances the base revision exactly once"
    );
    let rebuilt = controller
        .entity_batch_range(EntityId(1))
        .expect("the rebuilt index records the reference again");
    assert!(
        rebuilt.len() > range.len(),
        "the now-visible second value adds resident batches ({rebuilt:?} vs {range:?})"
    );
}
