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

#[test]
fn display_is_one_rectangle_per_visible_cell_plus_text() {
    let table = TableGeometry::new(parts()).expect("valid table");
    // Two cells, four edges each, plus two text runs.
    assert_eq!(table.display().len(), 10, "{:?}", table.display());
    assert!(matches!(
        table.display().last(),
        Some(SemanticGeometry::Text { text, .. }) if text == "name"
    ));
    assert_eq!(table.texts(), vec!["id".to_string(), "name".to_string()]);
}

#[test]
fn transformed_scales_sizes_and_rotates_the_frame() {
    let table = TableGeometry::new(parts()).expect("valid table");
    // 90 degree rotation about Z then a uniform scale of 2.
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
    let transform = Transform3::scale(2.0).matrix_mul(&rotate);
    let moved = table.transformed(&transform).expect("similarity");
    assert!((moved.columns()[0].width - 8.0).abs() < 1e-9);
    assert!((moved.rows()[0].height - 6.0).abs() < 1e-9);
    assert!(moved.frame().horizontal.x.abs() < 1e-9);
    assert!((moved.frame().horizontal.y - 1.0).abs() < 1e-9);
    assert_eq!(moved.display().len(), table.display().len());
}

#[test]
fn non_uniform_scale_and_mirror_are_refused() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let mut non_uniform = Transform3::identity();
    non_uniform.matrix[1][1] = 2.0;
    assert!(matches!(
        table.transformed(&non_uniform),
        Err(CadError::Unsupported(_))
    ));
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    assert!(matches!(
        table.transformed(&mirror),
        Err(CadError::Unsupported(_))
    ));
}

#[test]
fn with_cell_text_rebuilds_and_refuses_out_of_range() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let edited = table.with_cell_text(0, 0, "code").expect("in range");
    assert_eq!(edited.texts()[0], "code");
    assert!(table.with_cell_text(1, 0, "x").is_err());
    assert!(table.with_cell_text(0, 9, "x").is_err());
}

#[test]
fn merge_outside_the_grid_is_rejected() {
    let mut parts = parts();
    parts.rows[0].cells[1].merge_width = 2;
    assert!(TableGeometry::new(parts).is_err());
}

#[test]
fn from_import_normalises_the_frame_directions() {
    let mut parts = parts();
    parts.frame.horizontal = Point3 {
        x: 5.0,
        y: 0.0,
        z: 0.0,
    };
    parts.frame.normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 7.0,
    };
    let table = TableGeometry::from_import(parts);
    let frame = table.frame();
    assert!((frame.horizontal.x - 1.0).abs() < 1e-12, "{frame:?}");
    assert!((frame.normal.z - 1.0).abs() < 1e-12, "{frame:?}");
}

#[test]
fn rotated_table_cell_text_carries_the_frame_rotation() {
    let table = TableGeometry::new(parts()).expect("valid table");
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
    let moved = table.transformed(&rotate).expect("rotation is supported");
    let mut texts = 0;
    for child in moved.display() {
        if let SemanticGeometry::Text {
            rotation, position, ..
        } = child
        {
            texts += 1;
            assert!(
                (rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
                "cell text rotation must follow the table: {rotation}"
            );
            // The first cell centre was (2, 1.5); a 90 degree rotation maps
            // it to (-1.5, 2).
            if texts == 1 {
                assert!((position.x + 1.5).abs() < 1e-9, "{position:?}");
                assert!((position.y - 2.0).abs() < 1e-9, "{position:?}");
            }
        }
    }
    assert_eq!(texts, 2, "both cell texts stay present");
}

#[test]
fn a_transform_that_tilts_the_plane_is_refused() {
    let table = TableGeometry::new(parts()).expect("valid table");
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
    let error = table.transformed(&tilt).expect_err("tilt is refused");
    assert!(
        matches!(error, CadError::Unsupported(ref message) if message.contains("tilt")),
        "{error:?}"
    );
}

#[test]
fn a_huge_merge_span_is_clamped_to_the_grid() {
    let mut parts = parts();
    parts.rows[0].cells[0].merge_width = 1_000_000;
    let table = TableGeometry::from_import(parts);
    assert_eq!(table.cell(0, 0).unwrap().merge_width, 2);
    assert!(!table.display().is_empty());
}

#[test]
fn overlapping_merges_are_rejected() {
    // 3 columns x 2 rows. The first anchor spans rows 0..2 in column 2; the
    // second anchor starts at (1, 0) and spans all three columns, reaching
    // back into the first anchor's cell (1, 2).
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
    let error = TableGeometry::new(parts).expect_err("overlap is structural");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("overlaps")),
        "{error:?}"
    );
}

#[test]
fn with_cell_texts_rebuilds_once_and_refuses_out_of_range() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let edited = table
        .with_cell_texts(vec![(0, 0, "a".into()), (0, 1, "b".into())])
        .expect("in range");
    assert_eq!(edited.texts(), vec!["a".to_string(), "b".to_string()]);
    assert!(table
        .with_cell_texts(vec![(0, 0, "a".into()), (9, 0, "b".into())])
        .is_err());
}
