//! TABLE per-cell appearance: the provider resolves each derived primitive's own
//! colour/lineweight and the expansion keeps it.
//!
//! The channel is the smallest honest one this build has: a table is one entity
//! with several colours (a cell's text, its background and its border can each
//! differ), so the domain's derived display carries a parallel appearance per
//! primitive and the provider stamps it onto the fragment/batch that primitive
//! becomes. The contract these tests pin:
//!
//! * a cell colour reaches the fragment (and therefore `RenderBatch::color`) as
//!   a **concrete** value with `color_unresolved == false`, so `expand_entity`
//!   keeps it instead of applying the entity's `ByLayer`/`ByBlock` provenance;
//! * a border lineweight reaches `DisplayFragment::lineweight` the same way, and
//!   the provider reports the renderer's one-pixel limit by name;
//! * a table with **no** per-cell appearance produces exactly the fragments it
//!   produced before the channel existed (entity colour, unresolved placeholder,
//!   `Complete`).

use super::*;
use cad_domain::{
    TableBorder, TableCellAppearanceEdit, TableCellBorders, TableCellGeometry, TableColumnGeometry,
    TableFieldEdit, TableFrame, TableGeometry, TableParts, TableRowGeometry,
};

const RED: [u8; 3] = [255, 0, 0];
const YELLOW: [u8; 3] = [255, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn table_cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: cad_domain::TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders {
            top: TableBorder::VISIBLE,
            right: TableBorder::VISIBLE,
            bottom: TableBorder::VISIBLE,
            left: TableBorder::VISIBLE,
        },
        text_color: None,
        fill_color: None,
    }
}

/// A 1 x 1 table with one text cell on the world XY plane.
fn one_cell_table() -> TableGeometry {
    TableGeometry::from_import(TableParts {
        frame: TableFrame::default(),
        columns: vec![TableColumnGeometry {
            width: 4.0,
            name: String::new(),
        }],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![table_cell("A")],
        }],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    })
}

fn table_entity(id: u128, geometry: TableGeometry) -> DbEntity {
    let mut entity = entity(id, SemanticGeometry::Table(Box::new(geometry)));
    entity.object.type_key = "AcadTable".into();
    entity
}

fn build(geometry: TableGeometry) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build(&table_entity(1, geometry), &context())
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
fn a_table_without_appearance_produces_the_pre_channel_fragments() {
    let representation = build(one_cell_table());
    // Four border arcs and one text run: the pre-appearance draw, unchanged.
    assert_eq!(representation.fragments.len(), 5);
    for fragment in &representation.fragments {
        // The entity-level colour/lineweight placeholders are untouched, so the
        // expansion still applies the entity's own provenance.
        assert!(
            fragment.color_unresolved,
            "a table without a per-cell colour must stay entity-coloured"
        );
        assert!(fragment.lineweight_unresolved);
        assert_eq!(fragment.color, DEFAULT_RENDER_COLOR);
    }
    assert_eq!(representation.completeness, Completeness::Complete);
}

#[test]
fn a_cell_colour_reaches_its_fragment_as_a_concrete_value() {
    let table = one_cell_table()
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                text_color: TableFieldEdit::Set(RED),
                fill: TableFieldEdit::Set(YELLOW),
                border_color: TableFieldEdit::Set(BLUE),
                border_lineweight: TableFieldEdit::Set(50),
            },
        )
        .expect("in grid");
    let representation = build(table);
    // The fill quad, four borders and the text run.
    assert_eq!(representation.fragments.len(), 6);

    let fill = representation
        .fragments
        .iter()
        .find(|fragment| matches!(fragment.primitive, DisplayPrimitive::Mesh(_)))
        .expect("a filled background primitive");
    assert_color(fill, YELLOW);
    assert!(!fill.color_unresolved);

    let text = representation
        .fragments
        .iter()
        .find(|fragment| matches!(fragment.primitive, DisplayPrimitive::Text { .. }))
        .expect("the cell text");
    assert_color(text, RED);
    assert!(!text.color_unresolved);
    // A text primitive has no border width.
    assert!(text.lineweight_unresolved);

    let borders: Vec<&DisplayFragment> = representation
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment.primitive, DisplayPrimitive::Lines(_)))
        .collect();
    assert_eq!(borders.len(), 4);
    for border in borders {
        assert_color(border, BLUE);
        assert!(!border.color_unresolved);
        assert!(
            (border.lineweight - 0.5).abs() < 1e-6,
            "50 hundredths = 0.5 mm"
        );
        assert!(
            !border.lineweight_unresolved,
            "a modelled border weight is a concrete value"
        );
    }

    // The renderer's one-pixel limit is reported by name, not implied.
    assert!(representation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "representation.table_lineweight_not_drawn"));
    assert!(matches!(
        representation.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_cell_colour_wins_over_the_entity_provenance_but_only_for_that_primitive() {
    let mut builder = empty_db();
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .set_layer_attributes(
            LayerId(0),
            cad_db::LayerAttributes {
                color: cad_db::LayerColor {
                    aci: Some(1),
                    rgb: [255, 0, 0],
                },
                ..cad_db::LayerAttributes::default()
            },
        )
        .unwrap();
    let mut source = one_cell_table();
    // The styled cell is the only cell here, so the contrast is made by a second
    // (unstylised) table entity sharing the database below.
    source = source
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                text_color: TableFieldEdit::Set(YELLOW),
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect("in grid");
    let entity = table_entity(7, source);
    builder.insert_entity(entity.clone()).unwrap();
    let db = builder.finish().unwrap();

    let representation = ProviderRegistry::with_default_provider()
        .build_expanded(&db, db.entity(EntityId(7)).unwrap(), &context())
        .unwrap();
    let text = representation
        .fragments
        .iter()
        .find(|fragment| matches!(fragment.primitive, DisplayPrimitive::Text { .. }))
        .expect("the cell text");
    // The provider resolved the cell's own colour: the `ByLayer` red of the
    // entity must not overwrite it.
    assert_color(text, YELLOW);
    assert!(!text.color_unresolved);
    // Every primitive without an override keeps the entity's resolved colour,
    // i.e. the layer's red — the expansion is otherwise unchanged.
    let border = representation
        .fragments
        .iter()
        .find(|fragment| matches!(fragment.primitive, DisplayPrimitive::Lines(_)))
        .expect("a border");
    assert_color(border, [255, 0, 0]);
    assert!(!border.color_unresolved);
}
