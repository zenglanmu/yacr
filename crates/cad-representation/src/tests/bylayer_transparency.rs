//! `ByLayer` transparency resolves from the layer table at draw time.
//!
//! The importer keeps the symbolic `EntityTransparency::ByLayer` instead of
//! baking the layer's opacity onto the entity, so the representation resolves it
//! here — the same rule colour, linetype and lineweight already follow. These
//! tests pin the resolution rule (the owning layer's `opacity`), the documented
//! default for a missing layer row, the byte-identical explicit path, the
//! `ByBlock` chain and its model-root fallback, the block-content rule, and the
//! property that makes the change worthwhile: a layer opacity edit changes its
//! `ByLayer` entities on the next expansion with **no** entity write.

use super::*;
use cad_db::LayerAttributes;

/// Insert a layer row plus its stored attributes (opacity here).
fn add_layer(builder: &mut DrawingDatabaseBuilder, id: u128, name: &str, opacity: f32) -> LayerId {
    builder
        .insert_layer(Layer {
            id: LayerId(id),
            name: name.into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(id),
            LayerAttributes {
                opacity,
                ..LayerAttributes::default()
            },
        )
        .unwrap();
    LayerId(id)
}

/// A model-space line on `layer`.
fn line_on_layer(id: u128, layer: LayerId) -> DbEntity {
    let mut entity = line_entity(id, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    entity.layer = layer;
    entity
}

/// Store this entity's transparency provenance, leaving the other channels
/// symbolic so they cannot influence the assertions.
fn set_transparency(
    builder: &mut DrawingDatabaseBuilder,
    id: u128,
    transparency: EntityTransparency,
) {
    builder
        .set_entity_render_attributes(
            EntityId(id),
            EntityRenderAttributes {
                transparency,
                ..Default::default()
            },
        )
        .unwrap();
}

fn expand(db: &DrawingDatabase, id: u128) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(id)).unwrap(), &context())
        .unwrap()
}

#[test]
fn by_layer_transparency_resolves_from_the_owning_layer_row() {
    let mut b = empty_db();
    let faded = add_layer(&mut b, 1, "FADED", 0.4);
    let opaque = add_layer(&mut b, 2, "OPAQUE", 1.0);
    b.insert_entity(line_on_layer(10, faded)).unwrap();
    set_transparency(&mut b, 10, EntityTransparency::ByLayer);
    b.insert_entity(line_on_layer(11, opaque)).unwrap();
    set_transparency(&mut b, 11, EntityTransparency::ByLayer);
    let db = b.finish().unwrap();

    assert_eq!(expand(&db, 10).fragments[0].alpha, 0.4);
    assert_eq!(expand(&db, 11).fragments[0].alpha, 1.0);
}

#[test]
fn a_missing_layer_attribute_row_uses_the_documented_default() {
    // `layer_attributes` documents `LayerAttributes::default()` for a row that
    // was never stored, and that default is fully opaque. The fallback therefore
    // comes from the existing default, not from a second one invented here.
    assert_eq!(LayerAttributes::default().opacity, 1.0);

    let mut b = empty_db();
    b.insert_layer(Layer {
        id: LayerId(4),
        name: "NO-ROW".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_on_layer(12, LayerId(4))).unwrap();
    set_transparency(&mut b, 12, EntityTransparency::ByLayer);
    let db = b.finish().unwrap();

    assert_eq!(expand(&db, 12).fragments[0].alpha, 1.0);
}

#[test]
fn explicit_transparency_is_untouched_by_the_layer_table() {
    let mut b = empty_db();
    let faded = add_layer(&mut b, 1, "FADED", 0.4);
    b.insert_entity(line_on_layer(13, faded)).unwrap();
    set_transparency(&mut b, 13, EntityTransparency::Explicit(0.25));
    let db = b.finish().unwrap();

    // The stored opacity reaches the fragment unchanged: the layer's 0.4 never
    // replaces an explicit entity value.
    assert_eq!(expand(&db, 13).fragments[0].alpha, 0.25);
}

#[test]
fn explicit_and_by_layer_values_share_the_one_sanitising_point() {
    // Neither arm clamps or sanitises in the representation: an unreadable value
    // is passed through exactly as stored on both paths and
    // `cad_scene::sanitize_alpha` is the single place that turns it opaque. The
    // two arms must therefore agree bit-for-bit.
    let mut b = empty_db();
    let unreadable = add_layer(&mut b, 1, "NAN-OPACITY", f32::NAN);
    b.insert_entity(line_on_layer(14, unreadable)).unwrap();
    set_transparency(&mut b, 14, EntityTransparency::ByLayer);
    b.insert_entity(line_on_layer(15, unreadable)).unwrap();
    set_transparency(&mut b, 15, EntityTransparency::Explicit(f32::NAN));
    let db = b.finish().unwrap();

    assert!(expand(&db, 14).fragments[0].alpha.is_nan());
    assert!(expand(&db, 15).fragments[0].alpha.is_nan());
}

#[test]
fn by_block_inherits_the_insert_and_falls_back_to_opaque_at_the_root() {
    let mut b = empty_db();
    let insert_layer = add_layer(&mut b, 1, "INSERT", 0.25);
    let child_layer = add_layer(&mut b, 2, "CHILD", 0.75);

    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(21)],
        dynamic_visibility: None,
    })
    .unwrap();
    // The block child is ByBlock: it takes the INSERT's resolved opacity (0.25),
    // not its own layer's 0.75.
    b.insert_entity(line_on_layer(21, child_layer)).unwrap();
    set_transparency(&mut b, 21, EntityTransparency::ByBlock);

    let mut insert = insert_entity(2, SpaceId::Model, 2, 10.0);
    insert.layer = insert_layer;
    b.insert_entity(insert).unwrap();
    set_transparency(&mut b, 2, EntityTransparency::ByLayer);

    // A ByBlock entity with no enclosing INSERT sits at the model root.
    b.insert_entity(line_on_layer(20, child_layer)).unwrap();
    set_transparency(&mut b, 20, EntityTransparency::ByBlock);
    let db = b.finish().unwrap();

    let root = &expand(&db, 20).fragments[0];
    assert_eq!(
        root.alpha, 1.0,
        "ByBlock at the model root has nothing to inherit, so it is opaque"
    );

    let expanded = expand(&db, 2);
    assert_eq!(expanded.fragments.len(), 1, "the block child is expanded");
    assert_eq!(
        expanded.fragments[0].alpha, 0.25,
        "ByBlock inherits the INSERT's resolved opacity"
    );
}

#[test]
fn block_content_resolves_by_layer_against_its_own_layer() {
    // The same rule as colour/lineweight/linetype: every entity resolves
    // `ByLayer` against the layer it is **itself** on, including entities inside
    // a block definition. Only `ByBlock` walks up the parent chain.
    let mut b = empty_db();
    let child_layer = add_layer(&mut b, 1, "CHILD", 0.25);
    let insert_layer = add_layer(&mut b, 2, "INSERT", 0.75);

    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(30), EntityId(31)],
        dynamic_visibility: None,
    })
    .unwrap();
    // First child: ByLayer on CHILD, so it uses CHILD's opacity.
    b.insert_entity(line_on_layer(30, child_layer)).unwrap();
    set_transparency(&mut b, 30, EntityTransparency::ByLayer);
    // Second child: ByBlock, so it inherits the INSERT (0.75).
    b.insert_entity(line_on_layer(31, child_layer)).unwrap();
    set_transparency(&mut b, 31, EntityTransparency::ByBlock);

    let mut insert = insert_entity(2, SpaceId::Model, 2, 10.0);
    insert.layer = insert_layer;
    b.insert_entity(insert).unwrap();
    set_transparency(&mut b, 2, EntityTransparency::ByLayer);
    let db = b.finish().unwrap();

    let expanded = expand(&db, 2);
    assert_eq!(expanded.fragments.len(), 2, "both block children expand");
    assert_eq!(
        expanded.fragments[0].alpha, 0.25,
        "a block child's ByLayer resolves against its own layer, not the INSERT's"
    );
    assert_eq!(
        expanded.fragments[1].alpha, 0.75,
        "ByBlock inherits the INSERT's already-resolved value"
    );
}

#[test]
fn editing_a_layer_opacity_changes_the_fragment_with_no_entity_write() {
    let mut b = empty_db();
    let walls = add_layer(&mut b, 1, "WALLS", 0.4);
    b.insert_entity(line_on_layer(40, walls)).unwrap();
    set_transparency(&mut b, 40, EntityTransparency::ByLayer);
    let mut db = b.finish().unwrap();

    assert_eq!(expand(&db, 40).fragments[0].alpha, 0.4);

    let entity_before = db.entity(EntityId(40)).unwrap().clone();
    let revision_before = db.revision();

    // The same post-import write `SETLAYERPROPERTY transparency` performs.
    let change_set = db
        .set_layer_attributes(
            walls,
            LayerAttributes {
                opacity: 0.2,
                ..LayerAttributes::default()
            },
            TransactionId(1),
            "SETLAYERPROPERTY transparency",
        )
        .expect("a layer opacity edit is allowed");
    assert!(
        change_set.changes.is_empty(),
        "a layer edit publishes no object change: {:?}",
        change_set.changes
    );
    assert_eq!(
        db.entity(EntityId(40)).unwrap(),
        &entity_before,
        "the entity row (including its revision) is untouched"
    );
    assert_eq!(db.revision(), Revision(revision_before.0 + 1));

    // Same stored attributes on the entity, new layer opacity: no re-import, no
    // entity update, and the fragment now carries the layer's new value.
    let after = expand(&db, 40);
    assert_eq!(after.fragments[0].alpha, 0.2);
    assert_eq!(
        db.entity_render_attributes(EntityId(40)).transparency,
        EntityTransparency::ByLayer,
        "the entity keeps the symbol; only the layer row moved"
    );
}
