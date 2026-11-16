//! First-class TABLE (`ACAD_TABLE`) entity: the grid frame, the column/row
//! definition with per-cell content, the resolved borders and the derived
//! display geometry.
//!
//! [`TableGeometry`] is the single builder of a table's display: it is derived
//! from `frame + columns + rows` by the private `display::build`, and the
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
//! line) and one `Text` per anchoring cell with content. The acceptance for that
//! migration was display equivalence with the old `Compound` output; the two
//! formatting fields added since — [`TableCellAlignment`] and the per-cell text
//! rotation — default to the old drawing exactly (centre/middle, frame rotation),
//! so a table that never sets them still derives the pre-formatting display.
//!
//! Honest boundary: the display carries a **per-primitive appearance** channel
//! ([`TableDisplayAppearance`]): a cell's text colour, its background fill (a
//! filled quad behind the text) and each border edge's colour and lineweight are
//! stored, drawn and written back to DXF. A cell that sets none of them emits
//! [`TableDisplayAppearance::NONE`] for every primitive, so its draw is exactly
//! what it was before the channel existed. The cell's `GRIDFORMAT` **margins** are
//! a second, sparse channel ([`TableCellMargins`]): an override insets the cell's
//! text anchor, and a cell that sets none draws at the anchor it always had.
//! What still cannot be carried: a **border linetype** (the only DXF group is an
//! `LTYPE` handle, this writer emits no per-cell border line type, and the
//! fragment expansion has no per-primitive dash source — a stored value could
//! neither draw nor round-trip, so the commands refuse it by name), and the
//! renderer draws every line one pixel wide, so a stored border lineweight reaches
//! the batch but no wider stroke (`render.lineweight_not_drawn`). A merged range
//! is stored as the anchor cell's `merge_width`/`merge_height`; covered cells are
//! still present in [`TableRowGeometry::cells`] (as the source stores them) and
//! contribute no display.

use crate::dimension_synthesis::{is_finite_point, length, normalize};
use crate::{CadError, CadResult, Point3, SemanticGeometry, StyleId, Transform3};

pub use appearance::{TableCellAppearanceEdit, TableDisplayAppearance, TableFieldEdit};
pub use format::{
    TableCellAlignH, TableCellAlignV, TableCellAlignment, TableRowKind, TableRowStyle,
};
pub use grid::{TableCellMargins, TableCellMarginsEdit};
pub use style::{TableStyleValues, DEFAULT_TABLE_MARGIN, DEFAULT_TABLE_STYLE_NAME};

use grid::CellMarginTable;

mod appearance;
mod csv;
mod display;
mod edit;
mod format;
mod grid;
#[cfg(test)]
mod grid_tests;
mod style;

/// Similarity tolerance for the table transform (the same ratio rule every
/// other similarity-only display entity uses).
const SIMILARITY_TOLERANCE: f64 = 1e-9;

/// The largest border lineweight a DXF record can carry, in 1/100 mm (group
/// 275-279 range `0..=211`, i.e. 0.00-2.11 mm). A larger stored value would be
/// written back truncated, so the model refuses it instead.
pub const MAX_BORDER_LINEWEIGHT_HUNDREDTHS: u16 = 211;

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

/// Visibility, colour, lineweight and double-line spacing of one cell edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableBorder {
    /// Whether the edge is drawn at all (`CellBorder.invisible` inverted).
    pub visible: bool,
    /// Inward offset of the second line of a double border; `0.0` is a single
    /// line.
    pub double_spacing: f64,
    /// The edge's own sRGB colour (DXF groups 65-69 of a cell record, 64-69 of a
    /// `TABLESTYLE` row record). `None` means "no override": the edge is drawn in
    /// the entity's resolved colour, exactly as every table border was before a
    /// per-edge colour existed. `Some` is stored and written back verbatim.
    pub color: Option<[u8; 3]>,
    /// The edge's own lineweight in **1/100 mm** (DXF groups 275-279, the DXF
    /// unit). `None` keeps the entity's resolved lineweight. The renderer draws
    /// every line one pixel wide, so a stored weight reaches the batch but no
    /// wider stroke (`render.lineweight_not_drawn`).
    pub lineweight_hundredths: Option<u16>,
}

impl Default for TableBorder {
    fn default() -> Self {
        TableBorder {
            visible: true,
            double_spacing: 0.0,
            color: None,
            lineweight_hundredths: None,
        }
    }
}

impl TableBorder {
    /// An invisible edge.
    pub const HIDDEN: TableBorder = TableBorder {
        visible: false,
        double_spacing: 0.0,
        color: None,
        lineweight_hundredths: None,
    };

    /// A visible single edge with no colour/lineweight override.
    pub const VISIBLE: TableBorder = TableBorder {
        visible: true,
        double_spacing: 0.0,
        color: None,
        lineweight_hundredths: None,
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

/// One cell: its first content's text, text-style identity, merge span and the
/// two formatting fields this build can draw.
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
    /// Horizontal/vertical text placement inside the cell (DXF cell-alignment
    /// code, group 170). [`TableCellAlignment::DEFAULT`] is centre/middle: the
    /// placement every cell had before alignment was modelled.
    pub alignment: TableCellAlignment,
    /// Text rotation in radians **relative to the table frame** (DXF group 145
    /// of the cell record). `0.0` is the neutral value and keeps the frame
    /// rotation the display has always used.
    pub rotation: f64,
    /// Number of columns this cell spans (including itself); `>= 1`.
    pub merge_width: u32,
    /// Number of rows this cell spans (including itself); `>= 1`.
    pub merge_height: u32,
    pub borders: TableCellBorders,
    /// The cell's own text (content) colour, sRGB (DXF cell group 64). `None`
    /// means "no override": the cell text is drawn in the entity's resolved
    /// colour, which is what every table did before a per-cell colour existed.
    ///
    /// The importer stores the **resolved** value (cell style, then row style,
    /// then the table's base style, then the `TABLESTYLE` row set), so a cell
    /// whose value equals its row's is still an inheriting cell.
    pub text_color: Option<[u8; 3]>,
    /// The cell's background fill colour, sRGB (DXF groups 63 + 283). `None`
    /// means "no background": no filled primitive is derived for the cell.
    pub fill_color: Option<[u8; 3]>,
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
/// [`Self::from_import`], [`Self::from_import_with_style`],
/// [`Self::transformed`], [`Self::with_cell_text`], [`Self::with_cell_texts`],
/// [`Self::with_merge`], [`Self::with_unmerge`], [`Self::with_cell_styles`],
/// [`Self::with_cell_format`], [`Self::with_style_resolved`] and
/// [`Self::with_style_name`]. `display` is derived from the other fields.
#[derive(Debug, Clone, PartialEq)]
pub struct TableGeometry {
    frame: TableFrame,
    columns: Vec<TableColumnGeometry>,
    rows: Vec<TableRowGeometry>,
    /// The table style snapshot (`TABLESTYLE`): the name, the data-row values
    /// and (when a row type is distinguished) the title/header row values.
    style: TableStyleValues,
    /// The sparse per-cell `GRIDFORMAT` margin overrides. A cell that sets no
    /// margin has no entry, so a table without margins stores nothing and emits
    /// no `CELLMARGIN` block ([`TableCellMargins`]).
    margins: CellMarginTable,
    name: String,
    description: String,
    display: display::TableDisplay,
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
        if geometry.display.geometry().is_empty() {
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
        let style = TableStyleValues::named(&parts.style_name);
        Self::from_import_with_style(parts, style)
    }

    /// Build a table from imported data with the source's resolved table style.
    ///
    /// The style name is taken from `style.name`, not from
    /// [`TableParts::style_name`] — the snapshot is the authority once the
    /// importer resolved the `TABLESTYLE` object. The cells keep their resolved
    /// values, so a cell that took its borders/height/style from that object
    /// still equals it and therefore still counts as inheriting.
    pub fn from_import_with_style(parts: TableParts, style: TableStyleValues) -> Self {
        let mut geometry = TableGeometry {
            frame: normalize_frame(parts.frame),
            columns: parts.columns,
            rows: parts.rows,
            style,
            margins: CellMarginTable::new(),
            name: parts.name,
            description: parts.description,
            display: display::TableDisplay::default(),
        };
        geometry.clamp_merge_spans();
        geometry.rebuild_display();
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
                        // An alignment and a frame-relative rotation are both
                        // invariant under a rotation + uniform scale.
                        alignment: cell.alignment,
                        rotation: cell.rotation,
                        merge_width: cell.merge_width,
                        merge_height: cell.merge_height,
                        borders: TableCellBorders {
                            top: scale_border(cell.borders.top, s),
                            right: scale_border(cell.borders.right, s),
                            bottom: scale_border(cell.borders.bottom, s),
                            left: scale_border(cell.borders.left, s),
                        },
                        // Colours are invariant under a similarity.
                        text_color: cell.text_color,
                        fill_color: cell.fill_color,
                    })
                    .collect(),
            })
            .collect();
        let mut geometry = TableGeometry {
            frame,
            columns,
            rows,
            style: self.style.scaled(s),
            margins: self
                .margins
                .iter()
                .map(|(address, margins)| (*address, margins.scaled(s)))
                .collect(),
            name: self.name.clone(),
            description: self.description.clone(),
            display: display::TableDisplay::default(),
        };
        geometry.rebuild_display();
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
        geometry.rebuild_display();
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

    /// One cell's `GRIDFORMAT` margin override (DXF `CELLMARGIN`), or
    /// [`TableCellMargins::INHERIT`] when the cell sets none.
    ///
    /// `None` for an address outside the grid, so a caller can tell "no
    /// override" from "no such cell".
    pub fn cell_margins(&self, row: usize, column: usize) -> Option<TableCellMargins> {
        self.cell(row, column)?;
        Some(
            self.margins
                .get(&(row, column))
                .copied()
                .unwrap_or(TableCellMargins::INHERIT),
        )
    }

    /// Every cell that stores a margin override, in row-major order.
    ///
    /// The exporter writes exactly these cells' `CELLMARGIN` blocks, so the file
    /// carries no block for a cell that inherits — that is what keeps an
    /// unset-margin table's file byte-identical to the pre-margin export.
    pub fn margin_overrides(
        &self,
    ) -> impl Iterator<Item = ((usize, usize), TableCellMargins)> + '_ {
        self.margins
            .iter()
            .map(|(address, margins)| (*address, *margins))
    }

    /// The style snapshot the cells inherit from (`TABLESTYLE`).
    pub fn style(&self) -> &TableStyleValues {
        &self.style
    }

    /// The table style name (the `TABLESTYLE` object's name).
    pub fn style_name(&self) -> &str {
        &self.style.name
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

    /// The derived display: grid lines, background fills and cell text.
    ///
    /// The appearance each primitive carries is a **parallel** list
    /// ([`Self::display_appearances`]); `display()` alone is the geometry
    /// projection every non-rendering consumer (bounds, picking, export) uses.
    pub fn display(&self) -> &[SemanticGeometry] {
        self.display.geometry()
    }

    /// The appearance of every primitive of [`Self::display`], in the same
    /// order (always the same length).
    ///
    /// [`TableDisplayAppearance::NONE`] means "no per-primitive override": the
    /// primitive is drawn in the entity's resolved colour/lineweight, so a table
    /// that never sets a colour derives the pre-appearance draw exactly.
    pub fn display_appearances(&self) -> &[TableDisplayAppearance] {
        self.display.appearance()
    }

    /// Rebuild the derived display after a definition edit.
    ///
    /// Private to the crate's table modules: the public writers call it
    /// themselves, so no caller can leave a display that disagrees with the
    /// definition.
    pub(crate) fn rebuild_display(&mut self) {
        self.display = display::build(&self.frame, &self.columns, &self.rows, &self.margins);
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

/// Scale a border's geometric part by the entity transform. The colour and the
/// lineweight are **not** geometric: a print width and a colour are invariant
/// under a scale, exactly like a DXF border override.
fn scale_border(border: TableBorder, factor: f64) -> TableBorder {
    TableBorder {
        visible: border.visible,
        double_spacing: border.double_spacing * factor,
        color: border.color,
        lineweight_hundredths: border.lineweight_hundredths,
    }
}

fn uniform_scale(transform: &Transform3) -> f64 {
    let m = &transform.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    ((sx + sy + sz) / 3.0).max(1e-12)
}

/// The structural rule one border must satisfy: its double-line spacing is a
/// finite non-negative length and its lineweight fits the DXF group range.
pub(crate) fn validate_border(border: TableBorder) -> CadResult<()> {
    if !border.double_spacing.is_finite() || border.double_spacing < 0.0 {
        return Err(CadError::InvalidInput(
            "table.structure: border double-line spacing must be finite and non-negative".into(),
        ));
    }
    if border
        .lineweight_hundredths
        .is_some_and(|hundredths| hundredths > MAX_BORDER_LINEWEIGHT_HUNDREDTHS)
    {
        return Err(CadError::InvalidInput(format!(
            "table.structure: border lineweights must be 0..={MAX_BORDER_LINEWEIGHT_HUNDREDTHS} \
             hundredths of a millimetre (the DXF group 275-279 range)"
        )));
    }
    Ok(())
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
            if !cell.rotation.is_finite() {
                return Err(CadError::InvalidInput(
                    "table.structure: cell text rotations must be finite".into(),
                ));
            }
            for border in [
                cell.borders.top,
                cell.borders.right,
                cell.borders.bottom,
                cell.borders.left,
            ] {
                validate_border(border)?;
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
mod appearance_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_support;
