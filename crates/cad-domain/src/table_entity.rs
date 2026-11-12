//! First-class TABLE (`ACAD_TABLE`) entity: the grid frame, the column/row
//! definition with per-cell content, the resolved borders and the derived
//! display geometry.
//!
//! [`TableGeometry`] is the single builder of a table's display: it is derived
//! from `frame + columns + rows` by [`TableGeometry::build_display`], and the
//! only writers are [`TableGeometry::new`] (creation path),
//! [`TableGeometry::from_import`] (import path, which never rejects),
//! [`TableGeometry::transformed`], [`TableGeometry::with_cell_text`] and
//! [`TableGeometry::with_cell_texts`]. This mirrors
//! `HatchGeometry`/`MultiLeaderGeometry`, so a caller cannot store a display
//! that disagrees with the definition.
//!
//! The display derivation is a **verbatim move** of the pre-table-entity
//! importer's `cad-import-acadrust::entity::table` cell-grid synthesis: one
//! `Line` per visible cell edge (a double border adds a second, inward-offset
//! line) and one `Text` per anchoring cell with content (cell centre, centred
//! horizontally and vertically, rotation 0). The acceptance for this migration
//! is display equivalence with that `Compound` output, not a new rendering.
//!
//! Honest boundary: per-cell border colours, lineweights and background fills
//! are not part of this representation (one colour per entity); the importer
//! reports that gap in its `Completeness::Partial` reasons. A merged range is
//! stored as the anchor cell's `merge_width`/`merge_height`; covered cells are
//! still present in [`TableRowGeometry::cells`] (as the source stores them) and
//! contribute no display.

use crate::dimension_synthesis::{add, is_finite_point, length, line_between, normalize, scale};
use crate::{
    CadError, CadResult, Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV, Transform3,
    DEFAULT_TEXT_WIDTH_FACTOR,
};

/// Similarity tolerance for the table transform (the same ratio rule every
/// other similarity-only display entity uses).
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// The fallback cell text height when neither the cell/row/table style nor the
/// content supplies a positive value.
///
/// Verbatim from the pre-table-entity importer: a missing or non-positive
/// source height becomes 2.5 drawing units. The importer uses this one
/// constant too, so the value has a single definition.
pub const DEFAULT_CELL_TEXT_HEIGHT: f64 = 2.5;

// ---------------------------------------------------------------------------
// The table definition.
// ---------------------------------------------------------------------------

/// The table's plane frame: the DXF insertion point, the horizontal direction
/// (group 11) and the normal (group 210), both stored normalised.
///
/// The vertical direction is always `normalize(cross(normal, horizontal))`, so
/// the frame is right-handed with the same convention the importer used. It is
/// derived, never stored twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableFrame {
    pub origin: Point3,
    /// Unit horizontal direction (the table's +u axis).
    pub horizontal: Point3,
    /// Unit plane normal (the table's +w axis).
    pub normal: Point3,
}

impl Default for TableFrame {
    fn default() -> Self {
        TableFrame {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
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
        }
    }
}

impl TableFrame {
    /// The unit vertical direction `normalize(cross(normal, horizontal))`.
    pub fn vertical(&self) -> Point3 {
        normalize(cross(self.normal, self.horizontal))
    }
}

/// Visibility and double-line spacing of one cell edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableBorder {
    /// Whether the edge is drawn at all (`CellBorder.invisible` inverted).
    pub visible: bool,
    /// Inward offset of the second line of a double border; `0.0` is a single
    /// line.
    pub double_spacing: f64,
}

impl Default for TableBorder {
    fn default() -> Self {
        TableBorder {
            visible: true,
            double_spacing: 0.0,
        }
    }
}

impl TableBorder {
    /// An invisible edge.
    pub const HIDDEN: TableBorder = TableBorder {
        visible: false,
        double_spacing: 0.0,
    };

    /// A visible single edge.
    pub const VISIBLE: TableBorder = TableBorder {
        visible: true,
        double_spacing: 0.0,
    };
}

/// The four resolved edges of one cell's effective style.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TableCellBorders {
    pub top: TableBorder,
    pub right: TableBorder,
    pub bottom: TableBorder,
    pub left: TableBorder,
}

/// One cell: its first content's text, text-style identity and merge span.
///
/// Only the first content is modelled (the importer has always drawn only the
/// first content). `text` is the *effective* string: `formatted_value` when the
/// source has one, otherwise the raw value text; an empty string means "no
/// drawn text" and emits no `Text` child.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCellGeometry {
    pub text: String,
    /// Effective text height the display uses. It must be finite and positive;
    /// the import path substitutes [`DEFAULT_CELL_TEXT_HEIGHT`] when every
    /// source level stores a non-positive value, and [`TableGeometry::new`]
    /// rejects a non-positive height consistently.
    pub text_height: f64,
    /// The effective text style name (DXF group 7 equivalent), resolved from the
    /// cell/row/base/table style and finally the content.
    pub style_name: String,
    /// The resolved text style identity the display `Text` carries.
    ///
    /// Kept alongside `style_name` because the display needs the numeric id
    /// (which only the import's style table can resolve) and the export needs
    /// the name.
    pub style: StyleId,
    /// The style's font resource key, when known.
    pub font: Option<String>,
    /// Number of columns this cell spans (including itself); `>= 1`.
    pub merge_width: u32,
    /// Number of rows this cell spans (including itself); `>= 1`.
    pub merge_height: u32,
    pub borders: TableCellBorders,
}

/// One table row.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowGeometry {
    pub height: f64,
    /// The row's cells, in source column order. A merged range's covered cells
    /// remain present; only the anchor contributes display and text.
    pub cells: Vec<TableCellGeometry>,
}

/// One table column.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumnGeometry {
    pub width: f64,
    pub name: String,
}

/// The parts accepted by [`TableGeometry::from_import`] and
/// [`TableGeometry::new`].
///
/// A named struct keeps the import call readable without a long positional
/// constructor (the clippy `too_many_arguments` limit).
#[derive(Debug, Clone, PartialEq)]
pub struct TableParts {
    pub frame: TableFrame,
    pub columns: Vec<TableColumnGeometry>,
    pub rows: Vec<TableRowGeometry>,
    /// The table style name, when the source names one.
    pub style_name: String,
    /// Linked table-data name (DXF group 1).
    pub name: String,
    /// Linked table-data description.
    pub description: String,
}

/// A first-class TABLE entity.
///
/// All fields are private; the only writers are [`Self::new`],
/// [`Self::from_import`], [`Self::transformed`], [`Self::with_cell_text`] and
/// [`Self::with_cell_texts`]. `display` is derived from the other fields.
#[derive(Debug, Clone, PartialEq)]
pub struct TableGeometry {
    frame: TableFrame,
    columns: Vec<TableColumnGeometry>,
    rows: Vec<TableRowGeometry>,
    style_name: String,
    name: String,
    description: String,
    display: Vec<SemanticGeometry>,
}

impl TableGeometry {
    /// Build a table on the creation path, rejecting a definition whose display
    /// cannot be derived.
    ///
    /// The frame directions are stored normalised. The import path uses
    /// [`Self::from_import`] instead, which keeps the importer's honest
    /// degradation and never rejects. `new` refuses only **structural** defects:
    /// a non-finite or degenerate frame, no column or row, a non-finite or
    /// negative column width/row height, a total extent that is not positive, a
    /// merge span outside the grid or overlapping another merge, a non-finite or
    /// non-positive cell text height, and an empty derived display.
    pub fn new(parts: TableParts) -> CadResult<Self> {
        validate_parts(&parts)?;
        let geometry = Self::from_import(parts);
        if geometry.display.is_empty() {
            return Err(CadError::InvalidInput(
                "table.structure: the definition yields an empty display".into(),
            ));
        }
        Ok(geometry)
    }

    /// Build a table from imported data, degrading honestly and never rejecting.
    ///
    /// The frame directions are stored normalised, and any merge span is clamped
    /// to the remaining grid so malformed source spans cannot allocate or
    /// iterate over billions of covered cells. The returned geometry carries
    /// exactly the display the pre-table-entity importer produced.
    ///
    /// The importer owns the degradation reasons because only it knows whether
    /// any cell/row/table style supplied the resolved borders.
    pub fn from_import(parts: TableParts) -> Self {
        let mut geometry = TableGeometry {
            frame: normalize_frame(parts.frame),
            columns: parts.columns,
            rows: parts.rows,
            style_name: parts.style_name,
            name: parts.name,
            description: parts.description,
            display: Vec::new(),
        };
        geometry.clamp_merge_spans();
        geometry.display = geometry.build_display();
        geometry
    }

    /// Clamp every stored merge span to the remaining grid.
    ///
    /// A source can carry `merge_width`/`merge_height` values near `i32::MAX`;
    /// iterating those literally would hang the import. A span is clipped to the
    /// columns/rows that remain from the anchor, and never below one.
    fn clamp_merge_spans(&mut self) {
        let row_count = self.rows.len();
        let column_count = self.columns.len();
        for (row_index, row) in self.rows.iter_mut().enumerate() {
            for (column_index, cell) in row.cells.iter_mut().enumerate() {
                let max_width = column_count.saturating_sub(column_index).max(1) as u32;
                let max_height = row_count.saturating_sub(row_index).max(1) as u32;
                cell.merge_width = cell.merge_width.clamp(1, max_width);
                cell.merge_height = cell.merge_height.clamp(1, max_height);
            }
        }
    }

    /// Transform by a **positive-determinant similarity** (translation +
    /// rotation + uniform scale) and rebuild the display.
    ///
    /// `origin` moves as a point, `horizontal`/`normal` map through the linear
    /// part and stay normalised, and every stored length (column width, row
    /// height, cell text height, double-border spacing) scales by the uniform
    /// factor. A non-uniform scale/shear or a mirror cannot be represented by
    /// this grid + flat-text arrangement and is refused for the whole entity
    /// with a named `Unsupported`, never silently dropped. A transform that tilts
    /// the plane normal away from +Z is refused for the same reason: a cell text
    /// child can rotate in world XY but cannot represent a tilted plane.
    pub fn transformed(&self, transform: &Transform3) -> CadResult<Self> {
        if !transform.is_uniform_scale(SIMILARITY_TOLERANCE) {
            return Err(CadError::Unsupported(
                "table transform requires a rotation + uniform scale; a non-uniform scale or \
                 shear cannot be represented exactly by the table grid"
                    .into(),
            ));
        }
        if transform.determinant() < 0.0 {
            return Err(CadError::Unsupported(
                "table transform would mirror it; a reflected table is not represented by this \
                 build and is refused rather than dropped"
                    .into(),
            ));
        }
        let s = uniform_scale(transform);
        if !s.is_finite() || s <= 0.0 {
            return Err(CadError::InvalidInput(
                "table transform has a non-finite or non-positive uniform scale".into(),
            ));
        }
        let horizontal = normalize(transform.apply_vector(self.frame.horizontal));
        let normal = normalize(transform.apply_vector(self.frame.normal));
        if length(horizontal) < 1e-9 || length(normal) < 1e-9 {
            return Err(CadError::InvalidInput(
                "table transform collapses the plane frame".into(),
            ));
        }
        // Cell text is a flat `Text` child: it can carry a rotation in the world
        // XY plane but not a tilted plane. A transform that tilts the normal away
        // from +Z is refused for the whole entity instead of silently drawing
        // text in the wrong orientation (the import path reports the same limit
        // when a source table already has a non-+Z normal).
        if (normal.z - 1.0).abs() > SIMILARITY_TOLERANCE {
            return Err(CadError::Unsupported(
                "table transform tilts the plane away from +Z; cell text cannot represent a \
                 tilted plane and the entity is refused rather than drawn with unrotated text"
                    .into(),
            ));
        }
        let frame = TableFrame {
            origin: transform.apply_point(self.frame.origin),
            horizontal,
            normal,
        };
        let columns = self
            .columns
            .iter()
            .map(|column| TableColumnGeometry {
                width: column.width * s,
                name: column.name.clone(),
            })
            .collect();
        let rows = self
            .rows
            .iter()
            .map(|row| TableRowGeometry {
                height: row.height * s,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| TableCellGeometry {
                        text: cell.text.clone(),
                        text_height: cell.text_height * s,
                        style_name: cell.style_name.clone(),
                        style: cell.style,
                        font: cell.font.clone(),
                        merge_width: cell.merge_width,
                        merge_height: cell.merge_height,
                        borders: TableCellBorders {
                            top: scale_border(cell.borders.top, s),
                            right: scale_border(cell.borders.right, s),
                            bottom: scale_border(cell.borders.bottom, s),
                            left: scale_border(cell.borders.left, s),
                        },
                    })
                    .collect(),
            })
            .collect();
        let mut geometry = TableGeometry {
            frame,
            columns,
            rows,
            style_name: self.style_name.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            display: Vec::new(),
        };
        geometry.display = geometry.build_display();
        Ok(geometry)
    }

    /// Replace one cell's text and rebuild the derived display.
    ///
    /// Refuses a row/column outside the grid (`table.cell_out_of_range`). Empty
    /// text is legal: the cell then draws no `Text`.
    pub fn with_cell_text(&self, row: usize, column: usize, text: &str) -> CadResult<Self> {
        self.with_cell_texts(vec![(row, column, text.to_string())])
    }

    /// Replace several cells' texts and rebuild the derived display **once**.
    ///
    /// The importer's FIND path uses this so replacing the same needle in N
    /// cells is one rebuild and one undo step, not N rebuilds. Every
    /// `(row, column)` must be inside the grid (`table.cell_out_of_range`);
    /// on a refusal the receiver is unchanged.
    pub fn with_cell_texts(&self, changes: Vec<(usize, usize, String)>) -> CadResult<Self> {
        let mut geometry = self.clone();
        for (row, column, text) in changes {
            let row_count = geometry.rows.len();
            let row_geometry = geometry.rows.get_mut(row).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "table.cell_out_of_range: row {row} is outside the table's {row_count} rows"
                ))
            })?;
            let cell_count = row_geometry.cells.len();
            let cell = row_geometry.cells.get_mut(column).ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "table.cell_out_of_range: column {column} is outside row {row}'s {cell_count} cells"
                ))
            })?;
            cell.text = text;
        }
        geometry.display = geometry.build_display();
        Ok(geometry)
    }

    pub fn frame(&self) -> TableFrame {
        self.frame
    }

    pub fn columns(&self) -> &[TableColumnGeometry] {
        &self.columns
    }

    pub fn rows(&self) -> &[TableRowGeometry] {
        &self.rows
    }

    pub fn cell(&self, row: usize, column: usize) -> Option<&TableCellGeometry> {
        self.rows.get(row).and_then(|row| row.cells.get(column))
    }

    pub fn style_name(&self) -> &str {
        &self.style_name
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// Every non-empty cell text, in row-major order.
    ///
    /// Covers both merged anchors and covered cells: the stored content is what
    /// FIND and the properties panel report, never a display-derived subset.
    pub fn texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .filter(|cell| !cell.text.is_empty())
            .map(|cell| cell.text.clone())
            .collect()
    }

    /// The derived display: grid lines and cell text.
    pub fn display(&self) -> &[SemanticGeometry] {
        &self.display
    }

    /// Derive the display from `frame + columns + rows`.
    ///
    /// Verbatim from the pre-table-entity importer, in the same order: row by
    /// row, cell by cell; each anchoring cell contributes its visible edges and
    /// then its first-content text.
    fn build_display(&self) -> Vec<SemanticGeometry> {
        let mut children: Vec<SemanticGeometry> = Vec::new();
        if self.columns.is_empty() || self.rows.is_empty() {
            return children;
        }
        let vertical = self.frame.vertical();
        if length(self.frame.horizontal) < 1e-9 || length(vertical) < 1e-9 {
            return children;
        }
        let corner = |x: f64, y: f64| {
            add(
                self.frame.origin,
                add(scale(self.frame.horizontal, x), scale(vertical, y)),
            )
        };
        // Cell text is a flat `Text` child: it can only rotate within world XY.
        // For a +Z plane that is the frame's horizontal direction; a tilted
        // source plane is drawn with world-XY text (rotation 0) and the importer
        // reports that limit.
        let text_rotation = if (self.frame.normal.z - 1.0).abs() <= SIMILARITY_TOLERANCE {
            self.frame.horizontal.y.atan2(self.frame.horizontal.x)
        } else {
            0.0
        };

        // Cells covered by a merge anchor (all but the top-left cell). Their
        // interior edges belong to the merged range and are not drawn. A
        // malformed span is clamped to the remaining grid: a raw `i32` span near
        // 2^31 must never drive this loop.
        let mut covered: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        for (row_index, row) in self.rows.iter().enumerate() {
            for (column_index, cell) in row.cells.iter().enumerate() {
                let span_w =
                    clamped_span(cell.merge_width as usize, self.columns.len(), column_index);
                let span_h = clamped_span(cell.merge_height as usize, self.rows.len(), row_index);
                for dr in 0..span_h {
                    for dc in 0..span_w {
                        if dr != 0 || dc != 0 {
                            covered.insert((row_index + dr, column_index + dc));
                        }
                    }
                }
            }
        }

        let mut y = 0.0;
        for (row_index, row) in self.rows.iter().enumerate() {
            let height = row.height.max(0.0);
            let mut x = 0.0;
            for (column_index, cell) in row.cells.iter().enumerate() {
                let width = self
                    .columns
                    .get(column_index)
                    .map(|column| column.width.max(0.0))
                    .unwrap_or(0.0);
                if covered.contains(&(row_index, column_index)) {
                    x += width;
                    continue;
                }
                // A merge anchor spans its covered columns/rows; clamp the
                // stored span to the remaining grid.
                let span_w_cells =
                    clamped_span(cell.merge_width as usize, self.columns.len(), column_index);
                let span_h_cells =
                    clamped_span(cell.merge_height as usize, self.rows.len(), row_index);
                let span_w: f64 = (0..span_w_cells)
                    .map(|dc| {
                        self.columns
                            .get(column_index + dc)
                            .map(|column| column.width.max(0.0))
                            .unwrap_or(0.0)
                    })
                    .sum();
                let span_h: f64 = (0..span_h_cells)
                    .map(|dr| {
                        self.rows
                            .get(row_index + dr)
                            .map(|row| row.height.max(0.0))
                            .unwrap_or(0.0)
                    })
                    .sum();

                // Each cell contributes the four edges its effective style
                // leaves visible; a double border adds a second parallel line.
                push_cell_borders(&mut children, &corner, x, y, span_w, span_h, &cell.borders);

                if !cell.text.is_empty() {
                    children.push(SemanticGeometry::Text {
                        text: cell.text.clone(),
                        position: corner(x + span_w * 0.5, y + span_h * 0.5),
                        style: cell.style,
                        height: if cell.text_height > 0.0 {
                            cell.text_height
                        } else {
                            DEFAULT_CELL_TEXT_HEIGHT
                        },
                        rotation: text_rotation,
                        font: cell.font.clone(),
                        h_align: TextAlignH::Center,
                        v_align: TextAlignV::Middle,
                        // Table cell text is placed at a fixed cell centre; the
                        // cell width is not a text wrap boundary.
                        wrap_width: None,
                        // Table cells do not stretch their text.
                        width_factor: DEFAULT_TEXT_WIDTH_FACTOR,
                    });
                }
                x += width;
            }
            y += height;
        }
        children
    }
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// Clamp a merge span to the grid cells remaining from `anchor`.
///
/// `span` may be an untrusted source value; the result is always at least one
/// and never points past `extent`.
fn clamped_span(span: usize, extent: usize, anchor: usize) -> usize {
    span.max(1).min(extent.saturating_sub(anchor).max(1))
}

/// Store the frame with unit horizontal/normal directions.
fn normalize_frame(frame: TableFrame) -> TableFrame {
    TableFrame {
        origin: frame.origin,
        horizontal: if length(frame.horizontal) >= 1e-9 {
            normalize(frame.horizontal)
        } else {
            frame.horizontal
        },
        normal: if length(frame.normal) >= 1e-9 {
            normalize(frame.normal)
        } else {
            frame.normal
        },
    }
}

/// Emit the four edges of one cell rectangle for the resolved style.
///
/// An edge is drawn only when it is visible; a double border adds a second
/// parallel line offset toward the cell interior by its spacing. `corner` maps
/// the cell's local `(u, v)` plane onto world space.
fn push_cell_borders(
    children: &mut Vec<SemanticGeometry>,
    corner: &impl Fn(f64, f64) -> Point3,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    borders: &TableCellBorders,
) {
    let bottom = borders.bottom;
    if bottom.visible {
        push_edge(children, corner(x, y), corner(x + width, y));
        if bottom.double_spacing > 0.0 {
            let s = bottom.double_spacing;
            push_edge(children, corner(x, y + s), corner(x + width, y + s));
        }
    }
    let top = borders.top;
    if top.visible {
        push_edge(
            children,
            corner(x, y + height),
            corner(x + width, y + height),
        );
        if top.double_spacing > 0.0 {
            let s = top.double_spacing;
            push_edge(
                children,
                corner(x, y + height - s),
                corner(x + width, y + height - s),
            );
        }
    }
    let left = borders.left;
    if left.visible {
        push_edge(children, corner(x, y), corner(x, y + height));
        if left.double_spacing > 0.0 {
            let s = left.double_spacing;
            push_edge(children, corner(x + s, y), corner(x + s, y + height));
        }
    }
    let right = borders.right;
    if right.visible {
        push_edge(
            children,
            corner(x + width, y),
            corner(x + width, y + height),
        );
        if right.double_spacing > 0.0 {
            let s = right.double_spacing;
            push_edge(
                children,
                corner(x + width - s, y),
                corner(x + width - s, y + height),
            );
        }
    }
}

/// Push a non-degenerate line segment between two points.
fn push_edge(children: &mut Vec<SemanticGeometry>, start: Point3, end: Point3) {
    if let Some(line) = line_between(start, end) {
        children.push(line);
    }
}

fn scale_border(border: TableBorder, factor: f64) -> TableBorder {
    TableBorder {
        visible: border.visible,
        double_spacing: border.double_spacing * factor,
    }
}

fn uniform_scale(transform: &Transform3) -> f64 {
    let m = &transform.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    ((sx + sy + sz) / 3.0).max(1e-12)
}

/// Structural validation shared by [`TableGeometry::new`].
fn validate_parts(parts: &TableParts) -> CadResult<()> {
    let frame = parts.frame;
    if !is_finite_point(frame.origin)
        || !is_finite_point(frame.horizontal)
        || !is_finite_point(frame.normal)
    {
        return Err(CadError::InvalidInput(
            "table.structure: the plane frame contains a non-finite value".into(),
        ));
    }
    if length(frame.horizontal) < 1e-9 || length(frame.normal) < 1e-9 {
        return Err(CadError::InvalidInput(
            "table.structure: the plane frame is degenerate".into(),
        ));
    }
    if parts.columns.is_empty() {
        return Err(CadError::InvalidInput(
            "table.structure: a table needs at least one column".into(),
        ));
    }
    if parts.rows.is_empty() {
        return Err(CadError::InvalidInput(
            "table.structure: a table needs at least one row".into(),
        ));
    }
    let mut total_width = 0.0;
    for column in &parts.columns {
        if !column.width.is_finite() || column.width < 0.0 {
            return Err(CadError::InvalidInput(
                "table.structure: column widths must be finite and non-negative".into(),
            ));
        }
        total_width += column.width;
    }
    let mut total_height = 0.0;
    for row in &parts.rows {
        if !row.height.is_finite() || row.height < 0.0 {
            return Err(CadError::InvalidInput(
                "table.structure: row heights must be finite and non-negative".into(),
            ));
        }
        total_height += row.height;
    }
    if total_width <= 0.0 || total_height <= 0.0 {
        return Err(CadError::InvalidInput(
            "table.structure: the table must have a positive extent".into(),
        ));
    }
    for (row_index, row) in parts.rows.iter().enumerate() {
        for (column_index, cell) in row.cells.iter().enumerate() {
            if column_index >= parts.columns.len() {
                return Err(CadError::InvalidInput(format!(
                    "table.structure: row {row_index} has a cell in column {column_index}, \
                     beyond the table's {} columns",
                    parts.columns.len()
                )));
            }
            let span_w = cell.merge_width.max(1) as usize;
            let span_h = cell.merge_height.max(1) as usize;
            if column_index + span_w > parts.columns.len() || row_index + span_h > parts.rows.len()
            {
                return Err(CadError::InvalidInput(format!(
                    "table.structure: the merge at ({row_index}, {column_index}) spans \
                     {span_w}x{span_h}, outside the {}x{} grid",
                    parts.rows.len(),
                    parts.columns.len()
                )));
            }
            if !cell.text_height.is_finite() || cell.text_height <= 0.0 {
                return Err(CadError::InvalidInput(
                    "table.structure: cell text heights must be finite and positive".into(),
                ));
            }
        }
    }
    // Merge ranges must not overlap. Rows are visited top to bottom and cells
    // left to right, so a cell already inside an earlier anchor is a covered
    // cell (skipped); an anchor that reaches into an already-covered cell is an
    // overlap and is rejected rather than given a silent precedence.
    let mut covered = vec![vec![false; parts.columns.len()]; parts.rows.len()];
    for (row_index, row) in parts.rows.iter().enumerate() {
        for (column_index, cell) in row.cells.iter().enumerate() {
            if covered[row_index][column_index] {
                continue;
            }
            let span_w = cell.merge_width.max(1) as usize;
            let span_h = cell.merge_height.max(1) as usize;
            for dr in 0..span_h {
                for dc in 0..span_w {
                    let (r, c) = (row_index + dr, column_index + dc);
                    if covered[r][c] {
                        return Err(CadError::InvalidInput(format!(
                            "table.structure: the merge at ({row_index}, {column_index}) \
                             overlaps the merge covering ({r}, {c})"
                        )));
                    }
                    covered[r][c] = true;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
