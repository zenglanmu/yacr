//! `ByLayer` linetype and lineweight resolve from the layer table at draw time.
//!
//! The importer keeps the symbolic `EntityLineType::ByLayer` /
//! `EntityLineWeight::ByLayer` provenance instead of baking the layer's pattern
//! and millimetres onto the entity, so the representation resolves them here.
//! These tests pin the resolution rules (1/100 mm -> mm, the documented default,
//! a layer whose own weight is `ByLayer`, the layer's linetype *name* looked up
//! in the drawing's linetype table), the block-content rule, and the property
//! that makes the change worthwhile: a layer linetype/lineweight edit changes its
//! `ByLayer` entities on the next expansion with **no** entity write.

use super::*;
use cad_db::{LayerAttributes, LayerLineWeight, LineType as DbLineType, LinetypePattern};

/// Attributes for a layer with this linetype name and stored weight.
fn layer_attributes(linetype: &str, lineweight: LayerLineWeight) -> LayerAttributes {
    LayerAttributes {
        linetype: linetype.into(),
        lineweight,
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

/// Insert a named linetype row in the drawing's table.
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

/// A model-space line on `layer`.
fn line_on_layer(id: u128, layer: LayerId) -> DbEntity {
    let mut entity = line_entity(id, SpaceId::Model, p(0.0, 0.0), p(10.0, 0.0));
    entity.layer = layer;
    entity
}

/// Store this entity's import attributes: symbolic linetype + lineweight.
fn set_symbolic(builder: &mut DrawingDatabaseBuilder, id: u128, source: EntityLineType) {
    set_style(builder, id, source, EntityLineWeight::ByLayer);
}

/// Store this entity's import attributes with an explicit provenance per field.
fn set_style(
    builder: &mut DrawingDatabaseBuilder,
    id: u128,
    linetype: EntityLineType,
    lineweight: EntityLineWeight,
) {
    builder
        .set_entity_render_attributes(
            EntityId(id),
            EntityRenderAttributes {
                transparency: EntityTransparency::Explicit(1.0),
                color: cad_db::EntityColor::Explicit([1, 2, 3]),
                lineweight,
                linetype,
                ..Default::default()
            },
        )
        .unwrap();
}

/// Store this entity's import attributes plus the entity's own linetype scale
/// (the source's DXF group `48`), which is independent of the linetype variant.
fn set_style_with_scale(
    builder: &mut DrawingDatabaseBuilder,
    id: u128,
    linetype: EntityLineType,
    linetype_scale: f64,
) {
    builder
        .set_entity_render_attributes(
            EntityId(id),
            EntityRenderAttributes {
                transparency: EntityTransparency::Explicit(1.0),
                color: cad_db::EntityColor::Explicit([1, 2, 3]),
                lineweight: EntityLineWeight::ByLayer,
                linetype,
                linetype_scale,
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
fn by_layer_lineweight_resolves_from_the_owning_layer_row() {
    let mut b = empty_db();
    // The importer stores the source's 1/100 mm integer, so 35 -> exactly 0.35.
    let thin = add_layer(
        &mut b,
        1,
        "THIN",
        layer_attributes("Continuous", LayerLineWeight::Hundredths(35)),
    );
    let hairline = add_layer(
        &mut b,
        2,
        "HAIRLINE",
        layer_attributes("Continuous", LayerLineWeight::Hundredths(0)),
    );
    // DXF 370 = -3 on the layer: a real value, the documented default weight.
    let by_default = add_layer(
        &mut b,
        3,
        "BYDEFAULT",
        layer_attributes("Continuous", LayerLineWeight::Default),
    );
    // DXF 370 = -1 on a *layer* is degenerate: the layer has no weight to give.
    let no_weight = add_layer(
        &mut b,
        4,
        "NOWEIGHT",
        layer_attributes("Continuous", LayerLineWeight::ByLayer),
    );
    for (id, layer) in [
        (10u128, thin),
        (11, hairline),
        (12, by_default),
        (13, no_weight),
    ] {
        b.insert_entity(line_on_layer(id, layer)).unwrap();
        set_symbolic(&mut b, id, EntityLineType::ByLayer);
    }
    let db = b.finish().unwrap();

    let thin_fragment = &expand(&db, 10).fragments[0];
    assert_eq!(thin_fragment.lineweight, 0.35);
    assert!(
        !thin_fragment.lineweight_unresolved,
        "a weight read from the layer row is a resolved value"
    );

    assert_eq!(expand(&db, 11).fragments[0].lineweight, 0.0);
    assert!(!expand(&db, 11).fragments[0].lineweight_unresolved);

    let default_fragment = &expand(&db, 12).fragments[0];
    assert_eq!(default_fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(
        !default_fragment.lineweight_unresolved,
        "the layer's own `Default` (-3) is a value the source really carries"
    );

    let nothing_fragment = &expand(&db, 13).fragments[0];
    assert_eq!(nothing_fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(
        nothing_fragment.lineweight_unresolved,
        "a layer whose own lineweight is ByLayer has no value to give"
    );
}

#[test]
fn by_layer_linetype_resolves_the_layer_pattern_from_the_linetype_table() {
    let mut b = empty_db();
    // The layer stores a *name*; the pattern lives in the drawing's table.
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    let dashed = add_layer(
        &mut b,
        1,
        "DASHED-LAYER",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    b.insert_entity(line_on_layer(20, dashed)).unwrap();
    set_symbolic(&mut b, 20, EntityLineType::ByLayer);
    let db = b.finish().unwrap();

    let rep = expand(&db, 20);
    let fragment = &rep.fragments[0];
    assert_eq!(fragment.linetype.elements, vec![2.0, -1.0]);
    assert!(
        !fragment.linetype_unresolved,
        "the layer's named pattern is a resolved value"
    );
    // And the dashes are really drawn: a 10-unit line with a 2/1 pattern.
    assert!(
        line_primitives(&rep).len() > 1,
        "the resolved layer linetype must subdivide the line"
    );
    assert_eq!(rep.completeness, Completeness::Complete);
}

#[test]
fn by_layer_linetype_name_absent_from_the_table_is_continuous_and_unresolved() {
    let mut b = empty_db();
    // The layer names a linetype this drawing does not define: no pattern can be
    // resolved, so the line is continuous and the fragment says so.
    let ghost = add_layer(
        &mut b,
        1,
        "GHOST",
        layer_attributes("GHOSTLINE", LayerLineWeight::Hundredths(35)),
    );
    b.insert_entity(line_on_layer(21, ghost)).unwrap();
    set_symbolic(&mut b, 21, EntityLineType::ByLayer);
    let db = b.finish().unwrap();

    let rep = expand(&db, 21);
    assert_eq!(line_primitives(&rep).len(), 1);
    assert!(rep.fragments[0].linetype.is_continuous());
    assert!(
        rep.fragments[0].linetype_unresolved,
        "an undefined layer linetype name is never turned into invented dashes"
    );
}

#[test]
fn a_missing_layer_attribute_row_uses_the_documented_defaults() {
    // A row inserted with no attribute set at all: `layer_attributes` documents
    // `LayerAttributes::default()` (Continuous, ByLayer weight) for it. The
    // default therefore comes from the existing fallback, not a second one.
    let documented = LayerAttributes::default();
    assert_eq!(documented.linetype, "Continuous");
    assert_eq!(
        documented.lineweight,
        LayerLineWeight::ByLayer,
        "the default row itself carries no weight"
    );

    // (a) The drawing defines `Continuous`, so the layer's name resolves.
    let mut b = empty_db();
    b.insert_layer(Layer {
        id: LayerId(4),
        name: "NO-ROW".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_on_layer(22, LayerId(4))).unwrap();
    set_symbolic(&mut b, 22, EntityLineType::ByLayer);
    add_linetype(&mut b, 1, "Continuous", &[]);
    let db = b.finish().unwrap();

    let fragment = &expand(&db, 22).fragments[0];
    assert!(fragment.linetype.is_continuous());
    assert!(
        !fragment.linetype_unresolved,
        "`Continuous` is defined by the table, so the name resolves"
    );
    // The default row's own lineweight is symbolic, so the weight falls back to
    // the documented millimetres and is flagged rather than claimed.
    assert_eq!(fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(fragment.lineweight_unresolved);

    // (b) A drawing whose table does not define `Continuous` still invents no
    // pattern: continuous, flagged.
    let mut b = empty_db();
    b.insert_layer(Layer {
        id: LayerId(5),
        name: "NO-ROW-NO-TABLE".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_on_layer(23, LayerId(5))).unwrap();
    set_symbolic(&mut b, 23, EntityLineType::ByLayer);
    let db = b.finish().unwrap();

    let no_table_fragment = &expand(&db, 23).fragments[0];
    assert!(no_table_fragment.linetype.is_continuous());
    assert!(
        no_table_fragment.linetype_unresolved,
        "a drawing that does not define `Continuous` cannot resolve the name"
    );
    assert_eq!(no_table_fragment.lineweight, DEFAULT_LINEWEIGHT_MM);
    assert!(no_table_fragment.lineweight_unresolved);
}

#[test]
fn explicit_linetype_and_lineweight_are_untouched_by_the_layer_table() {
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    let dashed = add_layer(
        &mut b,
        1,
        "DASHED-LAYER",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    b.insert_entity(line_on_layer(24, dashed)).unwrap();
    let stored = EntityLineType::Explicit {
        name: "DOTTED".into(),
        pattern: LinetypePattern::from_elements([0.5, -0.5]),
        scale: 2.5,
    };
    b.set_entity_render_attributes(
        EntityId(24),
        EntityRenderAttributes {
            transparency: EntityTransparency::Explicit(1.0),
            color: cad_db::EntityColor::Explicit([1, 2, 3]),
            lineweight: EntityLineWeight::Explicit(0.13),
            linetype: stored.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    let db = b.finish().unwrap();

    let fragment = &expand(&db, 24).fragments[0];
    // Byte-identical: the layer's pattern/weight never replaces an explicit one.
    let EntityLineType::Explicit { pattern, scale, .. } = &stored else {
        panic!("the fixture stores an explicit linetype");
    };
    assert_eq!(&fragment.linetype, pattern);
    assert!(!fragment.linetype_unresolved);
    assert_eq!(fragment.linetype_scale, *scale as f32);
    assert_eq!(fragment.lineweight, 0.13);
    assert!(!fragment.lineweight_unresolved);
}

#[test]
fn editing_a_layer_linetype_and_lineweight_changes_both_with_no_entity_write() {
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    add_linetype(&mut b, 2, "DASHDOT", &[1.0, -1.0, 0.0, -1.0]);
    let walls = add_layer(
        &mut b,
        1,
        "WALLS",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    b.insert_entity(line_on_layer(25, walls)).unwrap();
    set_symbolic(&mut b, 25, EntityLineType::ByLayer);
    let mut db = b.finish().unwrap();

    let before = expand(&db, 25);
    assert_eq!(before.fragments[0].linetype.elements, vec![2.0, -1.0]);
    assert_eq!(before.fragments[0].lineweight, 0.35);

    let entity_before = db.entity(EntityId(25)).unwrap().clone();
    let revision_before = db.revision();

    // The same post-import write `SETLAYERPROPERTY linetype/lineweight`
    // performs.
    let change_set = db
        .set_layer_attributes(
            walls,
            layer_attributes("DASHDOT", LayerLineWeight::Hundredths(50)),
            TransactionId(1),
            "SETLAYERPROPERTY linetype lineweight",
        )
        .expect("a layer style edit is allowed");
    assert!(
        change_set.changes.is_empty(),
        "a layer edit publishes no object change: {:?}",
        change_set.changes
    );
    assert_eq!(
        db.entity(EntityId(25)).unwrap(),
        &entity_before,
        "the entity row (including its revision) is untouched"
    );
    assert_eq!(db.revision(), Revision(revision_before.0 + 1));

    // Same stored attributes on the entity, new layer style: no re-import, no
    // entity update.
    let after = expand(&db, 25);
    assert_eq!(
        after.fragments[0].linetype.elements,
        vec![1.0, -1.0, 0.0, -1.0]
    );
    assert_eq!(after.fragments[0].lineweight, 0.5);
    assert!(!after.fragments[0].linetype_unresolved);
    assert!(!after.fragments[0].lineweight_unresolved);
}

#[test]
fn block_content_resolves_by_layer_against_its_own_layer() {
    // The same rule as colour: every entity resolves `ByLayer` against the layer
    // it is **itself** on, including entities inside a block definition. Only
    // `ByBlock` walks up the parent chain to the containing INSERT's already
    // resolved value, which may have come from the INSERT's own layer.
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    add_linetype(&mut b, 2, "DOTTED", &[0.5, -0.5]);
    let child_layer = add_layer(
        &mut b,
        1,
        "CHILD",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    let insert_layer = add_layer(
        &mut b,
        2,
        "INSERT",
        layer_attributes("DOTTED", LayerLineWeight::Hundredths(50)),
    );

    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(30), EntityId(31)],
        dynamic_visibility: None,
    })
    .unwrap();
    // Block child on CHILD with ByLayer: resolves to CHILD, not the INSERT layer.
    b.insert_entity(line_on_layer(30, child_layer)).unwrap();
    set_symbolic(&mut b, 30, EntityLineType::ByLayer);
    // Block child on CHILD with ByBlock: inherits the INSERT's resolved values.
    b.insert_entity(line_on_layer(31, child_layer)).unwrap();
    set_style(
        &mut b,
        31,
        EntityLineType::ByBlock,
        EntityLineWeight::ByBlock,
    );

    // The INSERT itself sits on INSERT and is ByLayer there.
    let mut insert = insert_entity(2, SpaceId::Model, 2, 10.0);
    insert.layer = insert_layer;
    b.insert_entity(insert).unwrap();
    set_symbolic(&mut b, 2, EntityLineType::ByLayer);
    let db = b.finish().unwrap();

    let rep = expand(&db, 2);
    // Dashed linetypes subdivide a line, so count groups rather than fragments:
    // the first child's fragments all carry CHILD's values, the second child's
    // fragments all carry the INSERT's inherited values.
    assert!(rep.fragments.len() >= 2, "both block children are expanded");
    let first = &rep.fragments[0];
    assert_eq!(first.linetype.elements, vec![2.0, -1.0]);
    assert_eq!(first.lineweight, 0.35);
    let last = rep.fragments.last().unwrap();
    assert_eq!(last.linetype.elements, vec![0.5, -0.5]);
    assert_eq!(last.lineweight, 0.5);
    for fragment in &rep.fragments {
        assert!(
            !fragment.linetype_unresolved && !fragment.lineweight_unresolved,
            "every child resolves: ByLayer through its own layer, ByBlock through the INSERT"
        );
    }
}

#[test]
fn a_by_layer_linetype_applies_the_entity_linetype_scale() {
    // The entity's own scale (DXF group `48`) used to be lost with the symbolic
    // variant: only `Explicit` had a slot for it. It now lives on
    // `EntityRenderAttributes::linetype_scale` for every provenance, so a
    // `ByLayer` entity with `2.0` must render exactly like an explicit linetype
    // with `scale = 2.0`.
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    let dashed = add_layer(
        &mut b,
        1,
        "DASHED-LAYER",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    // (a) group 48 = 1.0 is the identity: cycle 3 over a 10-unit line -> 4 dashes.
    b.insert_entity(line_on_layer(40, dashed)).unwrap();
    set_style_with_scale(&mut b, 40, EntityLineType::ByLayer, 1.0);
    // (b) group 48 = 2.0 doubles the cycle: dash 4 / gap 2 -> 2 dashes.
    b.insert_entity(line_on_layer(41, dashed)).unwrap();
    set_style_with_scale(&mut b, 41, EntityLineType::ByLayer, 2.0);
    // (c) a corrupt factor degrades to the identity instead of erasing dashes.
    b.insert_entity(line_on_layer(42, dashed)).unwrap();
    set_style_with_scale(&mut b, 42, EntityLineType::ByLayer, f64::NAN);
    // (d) the explicit reference the symbolic path must match.
    b.insert_entity(line_on_layer(43, dashed)).unwrap();
    set_style_with_scale(
        &mut b,
        43,
        EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 2.0,
        },
        2.0,
    );
    let db = b.finish().unwrap();

    let identity = expand(&db, 40);
    assert_eq!(identity.fragments[0].linetype_scale, 1.0);
    assert_eq!(identity.fragments[0].linetype.elements, vec![2.0, -1.0]);
    assert_eq!(line_primitives(&identity).len(), 4);

    let doubled = expand(&db, 41);
    assert_eq!(doubled.fragments[0].linetype_scale, 2.0);
    assert_eq!(
        doubled.fragments[0].linetype.elements,
        vec![2.0, -1.0],
        "the layer's pattern is scaled at draw time, never rewritten"
    );
    assert!(!doubled.fragments[0].linetype_unresolved);
    assert_eq!(
        line_primitives(&doubled).len(),
        2,
        "2/1 scaled by 2 is a 6-unit cycle over a 10-unit line"
    );

    // The explicit path is the yardstick: same pattern, same scale, same dashes.
    let explicit = expand(&db, 43);
    assert_eq!(
        explicit.fragments[0].linetype,
        doubled.fragments[0].linetype
    );
    assert_eq!(
        explicit.fragments[0].linetype_scale,
        doubled.fragments[0].linetype_scale
    );
    assert_eq!(
        line_primitives(&explicit).len(),
        line_primitives(&doubled).len()
    );

    let corrupt = expand(&db, 42);
    assert_eq!(
        corrupt.fragments[0].linetype_scale, 1.0,
        "a non-finite factor sanitises to the identity"
    );
    assert_eq!(line_primitives(&corrupt).len(), 4);
}

#[test]
fn an_explicit_linetype_keeps_its_variant_scale_unchanged() {
    // The explicit path resolves the copy inside its own variant, so the new
    // attribute field is not applied a second time: the variant's `1.0` wins over
    // a stored `2.0`.
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    let dashed = add_layer(
        &mut b,
        1,
        "DASHED-LAYER",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    b.insert_entity(line_on_layer(44, dashed)).unwrap();
    set_style_with_scale(
        &mut b,
        44,
        EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([2.0, -1.0]),
            scale: 1.0,
        },
        2.0,
    );
    let db = b.finish().unwrap();

    let rep = expand(&db, 44);
    assert_eq!(rep.fragments[0].linetype_scale, 1.0);
    assert_eq!(line_primitives(&rep).len(), 4);
}

#[test]
fn a_by_block_linetype_inherits_the_parents_resolved_scale() {
    // `ByBlock` inherits the containing INSERT's *resolved* scale, not just its
    // pattern: an INSERT that is `ByLayer` with group 48 = 2.0 hands `2.0` down.
    // A child's own factor multiplies the inherited one, and the default `1.0` is
    // the identity.
    let mut b = empty_db();
    add_linetype(&mut b, 1, "DASHED", &[2.0, -1.0]);
    let insert_layer = add_layer(
        &mut b,
        1,
        "INSERT",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    let child_layer = add_layer(
        &mut b,
        2,
        "CHILD",
        layer_attributes("DASHED", LayerLineWeight::Hundredths(35)),
    );
    b.insert_block(BlockDefinition {
        id: BlockId(2),
        entities: vec![EntityId(50), EntityId(51)],
        dynamic_visibility: None,
    })
    .unwrap();
    // 50: plain `ByBlock`, group 48 absent -> exactly the parent's resolved scale.
    b.insert_entity(line_on_layer(50, child_layer)).unwrap();
    set_style(
        &mut b,
        50,
        EntityLineType::ByBlock,
        EntityLineWeight::ByBlock,
    );
    // 51: `ByBlock` with its own group 48 = 2.0 -> 2.0 x inherited 2.0.
    b.insert_entity(line_on_layer(51, child_layer)).unwrap();
    set_style_with_scale(&mut b, 51, EntityLineType::ByBlock, 2.0);

    let mut insert = insert_entity(2, SpaceId::Model, 2, 10.0);
    insert.layer = insert_layer;
    b.insert_entity(insert).unwrap();
    set_style_with_scale(&mut b, 2, EntityLineType::ByLayer, 2.0);
    let db = b.finish().unwrap();

    let rep = expand(&db, 2);
    let scale_of = |id: u128| -> Vec<f32> {
        rep.fragments
            .iter()
            .filter(|fragment| fragment.source.entity == EntityId(id))
            .map(|fragment| fragment.linetype_scale)
            .collect()
    };
    let inherited = scale_of(50);
    assert!(
        !inherited.is_empty(),
        "the plain ByBlock child emits fragments"
    );
    assert!(
        inherited.iter().all(|scale| *scale == 2.0),
        "the INSERT's resolved scale is inherited, got {inherited:?}"
    );
    let multiplied = scale_of(51);
    assert!(!multiplied.is_empty(), "the scaled child emits fragments");
    assert!(
        multiplied.iter().all(|scale| *scale == 4.0),
        "a child's own factor multiplies the inherited one, got {multiplied:?}"
    );
    for fragment in &rep.fragments {
        assert!(!fragment.linetype_unresolved);
    }
}
