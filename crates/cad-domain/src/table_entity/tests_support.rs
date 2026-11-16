//! Shared fixtures for the table-entity tests (the derivation tests and the
//! appearance-channel tests). Split out so both test files build the same table
//! and neither duplicates the builders.

use super::*;

pub(super) fn cell(text: &str) -> TableCellGeometry {
    TableCellGeometry {
        text: text.to_string(),
        text_height: 2.5,
        style_name: "Standard".into(),
        style: StyleId(0),
        font: None,
        alignment: TableCellAlignment::DEFAULT,
        rotation: 0.0,
        merge_width: 1,
        merge_height: 1,
        borders: TableCellBorders::default(),
        text_color: None,
        fill_color: None,
    }
}

pub(super) fn parts() -> TableParts {
    TableParts {
        frame: TableFrame::default(),
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
        rows: vec![TableRowGeometry {
            height: 3.0,
            cells: vec![cell("id"), cell("name")],
        }],
        style_name: "Standard".into(),
        name: String::new(),
        description: String::new(),
    }
}

pub(super) fn grid(rows: u32, cols: u32) -> TableParts {
    let mut parts = parts();
    parts.columns = (0..cols)
        .map(|_| TableColumnGeometry {
            width: 4.0,
            name: String::new(),
        })
        .collect();
    parts.rows = (0..rows)
        .map(|_| TableRowGeometry {
            height: 3.0,
            cells: (0..cols).map(|_| cell("")).collect(),
        })
        .collect();
    parts
}

pub(super) fn resolve_none(_name: &str) -> (StyleId, Option<String>) {
    (StyleId(0), None)
}
