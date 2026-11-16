use super::*;

fn cell(text: &str) -> TableCellGeometry {
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

// ---------------------------------------------------------------------------
// Merge / unmerge (TABLEDIT MERGE|UNMERGE).
// ---------------------------------------------------------------------------

fn grid(rows: u32, cols: u32) -> TableParts {
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

#[test]
fn merging_two_cells_drops_the_shared_edge_and_survives_unmerge() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    // Unmerged: four cells, four edges each.
    assert_eq!(table.display().len(), 16);
    let merged = table.with_merge(0, 0, 1, 2).expect("in grid");
    assert_eq!(merged.cell(0, 0).unwrap().merge_width, 2);
    // The merged row owns one 2-column wide cell; the covered cell draws
    // nothing, so the row emits one cell rectangle instead of two.
    assert_eq!(merged.display().len(), 8 + 4);
    assert!(merged.is_cell_covered(0, 1));
    assert!(!merged.is_cell_covered(0, 0));
    assert_eq!(merged.merge_anchor(0, 1), Some((0, 0)));

    let unmerged = merged
        .with_unmerge(0, 1)
        .expect("a covered address resolves");
    assert_eq!(unmerged.cell(0, 0).unwrap().merge_width, 1);
    assert_eq!(unmerged.display().len(), 16);
    assert!(!unmerged.is_cell_covered(0, 1));
}

#[test]
fn a_merge_outside_the_grid_or_onto_a_covered_cell_is_refused() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let error = table.with_merge(1, 1, 2, 1).expect_err("past the last row");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.merge_out_of_grid")),
        "{error:?}"
    );
    let error = table.with_merge(0, 0, 0, 2).expect_err("a zero span");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.merge_span")),
        "{error:?}"
    );
    let error = table.with_merge(4, 4, 1, 1).expect_err("outside the grid");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.cell_out_of_range")),
        "{error:?}"
    );
    let merged = table.with_merge(0, 0, 2, 2).expect("one big merge");
    let error = merged.with_merge(1, 1, 1, 1).expect_err("covered anchor");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.merge_covered")),
        "{error:?}"
    );
    assert!(merged.with_merge(0, 0, 3, 1).is_err(), "overlaps itself");
}

#[test]
fn unmerging_a_single_cell_is_refused() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let error = table.with_unmerge(0, 0).expect_err("not merged");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.not_merged")),
        "{error:?}"
    );
}

#[test]
fn a_merge_anchor_keeps_its_text_and_the_covered_cells_are_not_drawn() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].text = "anchor".into();
    parts.rows[0].cells[1].text = "hidden".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let merged = table.with_merge(0, 0, 1, 2).expect("in grid");
    // The stored content is never dropped (FIND/properties read it)...
    assert_eq!(
        merged.texts(),
        vec!["anchor".to_string(), "hidden".to_string()]
    );
    // ...but only the anchor draws text.
    let drawn: Vec<&str> = merged
        .display()
        .iter()
        .filter_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(drawn, vec!["anchor"]);
}

// ---------------------------------------------------------------------------
// MATCHCELL (cell style copy).
// ---------------------------------------------------------------------------

#[test]
fn a_cell_style_copy_moves_height_style_and_borders_but_not_text() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].text = "source".into();
    parts.rows[0].cells[0].text_height = 4.0;
    parts.rows[0].cells[0].style_name = "Title".into();
    parts.rows[0].cells[0].style = StyleId(7);
    parts.rows[0].cells[0].borders.top = TableBorder::HIDDEN;
    parts.rows[1].cells[1].text = "target text".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let copied = table
        .with_cell_styles((0, 0), &[(1, 1), (0, 1)])
        .expect("in grid");
    let target = copied.cell(1, 1).expect("target");
    assert_eq!(target.text_height, 4.0);
    assert_eq!(target.style_name, "Title");
    assert_eq!(target.style, StyleId(7));
    assert_eq!(target.borders.top, TableBorder::HIDDEN);
    // Text and position are untouched by a style copy.
    assert_eq!(target.text, "target text");
    assert_eq!(copied.cell(0, 1).unwrap().text, "");
    // The source is unchanged.
    assert_eq!(copied.cell(0, 0).unwrap().text, "source");
}

#[test]
fn a_cell_style_copy_needs_targets_and_in_grid_addresses() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let error = table.with_cell_styles((0, 0), &[]).expect_err("no targets");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.matchcell_no_targets")),
        "{error:?}"
    );
    let error = table
        .with_cell_styles((0, 0), &[(0, 0), (9, 9)])
        .expect_err("out of range");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.cell_out_of_range")),
        "{error:?}"
    );
    // A refused batch leaves the receiver intact.
    assert_eq!(table, TableGeometry::new(grid(2, 2)).unwrap());
}

// ---------------------------------------------------------------------------
// TABLESTYLE (the style snapshot and the cells that inherit it).
// ---------------------------------------------------------------------------

fn resolve_none(_name: &str) -> (StyleId, Option<String>) {
    (StyleId(0), None)
}

#[test]
fn a_style_edit_re_resolves_inheriting_cells_only() {
    let mut parts = grid(2, 2);
    // The bottom-right cell carries its own override.
    parts.rows[1].cells[1].text_height = 9.0;
    parts.rows[1].cells[1].style_name = "Own".into();
    let table = TableGeometry::new(parts).expect("valid table");
    assert_eq!(table.style().text_height, DEFAULT_CELL_TEXT_HEIGHT);

    let mut style = table.style().clone();
    style.name = "Bold".into();
    style.text_height = 5.0;
    style.borders.left = TableBorder::HIDDEN;
    let styled = table
        .with_style_resolved(style, resolve_none)
        .expect("valid");
    assert_eq!(styled.style_name(), "Bold");
    // The inheriting cells follow the style...
    assert_eq!(styled.cell(0, 0).unwrap().text_height, 5.0);
    assert_eq!(styled.cell(0, 0).unwrap().borders.left, TableBorder::HIDDEN);
    // ...and the explicit cell keeps its own values.
    assert_eq!(styled.cell(1, 1).unwrap().text_height, 9.0);
    assert_eq!(styled.cell(1, 1).unwrap().style_name, "Own");
}

#[test]
fn a_style_name_change_resolves_the_text_style_id_through_the_caller() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let mut style = table.style().clone();
    style.text_style_name = "RomanS".into();
    let styled = table
        .with_style_resolved(style, |name| {
            assert_eq!(name, "RomanS");
            (StyleId(3), Some("romans.shx".into()))
        })
        .expect("valid");
    let cell = styled.cell(0, 0).unwrap();
    assert_eq!(cell.style_name, "RomanS");
    assert_eq!(cell.style, StyleId(3));
    assert_eq!(cell.font.as_deref(), Some("romans.shx"));
}

#[test]
fn an_invalid_style_snapshot_is_refused() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let mut style = table.style().clone();
    style.text_height = 0.0;
    let error = table
        .with_style_resolved(style, resolve_none)
        .expect_err("zero height");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.style_text_height")),
        "{error:?}"
    );
    let mut style = table.style().clone();
    style.horizontal_margin = -1.0;
    assert!(table.with_style_resolved(style, resolve_none).is_err());
    let mut style = table.style().clone();
    style.name = "  ".into();
    assert!(table.with_style_resolved(style, resolve_none).is_err());
}

#[test]
fn a_transform_scales_the_style_lengths_with_the_grid() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let scale = Transform3::scale(3.0);
    let scaled = table.transformed(&scale).expect("uniform scale");
    assert!((scaled.style().text_height - DEFAULT_CELL_TEXT_HEIGHT * 3.0).abs() < 1e-12);
    assert!((scaled.style().horizontal_margin - DEFAULT_TABLE_MARGIN * 3.0).abs() < 1e-12);
}

// ---------------------------------------------------------------------------
// TABLEEXPORT (CSV serialization).
// ---------------------------------------------------------------------------

#[test]
fn csv_writes_one_record_per_row_and_one_field_per_column() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].text = "id".into();
    parts.rows[0].cells[1].text = "name".into();
    parts.rows[1].cells[0].text = "1".into();
    parts.rows[1].cells[1].text = "bolt".into();
    let table = TableGeometry::new(parts).expect("valid table");
    assert_eq!(table.to_csv(), "id,name\r\n1,bolt\r\n");
}

#[test]
fn csv_quotes_only_the_fields_that_need_it() {
    let mut parts = grid(1, 3);
    parts.rows[0].cells[0].text = "a,b".into();
    parts.rows[0].cells[1].text = "say \"hi\"".into();
    parts.rows[0].cells[2].text = "plain".into();
    let table = TableGeometry::new(parts).expect("valid table");
    assert_eq!(table.to_csv(), "\"a,b\",\"say \"\"hi\"\"\",plain\r\n");
}

#[test]
fn csv_puts_a_merged_anchors_text_in_its_first_field_and_empties_the_rest() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].text = "merged".into();
    parts.rows[0].cells[1].text = "covered".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let merged = table.with_merge(0, 0, 1, 2).expect("in grid");
    assert_eq!(merged.to_csv(), "merged,\r\n,\r\n");
}

#[test]
fn csv_of_an_empty_grid_is_empty() {
    let mut parts = grid(1, 1);
    parts.rows[0].cells[0].text = String::new();
    let table = TableGeometry::new(parts).expect("valid table");
    assert_eq!(table.to_csv(), "\r\n");
}

#[test]
fn a_style_name_only_write_keeps_the_cells() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let renamed = table.with_style_name("Header").expect("valid name");
    assert_eq!(renamed.style_name(), "Header");
    assert_eq!(renamed.cell(0, 0).unwrap(), table.cell(0, 0).unwrap());
}

// ---------------------------------------------------------------------------
// Per-cell formatting (alignment + rotation).
// ---------------------------------------------------------------------------

/// The one `Text` child a single-cell table draws, with its anchor.
fn drawn_text(table: &TableGeometry) -> (Point3, f64, TextAlignH, TextAlignV) {
    table
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text {
                position,
                rotation,
                h_align,
                v_align,
                ..
            } => Some((*position, *rotation, *h_align, *v_align)),
            _ => None,
        })
        .expect("a cell text")
}

#[test]
fn the_default_cell_format_is_the_pre_formatting_display() {
    let mut parts = grid(1, 1);
    parts.rows[0].cells[0].text = "x".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let cell = table.cell(0, 0).unwrap();
    assert_eq!(cell.alignment, TableCellAlignment::DEFAULT);
    assert_eq!(cell.alignment.dxf_code(), 5);
    assert_eq!(cell.rotation, 0.0);
    // Centre/middle of the 4 x 3 cell, rotation 0, centred justification: the
    // pre-formatting output.
    let (position, rotation, h_align, v_align) = drawn_text(&table);
    assert!((position.x - 2.0).abs() < 1e-12, "{position:?}");
    assert!((position.y - 1.5).abs() < 1e-12, "{position:?}");
    assert_eq!(rotation, 0.0);
    assert_eq!(h_align, TextAlignH::Center);
    assert_eq!(v_align, TextAlignV::Middle);
}

#[test]
fn a_cell_alignment_moves_the_anchor_and_the_justification() {
    let mut parts = grid(1, 1);
    parts.rows[0].cells[0].text = "x".into();
    let table = TableGeometry::new(parts).expect("valid table");

    let left_top = TableCellAlignment {
        horizontal: TableCellAlignH::Left,
        vertical: TableCellAlignV::Top,
    };
    let top_left = table
        .with_cell_format(0, 0, Some(left_top), None)
        .expect("in range");
    let (position, _, h_align, v_align) = drawn_text(&top_left);
    assert_eq!(h_align, TextAlignH::Left);
    assert_eq!(v_align, TextAlignV::Top);
    assert!(position.x.abs() < 1e-12, "{position:?}");
    assert!((position.y - 3.0).abs() < 1e-12, "{position:?}");

    let bottom_right = TableCellAlignment {
        horizontal: TableCellAlignH::Right,
        vertical: TableCellAlignV::Bottom,
    };
    let right_bottom = table
        .with_cell_format(0, 0, Some(bottom_right), None)
        .expect("in range");
    let (position, _, h_align, v_align) = drawn_text(&right_bottom);
    assert_eq!(h_align, TextAlignH::Right);
    assert_eq!(v_align, TextAlignV::Bottom);
    assert!((position.x - 4.0).abs() < 1e-12, "{position:?}");
    assert!(position.y.abs() < 1e-12, "{position:?}");
}

#[test]
fn a_cell_rotation_is_added_to_the_frame_rotation() {
    let mut parts = grid(1, 1);
    parts.rows[0].cells[0].text = "x".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let rotated = table
        .with_cell_format(0, 0, None, Some(std::f64::consts::FRAC_PI_6))
        .expect("in range");
    let (_, rotation, _, _) = drawn_text(&rotated);
    assert!((rotation - std::f64::consts::FRAC_PI_6).abs() < 1e-12);

    // The frame's own rotation still adds on top of the cell's.
    let turn = Transform3::from_basis(
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
    let moved = rotated.transformed(&turn).expect("a rotation");
    let (_, rotation, _, _) = drawn_text(&moved);
    let expected = std::f64::consts::FRAC_PI_2 + std::f64::consts::FRAC_PI_6;
    assert!((rotation - expected).abs() < 1e-12, "{rotation}");
    assert_eq!(
        moved.cell(0, 0).unwrap().rotation,
        rotated.cell(0, 0).unwrap().rotation
    );
}

#[test]
fn a_cell_format_write_refuses_out_of_range_and_non_finite() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let error = table
        .with_cell_format(9, 0, Some(TableCellAlignment::DEFAULT), None)
        .expect_err("out of range");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.cell_out_of_range")),
        "{error:?}"
    );
    let error = table
        .with_cell_format(0, 0, None, Some(f64::NAN))
        .expect_err("non-finite rotation");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.starts_with("table.cell_rotation")),
        "{error:?}"
    );
    // A refused write leaves the receiver untouched.
    assert_eq!(table, TableGeometry::new(grid(2, 2)).unwrap());
    // A structural cell rotation must also be refused by the creation path.
    let mut parts = grid(1, 1);
    parts.rows[0].cells[0].rotation = f64::INFINITY;
    assert!(TableGeometry::new(parts).is_err());
}

#[test]
fn an_alignment_code_round_trips_through_its_label() {
    for code in 1..=9 {
        let alignment = TableCellAlignment::from_dxf_code(code);
        assert_eq!(alignment.dxf_code(), code);
        assert_eq!(
            TableCellAlignment::from_label(alignment.label()),
            Some(alignment)
        );
    }
    // 0 is the value a style with no group 170 carries, and 10 is out of range:
    // both are the DXF default, never an invented corner.
    assert_eq!(
        TableCellAlignment::from_dxf_code(0),
        TableCellAlignment::DEFAULT
    );
    assert_eq!(
        TableCellAlignment::from_dxf_code(10),
        TableCellAlignment::DEFAULT
    );
    assert_eq!(TableCellAlignment::from_label("middle centre"), None);
}

#[test]
fn a_cell_style_copy_moves_alignment_and_rotation_too() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].alignment = TableCellAlignment {
        horizontal: TableCellAlignH::Right,
        vertical: TableCellAlignV::Top,
    };
    parts.rows[0].cells[0].rotation = 0.25;
    parts.rows[1].cells[1].text = "target".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let copied = table.with_cell_styles((0, 0), &[(1, 1)]).expect("in grid");
    let target = copied.cell(1, 1).expect("target");
    assert_eq!(target.alignment, table.cell(0, 0).unwrap().alignment);
    assert_eq!(target.rotation, 0.25);
    assert_eq!(target.text, "target", "a format copy keeps the text");
}

// ---------------------------------------------------------------------------
// Row value sets (data / title / header) and their inheritance.
// ---------------------------------------------------------------------------

#[test]
fn the_three_row_value_sets_default_to_the_data_row() {
    let style = TableStyleValues::standard();
    assert_eq!(style.title_row, None);
    assert_eq!(style.header_row, None);
    for row in 0..3 {
        assert_eq!(style.row_style(style.row_kind(row, 3)), style.data_row());
    }
    // With both row types shown, row 0 is the title and row 1 the header.
    assert_eq!(style.row_kind(0, 3), TableRowKind::Title);
    assert_eq!(style.row_kind(1, 3), TableRowKind::Header);
    assert_eq!(style.row_kind(2, 3), TableRowKind::Data);
    // Suppressing the title moves the header up and leaves one data row.
    let mut suppressed = style.clone();
    suppressed.title_suppressed = true;
    assert_eq!(suppressed.row_kind(0, 3), TableRowKind::Header);
    assert_eq!(suppressed.row_kind(1, 3), TableRowKind::Data);
    // A one-row table has no header row to classify.
    assert_eq!(suppressed.row_kind(0, 1), TableRowKind::Header);
    let mut no_rows = style.clone();
    no_rows.header_suppressed = true;
    assert_eq!(no_rows.row_kind(1, 3), TableRowKind::Data);
}

#[test]
fn a_row_value_set_equal_to_the_data_row_stays_linked() {
    let mut style = TableStyleValues::standard();
    let mut title = style.data_row();
    title.text_height = 6.0;
    style.set_row_style(TableRowKind::Title, title.clone());
    assert_eq!(style.title_row.as_ref(), Some(&title));
    // Writing the same values back as the data row re-links the title row.
    style.set_row_style(TableRowKind::Data, title);
    assert_eq!(style.title_row, None);
    assert_eq!(style.text_height, 6.0);
    // A data-row write also reloads the data-row alignment.
    let mut row = style.data_row();
    row.alignment = TableCellAlignment {
        horizontal: TableCellAlignH::Left,
        vertical: TableCellAlignV::Bottom,
    };
    style.set_row_style(TableRowKind::Data, row);
    assert_eq!(
        style.data_row_alignment,
        TableCellAlignment {
            horizontal: TableCellAlignH::Left,
            vertical: TableCellAlignV::Bottom,
        }
    );
}

#[test]
fn a_title_row_style_edit_re_resolves_only_its_own_rows() {
    let mut parts = grid(3, 1);
    parts.rows[0].cells[0].text = "title".into();
    parts.rows[1].cells[0].text = "header".into();
    parts.rows[2].cells[0].text = "body".into();
    let table = TableGeometry::new(parts).expect("valid table");

    let mut style = table.style().clone();
    let mut title = style.row_style(TableRowKind::Title);
    title.text_height = 6.0;
    title.alignment = TableCellAlignment {
        horizontal: TableCellAlignH::Center,
        vertical: TableCellAlignV::Top,
    };
    style.set_row_style(TableRowKind::Title, title);
    let styled = table
        .with_style_resolved(style, resolve_none)
        .expect("valid");

    assert_eq!(styled.cell(0, 0).unwrap().text_height, 6.0);
    assert_eq!(
        styled.cell(0, 0).unwrap().alignment.vertical,
        TableCellAlignV::Top
    );
    // The header and data rows still inherit the data row.
    assert_eq!(
        styled.cell(1, 0).unwrap().text_height,
        DEFAULT_CELL_TEXT_HEIGHT
    );
    assert_eq!(
        styled.cell(1, 0).unwrap().alignment,
        TableCellAlignment::DEFAULT
    );
    assert_eq!(
        styled.cell(2, 0).unwrap().text_height,
        DEFAULT_CELL_TEXT_HEIGHT
    );
    // ...and the drawn title text really moved to the top of its cell.
    let (position, _, _, v_align) = drawn_text(&styled);
    assert_eq!(v_align, TextAlignV::Top);
    assert!((position.y - 3.0).abs() < 1e-12, "{position:?}");
}

#[test]
fn a_data_row_value_set_still_reaches_the_title_and_header_rows() {
    // The pre-row-style behaviour: with no distinguished row sets, every row
    // inherits the data row, so one `TABLESTYLE HEIGHT` write reaches all rows.
    let table = TableGeometry::new(grid(3, 2)).expect("valid table");
    let mut style = table.style().clone();
    style.text_height = 4.5;
    let styled = table
        .with_style_resolved(style, resolve_none)
        .expect("valid");
    for row in 0..3 {
        for column in 0..2 {
            assert_eq!(
                styled.cell(row, column).unwrap().text_height,
                4.5,
                "({row}, {column})"
            );
        }
    }
}

#[test]
fn an_explicit_alignment_override_survives_a_style_edit() {
    let mut parts = grid(2, 1);
    let own = TableCellAlignment {
        horizontal: TableCellAlignH::Right,
        vertical: TableCellAlignV::Bottom,
    };
    parts.rows[1].cells[0].alignment = own;
    let table = TableGeometry::new(parts).expect("valid table");
    let mut style = table.style().clone();
    let mut data = style.data_row();
    data.alignment = TableCellAlignment {
        horizontal: TableCellAlignH::Left,
        vertical: TableCellAlignV::Top,
    };
    style.set_row_style(TableRowKind::Data, data);
    let styled = table
        .with_style_resolved(style, resolve_none)
        .expect("valid");
    assert_eq!(
        styled.cell(0, 0).unwrap().alignment,
        TableCellAlignment {
            horizontal: TableCellAlignH::Left,
            vertical: TableCellAlignV::Top,
        },
        "the inheriting cell follows the style"
    );
    assert_eq!(
        styled.cell(1, 0).unwrap().alignment,
        own,
        "the explicitly formatted cell keeps its own alignment"
    );
}

#[test]
fn a_transform_scales_every_row_value_set() {
    let mut style = TableStyleValues::standard();
    let mut title = style.data_row();
    title.text_height = 6.0;
    style.set_row_style(TableRowKind::Title, title);
    let table = TableGeometry::from_import_with_style(grid(2, 1), style);
    let scaled = table.transformed(&Transform3::scale(2.0)).expect("uniform");
    assert!((scaled.style().text_height - DEFAULT_CELL_TEXT_HEIGHT * 2.0).abs() < 1e-12);
    assert!((scaled.style().title_row.as_ref().unwrap().text_height - 12.0).abs() < 1e-12);
}
