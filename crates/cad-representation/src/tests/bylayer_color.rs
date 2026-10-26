//! `ByLayer` colour resolves from the layer table at draw time.
//!
//! The importer keeps the symbolic `EntityColor::ByLayer` provenance instead of
//! baking the layer's RGB onto the entity, so the representation resolves it
//! here. These tests pin the resolution rule, the documented fallback for a
//! missing attribute row, and the property that makes the change worthwhile: a
//! layer colour edit recolours its entities on the next expansion with **no**
//! entity write.

use super::*;
use cad_db::{LayerAttributes, LayerColor};

/// Attributes for a layer with this colour identity.
///
/// `aci` and `rgb` are both set explicitly, mirroring what the importer stores:
/// an ACI source keeps its index *and* its resolved sRGB value, a true-colour
/// source has `aci: None` and only an RGB value.
fn layer_attributes(aci: Option<u8>, rgb: [u8; 3]) -> LayerAttributes {
    LayerAttributes {
        color: LayerColor { aci, rgb },
        ..LayerAttributes::default()
    }
}

/// Insert a layer row plus its stored attributes.
fn add_layer(
    builder: &mut DrawingDatabaseBuilder,
    id: u128,
    name: &str,
    attributes: LayerAttributes,
) -> LayerId {
    builder
        .insert_layer(Layer {
            id: LayerId(id),
            name: name.into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(LayerId(id), attributes)
        .unwrap();
    LayerId(id)
}

/// A model-space line on `layer`.
fn line_on_layer(id: u128, layer: LayerId) -> DbEntity {
    let mut entity = line_entity(id, SpaceId::Model, p(0.0, 0.0), p(1.0, 0.0));
    entity.layer = layer;
    entity
}

/// Store this entity's import attributes (explicit opacity, given colour).
fn set_color(builder: &mut DrawingDatabaseBuilder, id: u128, color: cad_db::EntityColor) {
    builder
        .set_entity_render_attributes(
            EntityId(id),
            attrs_styled(color, cad_db::EntityLineWeight::ByLayer),
        )
        .unwrap();
}

fn expand(db: &DrawingDatabase, id: u128) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(id)).unwrap(), &context())
        .unwrap()
}

/// Assert one fragment's colour is this exact sRGB triple.
fn assert_color(fragment: &DisplayFragment, expected: [u8; 3]) {
    for (channel, byte) in expected.iter().enumerate() {
        let want = *byte as f32 / 255.0;
        assert!(
            (fragment.color[channel] - want).abs() < 1e-6,
            "channel {channel}: got {} want {want} (expected sRGB {expected:?}, got {:?})",
            fragment.color[channel],
            fragment.color,
        );
    }
}

#[test]
fn by_layer_resolves_to_the_owning_layers_colour() {
    let mut b = empty_db();
    // ACI-sourced layer: the importer stores the index *and* the ACI's sRGB.
    let walls = add_layer(&mut b, 1, "WALLS", layer_attributes(Some(1), [255, 0, 0]));
    // True-colour layer: no ACI identity, only the source RGB.
    let detail = add_layer(&mut b, 2, "DETAIL", layer_attributes(None, [10, 20, 30]));

    b.insert_entity(line_on_layer(10, walls)).unwrap();
    set_color(&mut b, 10, cad_db::EntityColor::ByLayer);
    b.insert_entity(line_on_layer(11, detail)).unwrap();
    set_color(&mut b, 11, cad_db::EntityColor::ByLayer);
    let db = b.finish().unwrap();

    let walls_fragment = &expand(&db, 10).fragments[0];
    assert_color(walls_fragment, [255, 0, 0]);
    assert!(
        !walls_fragment.color_unresolved,
        "a ByLayer colour read from the layer table is a resolved value"
    );

    let detail_fragment = &expand(&db, 11).fragments[0];
    assert_color(detail_fragment, [10, 20, 30]);
    assert!(!detail_fragment.color_unresolved);
}

#[test]
fn by_layer_without_an_attribute_row_uses_the_documented_default() {
    let mut b = empty_db();
    // A row whose attribute set is the documented default.
    let bare = add_layer(&mut b, 3, "BARE", LayerAttributes::default());
    b.insert_entity(line_on_layer(12, bare)).unwrap();
    set_color(&mut b, 12, cad_db::EntityColor::ByLayer);

    // And a row inserted with no attribute set at all.
    b.insert_layer(Layer {
        id: LayerId(4),
        name: "NO-ROW".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_on_layer(13, LayerId(4))).unwrap();
    set_color(&mut b, 13, cad_db::EntityColor::ByLayer);
    let db = b.finish().unwrap();

    // `DrawingDatabase::layer_attributes` documents this fallback; the
    // expectation is read from it rather than re-typed here.
    let documented_default = LayerAttributes::default().color.rgb;
    assert_color(&expand(&db, 12).fragments[0], documented_default);
    assert_color(&expand(&db, 13).fragments[0], documented_default);
    assert!(
        !expand(&db, 13).fragments[0].color_unresolved,
        "the documented default is a resolved value, not an unresolved one"
    );
}

#[test]
fn explicit_colour_is_untouched_by_the_layer_table() {
    let mut b = empty_db();
    let red = add_layer(&mut b, 1, "RED", layer_attributes(Some(1), [255, 0, 0]));
    b.insert_entity(line_on_layer(20, red)).unwrap();
    set_color(&mut b, 20, cad_db::EntityColor::Explicit([1, 2, 3]));
    let db = b.finish().unwrap();

    let fragment = &expand(&db, 20).fragments[0];
    // Byte-identical: no rounding to an ACI palette entry, no layer lookup.
    assert_color(fragment, [1, 2, 3]);
    assert!(!fragment.color_unresolved);
}

#[test]
fn block_content_on_layer_zero_inherits_the_insert_layer() {
    // AutoCAD's rule: a block member that is **itself** on layer `0` is drawn on
    // the containing INSERT's layer, so its `ByLayer` colour is the INSERT's
    // layer colour. A member on any other layer keeps its own layer (the control
    // below). `ByBlock` is a different rule and still inherits the INSERT's
    // resolved colour — which must not be applied twice.
    let mut b = empty_db();
    // `empty_db` gives layer "0" the id `LayerId(0)`; RED and BLUE are separate
    // rows, so "layer 0" here is resolved by name like any other drawing.
    let zero = LayerId(0);
    let red = add_layer(&mut b, 1, "RED", layer_attributes(Some(1), [255, 0, 0]));
    let blue = add_layer(&mut b, 2, "BLUE", layer_attributes(Some(5), [0, 0, 255]));

    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(30), EntityId(31), EntityId(32)],
        dynamic_visibility: None,
    })
    .unwrap();
    // Block child on layer 0 with ByLayer: resolves to the INSERT's layer.
    b.insert_entity(line_on_layer(30, zero)).unwrap();
    set_color(&mut b, 30, cad_db::EntityColor::ByLayer);
    // Block child on layer 0 with ByBlock: inherits the INSERT's resolved colour.
    b.insert_entity(line_on_layer(31, zero)).unwrap();
    set_color(&mut b, 31, cad_db::EntityColor::ByBlock);
    // Control: a child on RED with ByLayer keeps RED under the new rule too.
    b.insert_entity(line_on_layer(32, red)).unwrap();
    set_color(&mut b, 32, cad_db::EntityColor::ByLayer);

    // The INSERT itself sits on BLUE and is ByLayer.
    let mut insert = insert_entity(2, SpaceId::Model, 2, 10.0);
    insert.layer = blue;
    b.insert_entity(insert).unwrap();
    set_color(&mut b, 2, cad_db::EntityColor::ByLayer);
    let db = b.finish().unwrap();

    let rep = expand(&db, 2);
    assert_eq!(
        rep.fragments.len(),
        3,
        "all three block children are expanded"
    );
    assert_color(&rep.fragments[0], [0, 0, 255]);
    assert!(
        !rep.fragments[0].color_unresolved,
        "the layer-0 child's ByLayer resolved through the INSERT's layer"
    );
    assert_color(&rep.fragments[1], [0, 0, 255]);
    assert!(
        !rep.fragments[1].color_unresolved,
        "ByBlock inherited the INSERT's ByLayer colour, resolved from BLUE"
    );
    assert_color(&rep.fragments[2], [255, 0, 0]);
    assert!(
        !rep.fragments[2].color_unresolved,
        "a member on a non-zero layer keeps its own layer"
    );
    // The stored fact survives: the member is still on layer 0.
    assert_eq!(
        db.entity(EntityId(30)).unwrap().layer,
        zero,
        "display resolution never rewrites DbEntity::layer"
    );
}

#[test]
fn editing_a_layer_colour_recolours_by_layer_entities_with_no_entity_write() {
    let mut b = empty_db();
    let walls = add_layer(&mut b, 1, "WALLS", layer_attributes(Some(1), [255, 0, 0]));
    b.insert_entity(line_on_layer(40, walls)).unwrap();
    set_color(&mut b, 40, cad_db::EntityColor::ByLayer);
    let mut db = b.finish().unwrap();

    assert_color(&expand(&db, 40).fragments[0], [255, 0, 0]);

    let entity_before = db.entity(EntityId(40)).unwrap().clone();
    let revision_before = db.revision();

    // The same post-import write `SETLAYERPROPERTY color` performs.
    let change_set = db
        .set_layer_attributes(
            walls,
            layer_attributes(Some(3), [0, 255, 0]),
            TransactionId(1),
            "SETLAYERPROPERTY color",
        )
        .expect("a layer colour edit is allowed");

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
    assert_eq!(
        db.revision(),
        Revision(revision_before.0 + 1),
        "only the drawing revision moves, once"
    );

    // Same stored attributes, new colour: no re-import, no entity update.
    assert_color(&expand(&db, 40).fragments[0], [0, 255, 0]);
}
