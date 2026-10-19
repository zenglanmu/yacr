//! Wildcard-regression test for ADR 0017 F.3: a dimension's measurement-text
//! font must be collected, or the text is silently never drawn.

use cad_db::{DrawingDatabase, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    DatabaseId, DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
    DimensionStyleValues, LayerId, Point3, SemanticGeometry, StyleId,
};

use crate::fonts::collect_fonts;

fn db() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder.finish().unwrap()
}

fn style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        text_above: true,
        linear_factor: 1.0,
        decimals: 4,
        suppress_trailing_zeros: false,
        text_style_name: "Standard".into(),
        text_style: StyleId(0),
        text_font: Some("arial.ttf".into()),
    }
}

fn definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: Point3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            second: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            dimension_line: Point3 {
                x: -1.0,
                y: -2.0,
                z: 0.0,
            },
        },
        text_middle: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        text_rotation: 0.0,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    }
}

#[test]
fn collect_fonts_returns_a_dimensions_text_font() {
    let dimension = DimensionGeometry::synthesized(definition(), "Standard".into(), style(), None)
        .expect("synthesized");
    let mut out = Vec::new();
    collect_fonts(
        &db(),
        &SemanticGeometry::Dimension(Box::new(dimension)),
        &mut out,
    );
    assert!(out.iter().any(|f| f == "arial.ttf"), "{out:?}");
}
