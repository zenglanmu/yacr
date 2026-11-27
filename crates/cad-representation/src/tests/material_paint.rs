//! A painted entity draws in its material's flat colour.
//!
//! The assignment lives in the document material table, so the expansion looks
//! it up per entity and overrides the stored `ByLayer`/explicit look without an
//! entity write. These tests pin that rule, the transparency-to-alpha mapping
//! and the untouched unpainted path.

use super::*;
use cad_domain::material::MaterialDocument;

/// A model-space line on layer 0.
fn line(id: u128) -> DbEntity {
    line_entity(id, SpaceId::Model, p(0.0, 0.0), p(2.0, 0.0))
}

fn database(entities: Vec<DbEntity>, materials: MaterialDocument) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    builder.set_materials(materials).unwrap();
    builder.finish().unwrap()
}

fn expand(db: &DrawingDatabase, id: u128) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(id)).unwrap(), &context())
        .unwrap()
}

fn assert_fragment(fragment: &DisplayFragment, color: [u8; 3], alpha: f32) {
    for (channel, byte) in color.iter().enumerate() {
        let want = *byte as f32 / 255.0;
        assert!(
            (fragment.color[channel] - want).abs() < 1e-6,
            "channel {channel}: got {} want {want}",
            fragment.color[channel]
        );
    }
    assert!(
        (fragment.alpha - alpha).abs() < 1e-6,
        "alpha {} want {alpha}",
        fragment.alpha
    );
    assert!(
        !fragment.color_unresolved,
        "a paint is a concrete colour, never unresolved"
    );
}

fn first_fragment(representation: &DisplayRepresentation) -> &DisplayFragment {
    representation
        .fragments
        .first()
        .expect("the line expands to one fragment")
}

#[test]
fn a_painted_entity_draws_in_the_material_colour() {
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign(EntityId(7), red).unwrap();
    let db = database(vec![line(7)], materials);
    let representation = expand(&db, 7);
    assert_fragment(first_fragment(&representation), [178, 34, 34], 1.0);
}

#[test]
fn a_transparent_material_multiplies_the_resolved_alpha() {
    let mut materials = MaterialDocument::new();
    let glass = materials.ensure("Glass", [170, 220, 240], 0.6).unwrap();
    materials.assign(EntityId(7), glass).unwrap();
    let db = database(vec![line(7)], materials);
    let representation = expand(&db, 7);
    assert_fragment(first_fragment(&representation), [170, 220, 240], 0.4);
}

#[test]
fn a_paint_overrides_an_explicit_entity_colour_but_leaves_other_rows_alone() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(line(7)).unwrap();
    builder.insert_entity(line(8)).unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(8),
            attrs_styled(
                cad_db::EntityColor::Explicit([10, 20, 30]),
                cad_db::EntityLineWeight::ByLayer,
            ),
        )
        .unwrap();
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign(EntityId(7), red).unwrap();
    builder.set_materials(materials).unwrap();
    let db = builder.finish().unwrap();

    assert_fragment(first_fragment(&expand(&db, 7)), [178, 34, 34], 1.0);
    // The unpainted row keeps its explicit colour.
    assert_fragment(first_fragment(&expand(&db, 8)), [10, 20, 30], 1.0);
}
