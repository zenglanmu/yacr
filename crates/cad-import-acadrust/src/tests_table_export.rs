//! TABLE export contract tests.
//!
//! A first-class `SemanticGeometry::Table` is written as a real `ACAD_TABLE`
//! entity from the domain grid (never from its derived display), and re-import
//! preserves the rows/columns, cell text and merge spans. The report names the
//! aspects that are deliberately not representable.

use std::io::Cursor;

use acadrust::{DxfReader, EntityType};

use super::*;
use crate::tests_multileader_fixtures::{export_one, import_bytes};

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
