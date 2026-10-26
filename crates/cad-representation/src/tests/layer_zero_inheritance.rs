//! AutoCAD's "a block member on layer `0` follows the INSERT's layer" rule.
//!
//! `docs/compatibility.md` records this rule; the implementation is one shared
//! resolution (`cad_db::LayerScope`, seeded by
//! [`DrawingDatabase::root_layer_scope`]) used by `provider/expand.rs`, so all
//! four `ByLayer` styles inherit **together** — colour, opacity, lineweight and
//! linetype cannot disagree. These tests pin:
//!
//! * all four styles follow the INSERT's layer for a layer-`0` member,
//! * layer `0` is resolved **by name**, never as `LayerId(0)` (the fixtures use
//!   a decoy row on `LayerId(0)` and give layer `0` a different id),
//! * the rule chains through nested blocks, one level per `INSERT`,
//! * a drawing with **no** layer `0` behaves exactly as before (no inheritance,
//!   no panic),
//! * the stored `DbEntity::layer` is never rewritten.

use super::*;
use cad_db::{
    BlockDefinition, DrawingDatabaseBuilder, Layer, LayerAttributes, LayerColor, LayerLineWeight,
    LineType as DbLineType, LinetypePattern,
};

/// Stored attributes for a layer with this style identity.
fn style(
    rgb: [u8; 3],
    linetype: &str,
    lineweight: LayerLineWeight,
    opacity: f32,
) -> LayerAttributes {
    LayerAttributes {
        color: LayerColor { aci: None, rgb },
        linetype: linetype.into(),
        lineweight,
        opacity,
        ..LayerAttributes::default()
    }
}

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

fn add_linetype(builder: &mut DrawingDatabaseBuilder, id: u128, name: &str, elements: &[f64]) {
    builder
        .insert_linetype(DbLineType {
            id: LinetypeId(id),
            name: name.into(),
            pattern: LinetypePattern::from_elements(elements.iter().copied()),
            complex: false,
        })
        .unwrap();
}

/// A line in `space` on `layer`.
fn line_in(id: u128, space: SpaceId, layer: LayerId, a: Point3, b: Point3) -> DbEntity {
    let mut entity = line_entity(id, space, a, b);
    entity.layer = layer;
    entity
}

/// An INSERT in `space` on `layer`.
fn insert_in(id: u128, space: SpaceId, block: u128, layer: LayerId, dx: f64) -> DbEntity {
    let mut entity = insert_entity(id, space, block, dx);
    entity.layer = layer;
    entity
}

/// Store symbolic `ByLayer` provenance for all four styles.
fn set_all_by_layer(builder: &mut DrawingDatabaseBuilder, id: u128) {
    builder
        .set_entity_render_attributes(
            EntityId(id),
            EntityRenderAttributes {
                transparency: EntityTransparency::ByLayer,
                color: cad_db::EntityColor::ByLayer,
                lineweight: EntityLineWeight::ByLayer,
                linetype: EntityLineType::ByLayer,
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

/// Assert a fragment carries one style identity, exactly.
fn assert_style(fragment: &DisplayFragment, rgb: [u8; 3], alpha: f32, lineweight: f32) {
    for (channel, byte) in rgb.iter().enumerate() {
        let want = *byte as f32 / 255.0;
        assert!(
            (fragment.color[channel] - want).abs() < 1e-6,
            "channel {channel}: got {} want {want} (expected {rgb:?}, got {:?})",
            fragment.color[channel],
            fragment.color,
        );
    }
    assert!(
        (fragment.alpha - alpha).abs() < 1e-6,
        "alpha: got {} want {alpha}",
        fragment.alpha
    );
    assert!(
        (fragment.lineweight - lineweight).abs() < 1e-6,
        "lineweight: got {} want {lineweight}",
        fragment.lineweight
    );
    assert!(!fragment.color_unresolved, "the colour resolved by layer");
    assert!(
        !fragment.lineweight_unresolved,
        "the lineweight resolved by layer"
    );
    assert!(
        !fragment.linetype_unresolved,
        "the linetype name resolved in the drawing's linetype table"
    );
}

/// Layers: decoy on id 0, layer `0` on id 9, RED on 4, BLUE on 5.
fn styled_builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    add_linetype(&mut b, 2, "DOTTED", &[0.5, -0.5]);
    add_linetype(&mut b, 3, "DASHDOT", &[2.0, -1.0, 0.0, -1.0]);
    // `LayerId(0)` is **not** layer "0": it is a decoy row with its own style,
    // so any id-based shortcut would show up in the assertions below.
    add_layer(
        &mut b,
        0,
        "DECOY",
        style([9, 9, 9], "DASHDOT", LayerLineWeight::Hundredths(90), 0.1),
    );
    // Layer "0" lives on id 9 and has a style of its own, distinct from every
    // other row.
    add_layer(
        &mut b,
        9,
        "0",
        style([1, 2, 3], "DOTTED", LayerLineWeight::Hundredths(50), 0.25),
    );
    add_layer(
        &mut b,
        4,
        "RED",
        style([255, 0, 0], "DASHED", LayerLineWeight::Hundredths(35), 0.75),
    );
    add_layer(
        &mut b,
        5,
        "BLUE",
        style([0, 0, 255], "DASHDOT", LayerLineWeight::Hundredths(20), 0.4),
    );
    b
}

#[test]
fn a_layer_zero_member_takes_all_four_styles_from_the_insert_layer() {
    let mut b = styled_builder();
    let zero = LayerId(9);
    let red = LayerId(4);
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(30)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_in(
        30,
        SpaceId::Block(BlockId(2)),
        zero,
        p(0.0, 0.0),
        p(10.0, 0.0),
    ))
    .unwrap();
    set_all_by_layer(&mut b, 30);
    let mut insert = insert_in(2, SpaceId::Model, 2, red, 10.0);
    insert.draw_order = 0;
    b.insert_entity(insert).unwrap();
    set_all_by_layer(&mut b, 2);
    let db = b.finish().unwrap();

    let rep = expand(&db, 2);
    assert!(!rep.fragments.is_empty(), "the block member is expanded");
    // Every emitted fragment (a dashed line subdivides) carries RED's style, not
    // layer "0"'s dotted/0.50/0.25/`[1,2,3]` and not the decoy's.
    for fragment in &rep.fragments {
        assert_style(fragment, [255, 0, 0], 0.75, 0.35);
        assert_eq!(fragment.linetype.elements, vec![2.0, -1.0]);
    }
    // The stored fact is untouched: the member is still on layer "0", and the
    // INSERT is still on RED.
    assert_eq!(db.entity(EntityId(30)).unwrap().layer, zero);
    assert_eq!(db.entity(EntityId(2)).unwrap().layer, red);
}

#[test]
fn a_member_on_the_decoy_row_does_not_inherit() {
    // Same-`INSERT` control for the name-based lookup: an entity stored on
    // `LayerId(0)` (named DECOY) is **not** on layer "0", so it keeps DECOY's
    // style instead of following the INSERT.
    let mut b = styled_builder();
    let decoy = LayerId(0);
    let red = LayerId(4);
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(31)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_in(
        31,
        SpaceId::Block(BlockId(2)),
        decoy,
        p(0.0, 0.0),
        p(10.0, 0.0),
    ))
    .unwrap();
    set_all_by_layer(&mut b, 31);
    let mut insert = insert_in(2, SpaceId::Model, 2, red, 10.0);
    insert.draw_order = 0;
    b.insert_entity(insert).unwrap();
    set_all_by_layer(&mut b, 2);
    let db = b.finish().unwrap();

    let rep = expand(&db, 2);
    for fragment in &rep.fragments {
        assert_style(fragment, [9, 9, 9], 0.1, 0.9);
        assert_eq!(fragment.linetype.elements, vec![2.0, -1.0, 0.0, -1.0]);
    }
}

#[test]
fn a_layer_zero_member_at_the_model_root_keeps_layer_zero() {
    // No containing INSERT: nothing to inherit, so layer "0"'s own style applies.
    let mut b = styled_builder();
    b.insert_entity(line_in(
        32,
        SpaceId::Model,
        LayerId(9),
        p(0.0, 0.0),
        p(10.0, 0.0),
    ))
    .unwrap();
    set_all_by_layer(&mut b, 32);
    let db = b.finish().unwrap();

    let rep = expand(&db, 32);
    for fragment in &rep.fragments {
        assert_style(fragment, [1, 2, 3], 0.25, 0.5);
        assert_eq!(fragment.linetype.elements, vec![0.5, -0.5]);
    }
}

#[test]
fn the_rule_chains_through_nested_inserts() {
    // Block A holds a layer-"0" line; block B holds an INSERT of A that is
    // **itself** on layer "0"; the model INSERT of B is on BLUE. The chain
    // resolves one level per INSERT: layer "0" -> B's INSERT layer (layer "0")
    // -> BLUE.
    let mut b = styled_builder();
    let zero = LayerId(9);
    let blue = LayerId(5);
    b.insert_block(BlockDefinition {
        id: BlockId(10),
        entities: vec![EntityId(50)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(11),
        entities: vec![EntityId(51)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_in(
        50,
        SpaceId::Block(BlockId(10)),
        zero,
        p(0.0, 0.0),
        p(10.0, 0.0),
    ))
    .unwrap();
    set_all_by_layer(&mut b, 50);
    // The inner INSERT is itself on layer "0" and ByLayer everywhere.
    b.insert_entity(insert_in(51, SpaceId::Block(BlockId(11)), 10, zero, 1.0))
        .unwrap();
    set_all_by_layer(&mut b, 51);
    let mut outer = insert_in(52, SpaceId::Model, 11, blue, 5.0);
    outer.draw_order = 0;
    b.insert_entity(outer).unwrap();
    set_all_by_layer(&mut b, 52);
    let db = b.finish().unwrap();

    let rep = expand(&db, 52);
    assert!(!rep.fragments.is_empty(), "the nested member is expanded");
    for fragment in &rep.fragments {
        assert_style(fragment, [0, 0, 255], 0.4, 0.2);
        assert_eq!(fragment.linetype.elements, vec![2.0, -1.0, 0.0, -1.0]);
    }
}

#[test]
fn a_drawing_without_layer_zero_behaves_exactly_as_before() {
    // No row named "0": the rule is off. A member on `LayerId(0)` keeps that
    // row's style instead of following the INSERT, with no panic and no guess.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    add_layer(
        &mut b,
        0,
        "MODEL",
        style([1, 2, 3], "DASHED", LayerLineWeight::Hundredths(50), 0.25),
    );
    add_layer(
        &mut b,
        4,
        "RED",
        style([255, 0, 0], "", LayerLineWeight::Hundredths(35), 0.75),
    );
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(30)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_in(
        30,
        SpaceId::Block(BlockId(2)),
        LayerId(0),
        p(0.0, 0.0),
        p(10.0, 0.0),
    ))
    .unwrap();
    set_all_by_layer(&mut b, 30);
    let mut insert = insert_in(2, SpaceId::Model, 2, LayerId(4), 10.0);
    insert.draw_order = 0;
    b.insert_entity(insert).unwrap();
    set_all_by_layer(&mut b, 2);
    let db = b.finish().unwrap();

    assert_eq!(db.layer_zero_id(), None, "the drawing has no layer \"0\"");
    let rep = expand(&db, 2);
    for fragment in &rep.fragments {
        assert_style(fragment, [1, 2, 3], 0.25, 0.5);
        assert_eq!(fragment.linetype.elements, vec![2.0, -1.0]);
    }
}
