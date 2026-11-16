//! The per-primitive appearance channel: defaults, inheritance, the display
//! primitives it produces, and the DXF-unit limits the model refuses.
//!
//! Split out of `tests.rs` for the code-size ceiling.

use super::tests_support::*;
use super::*;
use crate::{TableDisplayAppearance, TableFieldEdit};

const RED: [u8; 3] = [255, 0, 0];
const YELLOW: [u8; 3] = [255, 255, 0];

/// Every appearance override, applied to one cell.
fn full_appearance() -> TableCellAppearanceEdit {
    TableCellAppearanceEdit {
        text_color: TableFieldEdit::Set(RED),
        fill: TableFieldEdit::Set(YELLOW),
        border_color: TableFieldEdit::Set([0, 0, 255]),
        border_lineweight: TableFieldEdit::Set(50),
    }
}

#[test]
fn a_fresh_cell_and_row_carry_no_appearance_override() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let cell = table.cell(0, 0).expect("cell");
    assert_eq!(cell.text_color, None);
    assert_eq!(cell.fill_color, None);
    for border in [
        cell.borders.top,
        cell.borders.right,
        cell.borders.bottom,
        cell.borders.left,
    ] {
        assert_eq!(border.color, None);
        assert_eq!(border.lineweight_hundredths, None);
    }
    // The style snapshot's data row defaults to "no override" too, so a table
    // that never sets a colour keeps drawing in the entity's resolved colour.
    let style = table.style();
    assert_eq!(style.text_color, None);
    assert_eq!(style.fill_color, None);
    let row = TableRowStyle::default();
    assert_eq!(row.text_color, None);
    assert_eq!(row.fill_color, None);
    assert_eq!(row.borders.top, TableBorder::VISIBLE);
    // Every derived primitive of such a table carries the neutral appearance,
    // and the two lists stay the same length.
    assert_eq!(table.display().len(), table.display_appearances().len());
    assert!(table
        .display_appearances()
        .iter()
        .all(|appearance| *appearance == TableDisplayAppearance::NONE));
}

#[test]
fn appearance_never_changes_the_derived_geometry() {
    let table = TableGeometry::new(grid(2, 2)).expect("valid table");
    let geometry = table.display().to_vec();
    // Colour and lineweight only: no fill, so not a single primitive is added or
    // moved, which is what "None keeps the existing output byte-identical" means
    // at the geometry level.
    let appearance = TableCellAppearanceEdit {
        text_color: TableFieldEdit::Set(RED),
        fill: TableFieldEdit::Keep,
        border_color: TableFieldEdit::Set([0, 0, 255]),
        border_lineweight: TableFieldEdit::Set(50),
    };
    let styled = table
        .with_cell_appearance(0, 0, appearance)
        .expect("in grid");
    assert_eq!(styled.display(), geometry.as_slice());
    assert_eq!(
        styled.display().len(),
        styled.display_appearances().len(),
        "the appearance list is parallel to the geometry list"
    );
    // Clearing every override returns the neutral appearance exactly.
    let cleared = styled
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                text_color: TableFieldEdit::Clear,
                fill: TableFieldEdit::Clear,
                border_color: TableFieldEdit::Clear,
                border_lineweight: TableFieldEdit::Clear,
            },
        )
        .expect("in grid");
    assert!(cleared
        .display_appearances()
        .iter()
        .all(|appearance| *appearance == TableDisplayAppearance::NONE));
    assert_eq!(cleared.cell(0, 0).unwrap().text_color, None);
}

#[test]
fn a_fill_emits_a_filled_quad_behind_the_cells_borders() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let filled = table
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                fill: TableFieldEdit::Set(YELLOW),
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect("in grid");
    // One extra primitive: the quad.
    assert_eq!(filled.display().len(), table.display().len() + 1);
    let first = &filled.display()[0];
    let appearance = filled.display_appearances()[0];
    assert_eq!(appearance.color, Some(YELLOW));
    let SemanticGeometry::Mesh(mesh) = first else {
        panic!("the background must be a filled mesh, got {first:?}");
    };
    assert_eq!(mesh.vertices.len(), 4, "one quad");
    assert_eq!(mesh.triangles.len(), 2, "two triangles");
    // The quad covers the cell (4 x 3 units) on the table plane.
    let span_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max)
        - mesh
            .vertices
            .iter()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
    let span_y = mesh
        .vertices
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max)
        - mesh
            .vertices
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
    assert!((span_x - 4.0).abs() < 1e-9, "{span_x}");
    assert!((span_y - 3.0).abs() < 1e-9, "{span_y}");
    // The borders follow the fill, so they stay visible on top of it.
    assert!(filled.display()[1..]
        .iter()
        .all(|child| matches!(child, SemanticGeometry::Line { .. })));
}

#[test]
fn the_cell_colour_and_lineweight_reach_their_own_primitives() {
    let mut parts = grid(1, 2);
    parts.rows[0].cells[0].text = "A".into();
    let table = TableGeometry::new(parts).expect("valid table");
    let styled = table
        .with_cell_appearance(0, 0, full_appearance())
        .expect("in grid");
    // Emission order: the styled cell's fill, its four borders, its text, then
    // the (untouched) neighbour's four borders. Only the styled primitives carry
    // an override.
    let appearances: Vec<TableDisplayAppearance> = styled.display_appearances().to_vec();
    assert_eq!(appearances[0].color, Some(YELLOW), "the fill quad");
    for appearance in &appearances[1..5] {
        assert_eq!(appearance.color, Some([0, 0, 255]));
        assert_eq!(appearance.lineweight_hundredths, Some(50));
    }
    assert_eq!(appearances[5].color, Some(RED), "the cell text");
    assert_eq!(appearances[5].lineweight_hundredths, None);
    assert!(
        appearances[6..]
            .iter()
            .all(|appearance| *appearance == TableDisplayAppearance::NONE),
        "the neighbour cell keeps the neutral appearance: {appearances:?}"
    );
    assert_eq!(styled.cell(0, 1).unwrap().text_color, None);
    assert_eq!(
        styled.cell(0, 1).unwrap().borders,
        TableCellBorders::default()
    );
}

#[test]
fn a_style_row_supplies_the_appearance_to_inheriting_cells_only() {
    let mut parts = grid(2, 2);
    // The bottom-right cell overrides its own colour and fill.
    parts.rows[1].cells[1].text_color = Some([0, 255, 0]);
    parts.rows[1].cells[1].fill_color = Some([1, 2, 3]);
    let table = TableGeometry::new(parts).expect("valid table");
    let mut style = table.style().clone();
    // A title row value set that differs from the data row in its appearance.
    let mut data = style.data_row();
    data.text_color = Some(RED);
    data.fill_color = Some(YELLOW);
    data.borders.left.color = Some([0, 0, 255]);
    style.set_row_style(TableRowKind::Data, data.clone());
    style.set_row_style(TableRowKind::Title, data);
    let styled = table
        .with_style_resolved(style, resolve_none)
        .expect("valid");
    // The inheriting top-left cell follows the style...
    let inherited = styled.cell(0, 0).unwrap();
    assert_eq!(inherited.text_color, Some(RED));
    assert_eq!(inherited.fill_color, Some(YELLOW));
    assert_eq!(inherited.borders.left.color, Some([0, 0, 255]));
    // ...and the explicitly coloured cell keeps its own values.
    let own = styled.cell(1, 1).unwrap();
    assert_eq!(own.text_color, Some([0, 255, 0]));
    assert_eq!(own.fill_color, Some([1, 2, 3]));
}

#[test]
fn a_cell_style_copy_moves_the_colours_and_the_fill() {
    let mut parts = grid(2, 2);
    parts.rows[0].cells[0].text_color = Some(RED);
    parts.rows[0].cells[0].fill_color = Some(YELLOW);
    parts.rows[0].cells[0].borders.top.lineweight_hundredths = Some(35);
    let table = TableGeometry::new(parts).expect("valid table");
    let copied = table.with_cell_styles((0, 0), &[(1, 1)]).expect("in grid");
    let target = copied.cell(1, 1).unwrap();
    assert_eq!(target.text_color, Some(RED));
    assert_eq!(target.fill_color, Some(YELLOW));
    assert_eq!(target.borders.top.lineweight_hundredths, Some(35));
}

#[test]
fn a_border_lineweight_outside_the_dxf_range_is_refused() {
    let table = TableGeometry::new(grid(1, 1)).expect("valid table");
    let error = table
        .with_cell_appearance(
            0,
            0,
            TableCellAppearanceEdit {
                border_lineweight: TableFieldEdit::Set(
                    crate::table_entity::MAX_BORDER_LINEWEIGHT_HUNDREDTHS + 1,
                ),
                ..TableCellAppearanceEdit::default()
            },
        )
        .expect_err("out of range");
    assert!(
        matches!(error, CadError::InvalidInput(ref message) if message.contains("275-279")),
        "{error:?}"
    );
    // A refused write leaves the table untouched.
    assert_eq!(table, TableGeometry::new(grid(1, 1)).unwrap());
}

#[test]
fn a_similarity_transform_keeps_the_appearance_and_moves_the_fill() {
    let table = TableGeometry::new(grid(1, 1))
        .expect("valid table")
        .with_cell_appearance(0, 0, full_appearance())
        .expect("in grid");
    let scaled = table.transformed(&Transform3::scale(2.0)).expect("uniform");
    let cell = scaled.cell(0, 0).unwrap();
    assert_eq!(cell.text_color, Some(RED));
    assert_eq!(cell.fill_color, Some(YELLOW));
    assert_eq!(cell.borders.top.color, Some([0, 0, 255]));
    // A lineweight is a print width, not geometry: it does not scale.
    assert_eq!(cell.borders.top.lineweight_hundredths, Some(50));
    // The fill quad scales with the cell.
    let SemanticGeometry::Mesh(mesh) = &scaled.display()[0] else {
        panic!("the fill must still be a mesh");
    };
    let span_x = mesh
        .vertices
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max)
        - mesh
            .vertices
            .iter()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
    assert!((span_x - 8.0).abs() < 1e-9, "{span_x}");
}
