//! The derived display of a table: one primitive per drawn edge/text plus a
//! filled quad for a cell background, and the appearance each primitive carries.
//!
//! Split out of `table_entity.rs` for the code-size ceiling. The derivation is
//! still a **verbatim move** of the pre-table-entity importer's cell-grid
//! synthesis: one `Line` per visible cell edge (a double border adds a second,
//! inward-offset line) and one `Text` per anchoring cell with content, in the
//! same row-major order. A cell background fill is the only primitive this build
//! added, and it is emitted **before** the cell's own edges so the borders stay
//! on top of it.
//!
//! Every primitive is emitted with the
//! [`TableDisplayAppearance`](super::TableDisplayAppearance) its cell/edge
//! resolved. A cell that never set a colour/fill/lineweight emits
//! [`TableDisplayAppearance::NONE`] for all of its primitives, so the geometry
//! **and** the appearance of such a table are exactly what they were before the
//! channel existed.
//!
//! # Cell margins
//!
//! A cell's `GRIDFORMAT` margin override insets its text anchor: the text is
//! placed inside the cell rectangle shrunk by the stored margins, so a
//! left-aligned cell starts at `x + left`, a right-aligned one ends at
//! `x + width - right`, and a centred one sits in the middle of the remaining
//! box; the vertical rule is the same with `top`/`bottom`. A cell with **no**
//! margin override ([`TableCellMargins::INHERIT`](super::TableCellMargins))
//! keeps the anchor it always had, so a table without margins derives the
//! pre-margin geometry byte for byte. A margin larger than the cell cannot
//! invert the box: the inset is clamped so the two edges meet at the cell centre
//! at worst, which keeps the text inside its own cell.

use super::{CellMarginTable, TableCellBorders, TableCellMargins, TableFrame, TableRowGeometry};
use crate::dimension_synthesis::{add, length, line_between, normalize, scale};
use crate::{
    Mesh, Point3, SemanticGeometry, TableBorder, TableColumnGeometry, TableDisplayAppearance,
    TextAlignH, TextAlignV, TextLineSpacing, DEFAULT_CELL_TEXT_HEIGHT, DEFAULT_TEXT_WIDTH_FACTOR,
};

/// Similarity tolerance the display uses to decide whether a cell text can carry
/// the frame rotation (a flat `Text` only rotates within world XY).
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// The derived display of one table: the primitives and, in lockstep with them,
/// the appearance each one carries.
///
/// The two lists are pushed together by [`build`], so they cannot drift; every
/// other reader goes through [`TableDisplay::geometry`] or
/// [`TableDisplay::appearance`].
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableDisplay {
    geometry: Vec<SemanticGeometry>,
    appearance: Vec<TableDisplayAppearance>,
}

impl TableDisplay {
    /// Rebuild the pair from a persisted payload.
    ///
    /// The two lists are stored together and must be the same length, so the
    /// payload reader passes both; the invariant is checked by the caller (a
    /// length mismatch is refused there, before anything is built).
    pub(crate) fn from_parts(
        geometry: Vec<SemanticGeometry>,
        appearance: Vec<TableDisplayAppearance>,
    ) -> Self {
        TableDisplay {
            geometry,
            appearance,
        }
    }

    /// The primitives and their appearance, in lockstep, for the payload writer.
    pub(crate) fn parts(&self) -> (&[SemanticGeometry], &[TableDisplayAppearance]) {
        (&self.geometry, &self.appearance)
    }

    fn push(&mut self, geometry: SemanticGeometry, appearance: TableDisplayAppearance) {
        self.geometry.push(geometry);
        self.appearance.push(appearance);
    }

    pub(crate) fn geometry(&self) -> &[SemanticGeometry] {
        &self.geometry
    }

    pub(crate) fn appearance(&self) -> &[TableDisplayAppearance] {
        &self.appearance
    }
}

/// Derive the display from `frame + columns + rows`.
///
/// Row by row, cell by cell; each anchoring cell contributes (in order) its
/// background fill when it has one, its visible edges and then its first-content
/// text. The text anchor, its justification and its rotation come from the
/// cell's own alignment and rotation, whose defaults reproduce the old
/// cell-centre/centred/frame-rotation output.
pub(super) fn build(
    frame: &TableFrame,
    columns: &[TableColumnGeometry],
    rows: &[TableRowGeometry],
    margins: &CellMarginTable,
) -> TableDisplay {
    let mut out = TableDisplay::default();
    if columns.is_empty() || rows.is_empty() {
        return out;
    }
    let vertical = frame.vertical();
    if length(frame.horizontal) < 1e-9 || length(vertical) < 1e-9 {
        return out;
    }
    let corner = |x: f64, y: f64| {
        add(
            frame.origin,
            add(scale(frame.horizontal, x), scale(vertical, y)),
        )
    };
    // Cell text is a flat `Text` child: it can only rotate within world XY. For a
    // +Z plane that is the frame's horizontal direction; a tilted source plane is
    // drawn with world-XY text (rotation 0) and the importer reports that limit.
    let text_rotation = if (frame.normal.z - 1.0).abs() <= SIMILARITY_TOLERANCE {
        frame.horizontal.y.atan2(frame.horizontal.x)
    } else {
        0.0
    };

    // Cells covered by a merge anchor (all but the top-left cell). Their interior
    // edges belong to the merged range and are not drawn. The one coverage rule
    // lives in `edit::merge_coverage`, which clamps a malformed span to the
    // remaining grid: a raw `i32` span near 2^31 must never drive this loop.
    let covered = super::edit::merge_coverage(rows, columns.len());

    let mut y = 0.0;
    for (row_index, row) in rows.iter().enumerate() {
        let height = row.height.max(0.0);
        let mut x = 0.0;
        for (column_index, cell) in row.cells.iter().enumerate() {
            let width = columns
                .get(column_index)
                .map(|column| column.width.max(0.0))
                .unwrap_or(0.0);
            if covered.contains(&(row_index, column_index)) {
                x += width;
                continue;
            }
            // A merge anchor spans its covered columns/rows; clamp the stored
            // span to the remaining grid.
            let span_w_cells = super::edit::span(cell.merge_width, columns.len(), column_index);
            let span_h_cells = super::edit::span(cell.merge_height, rows.len(), row_index);
            let span_w: f64 = (0..span_w_cells)
                .map(|dc| {
                    columns
                        .get(column_index + dc)
                        .map(|column| column.width.max(0.0))
                        .unwrap_or(0.0)
                })
                .sum();
            let span_h: f64 = (0..span_h_cells)
                .map(|dr| {
                    rows.get(row_index + dr)
                        .map(|row| row.height.max(0.0))
                        .unwrap_or(0.0)
                })
                .sum();

            // The background is a filled quad **behind** the cell: emitted first
            // so the borders and text paint over it.
            if let Some(fill) = cell.fill_color {
                let rect = FillRect {
                    x,
                    y,
                    width: span_w,
                    height: span_h,
                };
                push_fill(&mut out, &corner, rect, fill, frame.normal);
            }

            // Each cell contributes the four edges its effective style leaves
            // visible; a double border adds a second parallel line.
            push_cell_borders(&mut out, &corner, x, y, span_w, span_h, &cell.borders);

            if !cell.text.is_empty() {
                // The alignment decides both where the text anchors and which
                // `Text` justification it carries, so the drawn result is the
                // same rectangle a user sees in AutoCAD: a left-aligned cell
                // starts at the cell's left edge, a top-aligned one hangs from
                // its top edge. The default (centre/middle) reproduces the
                // pre-alignment cell centre exactly.
                //
                // A `GRIDFORMAT` margin override shrinks that box first, so the
                // anchor is the aligned edge of the inset rectangle; with no
                // override the box is the cell itself and the anchor is exactly
                // what it was before margins existed.
                let cell_margins = margins
                    .get(&(row_index, column_index))
                    .copied()
                    .unwrap_or(TableCellMargins::INHERIT);
                let (left, right) = inset_pair(cell_margins.left, cell_margins.right, span_w);
                let (bottom, top) = inset_pair(cell_margins.bottom, cell_margins.top, span_h);
                let (anchor_x, h_align) = match cell.alignment.horizontal {
                    super::TableCellAlignH::Left => (x + left, TextAlignH::Left),
                    super::TableCellAlignH::Center => {
                        (x + left + (span_w - left - right) * 0.5, TextAlignH::Center)
                    }
                    super::TableCellAlignH::Right => (x + span_w - right, TextAlignH::Right),
                };
                let (anchor_y, v_align) = match cell.alignment.vertical {
                    super::TableCellAlignV::Top => (y + span_h - top, TextAlignV::Top),
                    super::TableCellAlignV::Middle => (
                        y + bottom + (span_h - bottom - top) * 0.5,
                        TextAlignV::Middle,
                    ),
                    super::TableCellAlignV::Bottom => (y + bottom, TextAlignV::Bottom),
                };
                out.push(
                    SemanticGeometry::Text {
                        text: cell.text.clone(),
                        position: corner(anchor_x, anchor_y),
                        style: cell.style,
                        height: if cell.text_height > 0.0 {
                            cell.text_height
                        } else {
                            DEFAULT_CELL_TEXT_HEIGHT
                        },
                        // The stored rotation is relative to the frame, so it is
                        // added to it here. On a tilted source plane the frame
                        // part is 0 (world XY text, reported by the importer) and
                        // the cell's own part still applies.
                        rotation: text_rotation + cell.rotation,
                        font: cell.font.clone(),
                        h_align,
                        v_align,
                        // Table cell text is placed at a fixed cell anchor; the
                        // cell width is not a text wrap boundary.
                        wrap_width: None,
                        // Table cells do not stretch their text.
                        width_factor: DEFAULT_TEXT_WIDTH_FACTOR,
                        // An ACAD_TABLE cell has no line-spacing group; the
                        // neutral spacing keeps the pre-line-spacing layout.
                        line_spacing: TextLineSpacing::default(),
                    },
                    TableDisplayAppearance::border(cell.text_color, None),
                );
            }
            x += width;
        }
        y += height;
    }
    out
}

/// The two insets of one cell axis, clamped so they cannot cross.
///
/// A margin pair wider than the cell would invert the text box; the clamp keeps
/// the lower edge at or below the upper one (they meet at the centre at worst),
/// so an over-large margin can never place the text outside its own cell. The
/// unset side is `0.0`, which is exactly the pre-margin anchor.
fn inset_pair(low: Option<f64>, high: Option<f64>, extent: f64) -> (f64, f64) {
    let extent = extent.max(0.0);
    let mut low = low
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    let mut high = high
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    if low + high > extent {
        // Shrink both proportionally so their ratio is kept; a zero total is
        // already the no-inset case.
        let total = low + high;
        if total > 0.0 {
            low = low * extent / total;
            high = high * extent / total;
        }
    }
    (low, high)
}

/// One cell rectangle in the table plane, as the fill emitter needs it.
struct FillRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Push the filled quad that covers one cell (a `Mesh`, the filled primitive a
/// solid HATCH already uses: four corners, two triangles, the plane normal).
fn push_fill(
    out: &mut TableDisplay,
    corner: &impl Fn(f64, f64) -> Point3,
    rect: FillRect,
    fill: [u8; 3],
    normal: Point3,
) {
    let FillRect {
        x,
        y,
        width,
        height,
    } = rect;
    if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return;
    }
    let normal = normalize(normal);
    let vertices = vec![
        corner(x, y),
        corner(x + width, y),
        corner(x + width, y + height),
        corner(x, y + height),
    ];
    out.push(
        SemanticGeometry::Mesh(Mesh {
            normals: vec![normal; vertices.len()],
            vertices,
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            face_sources: Vec::new(),
            // No per-vertex colours: the fill is one flat colour and the mesh
            // pipeline multiplies the batch tint (the appearance colour) into
            // white vertices, exactly like a single-colour solid HATCH.
            colors: Vec::new(),
            // A synthesized display primitive has no texture parameterisation.
            uvs: Vec::new(),
            texture: None,
        }),
        TableDisplayAppearance::colored(fill),
    );
}

/// Emit the four edges of one cell rectangle for the resolved style.
///
/// An edge is drawn only when it is visible; a double border adds a second
/// parallel line offset toward the cell interior by its spacing. `corner` maps
/// the cell's local `(u, v)` plane onto world space.
fn push_cell_borders(
    out: &mut TableDisplay,
    corner: &impl Fn(f64, f64) -> Point3,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    borders: &TableCellBorders,
) {
    let bottom = borders.bottom;
    if bottom.visible {
        push_edge(out, corner(x, y), corner(x + width, y), bottom);
        if bottom.double_spacing > 0.0 {
            let s = bottom.double_spacing;
            push_edge(out, corner(x, y + s), corner(x + width, y + s), bottom);
        }
    }
    let top = borders.top;
    if top.visible {
        push_edge(
            out,
            corner(x, y + height),
            corner(x + width, y + height),
            top,
        );
        if top.double_spacing > 0.0 {
            let s = top.double_spacing;
            push_edge(
                out,
                corner(x, y + height - s),
                corner(x + width, y + height - s),
                top,
            );
        }
    }
    let left = borders.left;
    if left.visible {
        push_edge(out, corner(x, y), corner(x, y + height), left);
        if left.double_spacing > 0.0 {
            let s = left.double_spacing;
            push_edge(out, corner(x + s, y), corner(x + s, y + height), left);
        }
    }
    let right = borders.right;
    if right.visible {
        push_edge(
            out,
            corner(x + width, y),
            corner(x + width, y + height),
            right,
        );
        if right.double_spacing > 0.0 {
            let s = right.double_spacing;
            push_edge(
                out,
                corner(x + width - s, y),
                corner(x + width - s, y + height),
                right,
            );
        }
    }
}

/// Push one non-degenerate border segment carrying its edge's appearance.
fn push_edge(out: &mut TableDisplay, start: Point3, end: Point3, border: TableBorder) {
    if let Some(line) = line_between(start, end) {
        out.push(
            line,
            TableDisplayAppearance::border(border.color, border.lineweight_hundredths),
        );
    }
}
