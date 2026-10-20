//! Wildcard-regression test for ADR 0017 F.3: a dimension's measurement-text
//! font must be collected, or the text is silently never drawn.

use cad_db::{DrawingDatabase, DrawingDatabaseBuilder, Layer, Style};
use cad_domain::{
    DatabaseId, DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints,
    DimensionStyleValues, LayerId, Point3, SemanticGeometry, StyleId,
};

use crate::font_keys::{collect_fonts, needs_text_shaping};

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
        center_mark: 0.0,
        dim_line_increment: 3.75,
        text_vertical: 1,
        text_vertical_position: 0.0,
        suppress_ext1: false,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: false,
        linear_factor: 1.0,
        decimals: 4,
        rounding: 0.0,
        postfix: String::new(),
        zero_suppression: 0,
        linear_unit_format: 2,
        decimal_separator: 46,
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

/// A dimension whose text style names no font still needs shaping: the host must
/// install an engine (and resolve through the fallback chain) even though
/// `requested_fonts` is empty. This is the created-dimension defect.
#[test]
fn a_dimension_with_no_font_still_needs_text_shaping() {
    let mut style = style();
    style.text_font = None;
    let dimension = DimensionGeometry::synthesized(definition(), "Standard".into(), style, None)
        .expect("synthesized");
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
    builder
        .insert_entity(cad_db::DbEntity {
            object: cad_db::DbObject {
                id: cad_domain::ObjectId(1),
                type_key: "AcDbDimension".into(),
                revision: cad_domain::Revision(0),
                source_handle: None,
            },
            id: cad_domain::EntityId(1),
            layer: LayerId(0),
            space: cad_domain::SpaceId::Model,
            geometry: SemanticGeometry::Dimension(Box::new(dimension)),
            draw_order: 0,
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    assert!(crate::fonts::requested_fonts(&drawing).is_empty());
    assert!(needs_text_shaping(&drawing));
}

/// The optimisation is preserved: a drawing with only geometry installs no font
/// engine, and whitespace-only text needs no glyphs.
#[test]
fn geometry_and_whitespace_only_text_need_no_shaping() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(cad_db::DbEntity {
            object: cad_db::DbObject {
                id: cad_domain::ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: cad_domain::Revision(0),
                source_handle: None,
            },
            id: cad_domain::EntityId(1),
            layer: LayerId(0),
            space: cad_domain::SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    let drawing = builder.finish().unwrap();
    assert!(!needs_text_shaping(&drawing));
}
