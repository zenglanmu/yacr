//! TABLE export contract tests.
//!
//! A first-class `SemanticGeometry::Table` is written as a real `ACAD_TABLE`
//! entity from the domain grid (never from its derived display), and re-import
//! preserves the rows/columns, cell text and merge spans. The report names the
//! aspects that are deliberately not representable.

use std::io::Cursor;

use acadrust::{DxfReader, EntityType};

use super::*;
use crate::tests_multileader_fixtures::{db_with_block, export_db, export_one, import_bytes};

fn cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
    }
}

/// A 2x2 table whose first row is one merged 2-column anchor with text.
fn source_table() -> TableGeometry {
    let mut anchor = cell("Total");
    anchor.merge_width = 2;
    TableGeometry::from_import(TableParts {
        frame: TableFrame {
            origin: Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            },
            horizontal: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        },
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 6.0,
                name: "B".into(),
            },
        ],
        rows: vec![
            TableRowGeometry {
                height: 3.0,
                cells: vec![anchor, cell("")],
            },
            TableRowGeometry {
                height: 2.0,
                cells: vec![cell("x"), cell("y")],
            },
        ],
        style_name: "Standard".into(),
        name: "Data".into(),
        description: "linked".into(),
    })
}

#[test]
fn table_exports_as_an_acadtable_entity_with_cells_and_merges() {
    let source = source_table();
    let (bytes, report) = export_one(SemanticGeometry::Table(Box::new(source.clone())));

    // 1. The written file parses and carries a real TABLE entity.
    let document = DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    let written = document
        .entities()
        .find_map(|entity| match entity {
            EntityType::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("one TABLE entity");
    assert_eq!(written.rows.len(), 2);
    assert_eq!(written.columns.len(), 2);
    assert!((written.columns[0].width - 4.0).abs() < 1e-9);
    assert!((written.columns[1].width - 6.0).abs() < 1e-9);
    assert_eq!(written.rows[0].cells[0].merge_width, 2);
    // The table name/description live on a `TABLECONTENT` object that acadrust's
    // DXF writer does not emit for a plain entity; the report must say so
    // instead of implying they round-tripped.
    assert!(
        report.entries.iter().any(|entry| entry
            .message
            .contains("linked table-data name and description")),
        "{report:?}"
    );

    // 2. The report marks it Converted with the named losses.
    assert!(report.counts.converted >= 1, "{report:?}");
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.table"),
        "{report:?}"
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.message.contains("TABLESTYLE object is not written")),
        "{report:?}"
    );

    // 3. Re-import preserves the cells, text and merge span.
    let reimported = import_bytes(&bytes);
    let entity = reimported
        .model_space()
        .into_iter()
        .find(|entity| entity.object.type_key == "AcDbTable")
        .expect("AcDbTable re-imported");
    let SemanticGeometry::Table(table) = &entity.geometry else {
        panic!("expected a Table variant, got {:?}", entity.geometry);
    };
    assert_eq!(table.rows().len(), 2);
    assert_eq!(table.columns().len(), 2);
    assert_eq!(table.rows()[0].cells[0].merge_width, 2);
    assert_eq!(
        table.texts(),
        vec!["Total".to_string(), "x".to_string(), "y".to_string()]
    );
}

fn imported_table(database: &cad_db::DrawingDatabase) -> &TableGeometry {
    let entity = database
        .model_space()
        .into_iter()
        .find(|entity| entity.object.type_key == "AcDbTable")
        .expect("AcDbTable re-imported");
    match &entity.geometry {
        SemanticGeometry::Table(table) => table,
        other => panic!("expected a Table variant, got {other:?}"),
    }
}

/// 90 degree rotation about Z followed by a translation.
fn rotate_and_translate() -> Transform3 {
    let rotate = Transform3::from_basis(
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Point3 {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    Transform3::translation(Point3 {
        x: 100.0,
        y: 50.0,
        z: 0.0,
    })
    .matrix_mul(&rotate)
}

#[test]
fn exported_merged_table_passes_write_validation_after_reimport() {
    let source = source_table();
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(source)));
    let reimported = import_bytes(&bytes);
    let table = imported_table(&reimported);
    cad_db::validate_geometry(&SemanticGeometry::Table(Box::new(table.clone())))
        .expect("the re-imported merged table must be writable");
}

#[test]
fn a_block_embedded_table_is_exported_at_its_placed_frame() {
    let geometry = source_table();
    let (bytes, report) = export_db(&db_with_block(
        SemanticGeometry::Table(Box::new(geometry)),
        rotate_and_translate(),
    ));
    assert!(report.counts.dropped == 0, "{report:?}");

    let document = DxfReader::from_reader(Cursor::new(bytes.clone()))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    let written = document
        .entities()
        .find_map(|entity| match entity {
            EntityType::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("one TABLE entity");
    // `source_table` has its origin at (1, 2); a 90 degree rotation maps it to
    // (-2, 1) and the translation adds (100, 50), so the placed origin is
    // (98, 51).
    assert!(
        (written.insertion_point.x - 98.0).abs() < 1e-9,
        "{written:?}"
    );
    assert!(
        (written.insertion_point.y - 51.0).abs() < 1e-9,
        "{written:?}"
    );
    assert!(written.horizontal_direction.x.abs() < 1e-9, "{written:?}");
    assert!(
        (written.horizontal_direction.y - 1.0).abs() < 1e-9,
        "{written:?}"
    );

    // A re-import sees the placed frame, not block-local coordinates.
    let reimported = import_bytes(&bytes);
    let table = imported_table(&reimported);
    let frame = table.frame();
    assert!((frame.origin.x - 98.0).abs() < 1e-9, "{frame:?}");
    assert!((frame.origin.y - 51.0).abs() < 1e-9, "{frame:?}");
    assert!(frame.horizontal.x.abs() < 1e-9, "{frame:?}");
    assert!((frame.horizontal.y - 1.0).abs() < 1e-9, "{frame:?}");
}

#[test]
fn a_tilted_block_placement_drops_the_table_without_writing_it() {
    let (sin, cos) = std::f64::consts::FRAC_PI_4.sin_cos();
    let tilt = Transform3::from_basis(
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: cos,
            z: sin,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    );
    let (bytes, report) = export_db(&db_with_block(
        SemanticGeometry::Table(Box::new(source_table())),
        tilt,
    ));
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.dropped.transform"
                && entry
                    .message
                    .contains("cannot place Table under an insert transform")),
        "{report:?}"
    );
    let document = DxfReader::from_reader(Cursor::new(bytes))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    assert!(
        !document
            .entities()
            .any(|entity| matches!(entity, EntityType::Table(_))),
        "a refused placement must not write a block-local table"
    );
}

#[test]
fn every_distinct_cell_style_in_one_row_is_registered() {
    let mut first = cell("a");
    first.style_name = "StyleA".into();
    let mut second = cell("b");
    second.style_name = "StyleB".into();
    let table = TableGeometry::from_import(TableParts {
        frame: TableFrame::default(),
        columns: vec![
            TableColumnGeometry {
                width: 4.0,
                name: "A".into(),
            },
            TableColumnGeometry {
                width: 4.0,
                name: "B".into(),
            },
        ],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![first, second],
        }],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    });
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(table)));
    let document = DxfReader::from_reader(Cursor::new(bytes))
        .expect("reader")
        .read()
        .expect("written DXF parses");
    assert!(
        document.text_styles.get("StyleA").is_some(),
        "StyleA is missing from the written STYLE table"
    );
    assert!(
        document.text_styles.get("StyleB").is_some(),
        "StyleB is missing from the written STYLE table"
    );
}

#[test]
fn cell_text_height_and_style_survive_export_and_reimport() {
    let mut styled = cell("x");
    styled.text_height = 5.0;
    styled.style_name = "MyStyle".into();
    let table = TableGeometry::from_import(TableParts {
        frame: TableFrame::default(),
        columns: vec![TableColumnGeometry {
            width: 4.0,
            name: "A".into(),
        }],
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![styled],
        }],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    });
    let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(table)));
    let reimported = import_bytes(&bytes);
    let imported = imported_table(&reimported);
    let cell = imported.cell(0, 0).expect("the re-imported cell");
    assert!((cell.text_height - 5.0).abs() < 1e-9, "{cell:?}");
    assert_eq!(cell.style_name, "MyStyle");
}

#[test]
fn a_non_standard_table_style_name_is_reported_as_not_written() {
    let source = source_table();
    let source = TableGeometry::from_import(TableParts {
        frame: source.frame(),
        columns: source.columns().to_vec(),
        rows: source.rows().to_vec(),
        style_name: "MyTableStyle".into(),
        name: source.name().into(),
        description: source.description().into(),
    });
    let (bytes, report) = export_one(SemanticGeometry::Table(Box::new(source)));
    assert!(
        report.entries.iter().any(|entry| entry
            .message
            .contains("the table style name is also not written")),
        "{report:?}"
    );
    let reimported = import_bytes(&bytes);
    assert_ne!(
        imported_table(&reimported).style_name(),
        "MyTableStyle",
        "the name is not preserved"
    );
}
