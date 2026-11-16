//! The per-cell `GRIDFORMAT` margin channel: defaults, structural refusals, the
//! text anchor it moves, and the "unset means inherit" identity.
//!
//! Split out of `tests.rs` for the code-size ceiling, like
//! [`appearance_tests`](super::appearance_tests).

use super::tests_support::*;
use super::*;
use crate::{Point3, SemanticGeometry, TableCellAppearanceEdit, TableFieldEdit, Transform3};

/// The `Text` primitive of cell `(row, column)`'s text, taken from the derived
/// display (the display is the only authority on where a cell's text is drawn).
fn text_position(table: &TableGeometry, text: &str) -> Point3 {
    for child in table.display() {
        if let SemanticGeometry::Text {
            text: drawn,
            position,
            ..
        } = child
        {
            if drawn == text {
                return *position;
            }
        }
    }
    panic!("no Text primitive for `{text}`: {:?}", table.display());
}

fn with_margins(margins: TableCellMargins) -> TableGeometry {
    let table = TableGeometry::new(parts()).expect("valid table");
    table
        .with_cell_margins(
            0,
            0,
            TableCellMarginsEdit {
                left: edit(margins.left),
                top: edit(margins.top),
                right: edit(margins.right),
                bottom: edit(margins.bottom),
            },
        )
        .expect("in grid")
}

/// `Some(value)` writes it, `None` leaves the field alone.
fn edit(value: Option<f64>) -> TableFieldEdit<f64> {
    match value {
        Some(value) => TableFieldEdit::Set(value),
        None => TableFieldEdit::Keep,
    }
}

#[test]
fn a_fresh_table_stores_no_margin_override() {
    let table = TableGeometry::new(parts()).expect("valid table");
    assert_eq!(
        table.cell_margins(0, 0),
        Some(TableCellMargins::INHERIT),
        "a cell that sets no margin inherits"
    );
    assert_eq!(
        table.margin_overrides().count(),
        0,
        "the stored override table is sparse"
    );
    // Out of the grid is `None` (no such cell) rather than `INHERIT`.
    assert_eq!(table.cell_margins(9, 9), None);
    // The default margin is the neutral value, and a table that never sets one
    // draws exactly the pre-margin anchor: the cell centre.
    assert!(TableCellMargins::default().is_inherit());
    let centre = text_position(&table, "id");
    assert!((centre.x - 2.0).abs() < 1e-9);
    assert!((centre.y - 1.5).abs() < 1e-9);
}

#[test]
fn margins_never_change_the_geometry_until_one_is_set() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let before = table.display().to_vec();
    // Explicitly inheriting every edge is the "never set" state: the sparse table
    // stores no entry and the display is byte-identical.
    let cleared = table
        .with_cell_margins(0, 0, TableCellMarginsEdit::CLEAR)
        .expect("in grid");
    assert_eq!(cleared.margin_overrides().count(), 0);
    assert_eq!(cleared.display(), before.as_slice());

    // A write that leaves every field alone is also a no-op on the display.
    let kept = table
        .with_cell_margins(
            0,
            0,
            TableCellMarginsEdit {
                left: TableFieldEdit::Keep,
                ..TableCellMarginsEdit::default()
            },
        )
        .expect("in grid");
    assert_eq!(kept.display(), before.as_slice());
}

#[test]
fn a_left_margin_moves_a_centred_text_by_half_the_remaining_box() {
    // Cell 0,0 is 4 units wide with the text centred at x = 2.0.
    let table = with_margins(TableCellMargins::new(1.0, 0.0, 0.0, 0.0));
    let position = text_position(&table, "id");
    assert!(
        (position.x - 2.5).abs() < 1e-9,
        "1.0 + (4.0 - 1.0) / 2 = 2.5, got {position:?}"
    );
    // The vertical axis is untouched.
    assert!((position.y - 1.5).abs() < 1e-9);
}

#[test]
fn a_right_margin_moves_a_right_aligned_text_inward() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let right_aligned = table
        .with_cell_format(
            0,
            0,
            Some(TableCellAlignment {
                horizontal: TableCellAlignH::Right,
                vertical: TableCellAlignV::Middle,
            }),
            None,
        )
        .expect("in grid");
    assert!((text_position(&right_aligned, "id").x - 4.0).abs() < 1e-9);
    let inset = right_aligned
        .with_cell_margins(
            0,
            0,
            TableCellMarginsEdit {
                right: TableFieldEdit::Set(1.25),
                ..TableCellMarginsEdit::default()
            },
        )
        .expect("in grid");
    assert!((text_position(&inset, "id").x - 2.75).abs() < 1e-9);
}

#[test]
fn a_top_margin_moves_a_top_aligned_text_inward() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let top_aligned = table
        .with_cell_format(
            0,
            0,
            Some(TableCellAlignment {
                horizontal: TableCellAlignH::Center,
                vertical: TableCellAlignV::Top,
            }),
            None,
        )
        .expect("in grid");
    assert!((text_position(&top_aligned, "id").y - 3.0).abs() < 1e-9);
    let bottom_aligned = table
        .with_cell_format(
            0,
            0,
            Some(TableCellAlignment {
                horizontal: TableCellAlignH::Center,
                vertical: TableCellAlignV::Bottom,
            }),
            None,
        )
        .expect("in grid");
    let inset = bottom_aligned
        .with_cell_margins(
            0,
            0,
            TableCellMarginsEdit {
                bottom: TableFieldEdit::Set(0.5),
                top: TableFieldEdit::Set(1.0),
                ..TableCellMarginsEdit::default()
            },
        )
        .expect("in grid");
    assert!((text_position(&inset, "id").y - 0.5).abs() < 1e-9);
}

#[test]
fn a_margin_wider_than_the_cell_cannot_invert_the_text_box() {
    // Two 100-unit margins on a 4-unit cell: the clamp keeps the box non-negative
    // (both edges meet at the centre at worst) instead of placing the text
    // outside the cell.
    let table = with_margins(TableCellMargins::new(100.0, 100.0, 100.0, 100.0));
    let position = text_position(&table, "id");
    assert!(
        position.x >= 0.0 && position.x <= 4.0,
        "the anchor stays inside the cell: {position:?}"
    );
    assert!(
        position.y >= 0.0 && position.y <= 3.0,
        "the anchor stays inside the cell: {position:?}"
    );
}

#[test]
fn a_margin_must_be_finite_and_non_negative() {
    let table = TableGeometry::new(parts()).expect("valid table");
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let error = table
            .with_cell_margins(
                0,
                0,
                TableCellMarginsEdit {
                    left: TableFieldEdit::Set(bad),
                    ..TableCellMarginsEdit::default()
                },
            )
            .expect_err("a negative/non-finite margin is refused");
        let message = format!("{error}");
        assert!(
            message.contains("table.cell_margin"),
            "the refusal carries the model's stable code: {message}"
        );
    }
    // A refused write leaves the receiver untouched.
    assert_eq!(table.margin_overrides().count(), 0);
}

#[test]
fn a_margin_outside_the_grid_is_refused_by_address() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let error = table
        .with_cell_margins(5, 5, TableCellMarginsEdit::uniform(1.0))
        .expect_err("out of grid");
    assert!(format!("{error}").contains("table.cell_out_of_range"));
    let error = table
        .with_cell_margins_bulk(vec![((0, 7), TableCellMargins::uniform(1.0))])
        .expect_err("out of grid");
    assert!(format!("{error}").contains("table.cell_out_of_range"));
}

#[test]
fn the_bulk_writer_stores_a_whole_source_set_in_one_write() {
    let table = TableGeometry::new(parts()).expect("valid table");
    let filled = table
        .with_cell_margins_bulk(vec![
            ((0, 0), TableCellMargins::uniform(0.5)),
            ((0, 1), TableCellMargins::new(0.1, 0.2, 0.3, 0.4)),
            ((0, 1), TableCellMargins::INHERIT),
        ])
        .expect("in grid");
    // The explicit inherit entry of the same cell wins (last write), so it is
    // dropped and only the first cell keeps an override.
    assert_eq!(filled.margin_overrides().count(), 1);
    assert_eq!(
        filled.cell_margins(0, 0),
        Some(TableCellMargins::uniform(0.5))
    );
    assert_eq!(
        filled.cell_margins(0, 1),
        Some(TableCellMargins::INHERIT),
        "the later explicit inherit cleared it"
    );
}

#[test]
fn the_transform_scales_the_margins_with_the_table() {
    let table = with_margins(TableCellMargins::uniform(1.0));
    let transform = Transform3 {
        matrix: [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, 2.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let scaled = table.transformed(&transform).expect("uniform scale");
    assert_eq!(
        scaled.cell_margins(0, 0),
        Some(TableCellMargins::uniform(2.0))
    );
    // The anchor scales with the geometry: the cell is 8 units wide, the margin
    // 2, so the centre sits at 2 + (8 - 4) / 2 = 4.
    let position = text_position(&scaled, "id");
    assert!((position.x - 4.0).abs() < 1e-9, "{position:?}");
}

#[test]
fn matchcell_copies_the_margin_override() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let table = table
        .with_cell_margins(0, 0, TableCellMarginsEdit::uniform(0.75))
        .expect("in grid");
    let copied = table
        .with_cell_styles((0, 0), &[(0, 1)])
        .expect("both cells exist");
    assert_eq!(
        copied.cell_margins(0, 1),
        Some(TableCellMargins::uniform(0.75))
    );
    // Copying an inheriting source clears the destination.
    let cleared = copied
        .with_cell_styles((1, 0), &[(0, 1)])
        .expect("both cells exist");
    assert_eq!(cleared.cell_margins(0, 1), Some(TableCellMargins::INHERIT));
}

#[test]
fn the_margin_label_is_the_typed_form() {
    assert_eq!(TableCellMargins::INHERIT.label(), "INHERIT");
    assert_eq!(TableCellMargins::uniform(0.25).label(), "0.25");
    assert_eq!(
        TableCellMargins::new(0.1, 0.2, 0.3, 0.4).label(),
        "0.1/0.2/0.3/0.4"
    );
    // The DXF `CELLMARGIN` order is top, left, bottom, right.
    assert_eq!(
        TableCellMargins::new(1.0, 2.0, 3.0, 4.0).dxf_values(),
        [Some(2.0), Some(1.0), Some(4.0), Some(3.0)]
    );
}

#[test]
fn the_border_type_and_visibility_edits_change_the_drawn_primitives() {
    // A 1 x 1 cell with four visible single edges draws four lines.
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let lines = |table: &TableGeometry| {
        table
            .display()
            .iter()
            .filter(|child| matches!(child, SemanticGeometry::Line { .. }))
            .count()
    };
    assert_eq!(lines(&table), 4);

    // A non-zero spacing **is** the `Double` border type, and the display emits
    // the second parallel line per edge (8 lines for four doubled edges).
    let doubled = table
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                border_spacing: TableFieldEdit::Set(0.5),
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect("in grid");
    assert_eq!(lines(&doubled), 8, "each doubled edge adds one line");
    assert_eq!(
        doubled.cell(0, 0).expect("cell").borders.top.double_spacing,
        0.5
    );

    // Hiding the edges removes them, and clearing the spacing returns the four
    // single lines exactly.
    let hidden = table
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                border_visible: TableFieldEdit::Set(false),
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect("in grid");
    assert_eq!(lines(&hidden), 0);
    let single = doubled
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                border_spacing: TableFieldEdit::Clear,
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect("in grid");
    assert_eq!(lines(&single), 4);
    assert_eq!(
        single.cell(0, 0).expect("cell").borders.top.double_spacing,
        0.0
    );
}
