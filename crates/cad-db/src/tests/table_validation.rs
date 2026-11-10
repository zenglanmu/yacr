//! TABLE write-validation contracts: merges, frame directions and borders.

use super::*;

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

fn parts() -> TableParts {
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

fn geometry(parts: TableParts) -> SemanticGeometry {
    SemanticGeometry::Table(Box::new(TableGeometry::from_import(parts)))
}

#[test]
fn a_merged_table_passes_write_validation() {
    let mut parts = parts();
    parts.rows[0].cells[0].merge_width = 2;
    crate::validate_geometry(&geometry(parts)).expect("a valid merged table");
}

#[test]
fn overlapping_merge_ranges_are_rejected() {
    // 3 columns x 2 rows. The first anchor spans rows 0..2 in column 2; the
    // second starts at (1, 0) and spans all three columns, reaching back into
    // the first anchor's cell (1, 2).
    let mut parts = parts();
    parts.columns.push(TableColumnGeometry {
        width: 2.0,
        name: "C".into(),
    });
    parts.rows[0].cells.push(cell(""));
    parts.rows.push(TableRowGeometry {
        height: 2.0,
        cells: vec![cell(""), cell(""), cell("")],
    });
    parts.rows[0].cells[2].merge_height = 2;
    parts.rows[1].cells[0].merge_width = 3;

    let error = crate::validate_geometry(&geometry(parts.clone()))
        .expect_err("overlapping merges are not writable");
    assert!(
        format!("{error:?}").contains("overlap"),
        "the reason names the defect: {error:?}"
    );
    // The creation path rejects the same structural defect.
    assert!(TableGeometry::new(parts).is_err());
}

#[test]
fn negative_double_line_spacing_is_rejected() {
    let mut parts = parts();
    parts.rows[0].cells[0].borders.top.double_spacing = -0.5;
    let error = crate::validate_geometry(&geometry(parts))
        .expect_err("negative double-line spacing is not writable");
    assert!(format!("{error:?}").contains("non-negative"), "{error:?}");
}

#[test]
fn a_zero_size_row_or_column_is_still_writable() {
    let mut parts = parts();
    parts.columns[0].width = 0.0;
    parts.rows[0].height = 0.0;
    // The other column and row still give the table a positive extent.
    parts.rows.push(TableRowGeometry {
        height: 3.0,
        cells: vec![cell(""), cell("x")],
    });
    parts.rows[0].cells[1].text = "y".into();
    crate::validate_geometry(&geometry(parts)).expect("zero-size lanes are writable");
}

#[test]
fn a_non_unit_frame_is_normalised_by_the_import_path() {
    let mut parts = parts();
    parts.frame.horizontal = Point3 {
        x: 3.0,
        y: 0.0,
        z: 0.0,
    };
    parts.frame.normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 9.0,
    };
    crate::validate_geometry(&geometry(parts)).expect("the importer normalises the frame");
}
