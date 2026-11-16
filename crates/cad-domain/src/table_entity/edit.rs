//! Table cell/merge/style edits: the derivation-preserving writers.
//!
//! Every writer returns a **new** [`TableGeometry`] whose `display` was rebuilt
//! from the edited definition; no caller can store a display that disagrees with
//! the grid. A refusal leaves the receiver untouched (the writers clone first),
//! which is what makes "a refused command writes nothing" true at the model
//! layer too.
//!
//! The stable refusal codes (`table.*`) are the model's own structural codes;
//! the command layer adds the command-specific ones (`tabledit.*`,
//! `matchcell.*`, `tablestyle.*`) after checking its own preconditions (one
//! selected table, 1-based addressing).

use std::collections::HashSet;

use crate::{CadError, CadResult, StyleId, TableCellAlignment, TableGeometry, TableStyleValues};

use super::TableCellAppearanceEdit;

impl TableGeometry {
    /// The cell address that owns `(row, column)`: its own address when it is
    /// not covered, otherwise the merge anchor's.
    ///
    /// `None` when the address is outside the grid.
    pub fn merge_anchor(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        let _ = self.cell(row, column)?;
        let covered = merge_coverage(self.rows(), self.columns().len());
        if !covered.contains(&(row, column)) {
            return Some((row, column));
        }
        // The anchor is the unique cell whose span covers this address; the
        // coverage set is built from the spans, so a scan finds it exactly.
        for (row_index, source_row) in self.rows().iter().enumerate() {
            for (column_index, cell) in source_row.cells.iter().enumerate() {
                let span_w = span(cell.merge_width, self.columns().len(), column_index);
                let span_h = span(cell.merge_height, self.rows().len(), row_index);
                if row >= row_index
                    && row < row_index + span_h
                    && column >= column_index
                    && column < column_index + span_w
                {
                    return Some((row_index, column_index));
                }
            }
        }
        None
    }

    /// Whether `(row, column)` is inside the grid but belongs to another cell's
    /// merge range (so it draws no borders and no text of its own).
    pub fn is_cell_covered(&self, row: usize, column: usize) -> bool {
        self.cell(row, column).is_some()
            && merge_coverage(self.rows(), self.columns().len()).contains(&(row, column))
    }

    /// Merge the range anchored at `(row, column)` into `height` rows x `width`
    /// columns and rebuild the display.
    ///
    /// `(row, column)` is 0-based here (the command layer converts its 1-based
    /// payload). Structural refusals, all before a new geometry is built:
    ///
    /// * `table.cell_out_of_range` — the anchor is outside the grid;
    /// * `table.merge_span` — a span of zero (a merge always keeps its anchor);
    /// * `table.merge_out_of_grid` — the span reaches past the last row/column;
    /// * `table.merge_covered` — the anchor itself is covered by another merge;
    /// * `table.merge_overlap` — the span reaches into another merge's range.
    ///
    /// The covered cells keep their stored text (the model never drops content),
    /// it is simply not drawn: the command reports that as a diagnostic.
    pub fn with_merge(
        &self,
        row: usize,
        column: usize,
        height: u32,
        width: u32,
    ) -> CadResult<Self> {
        if self.cell(row, column).is_none() {
            return Err(out_of_range(row, column, self));
        }
        if height == 0 || width == 0 {
            return Err(CadError::InvalidInput(format!(
                "table.merge_span: the merge at ({row}, {column}) needs a height and width of at \
                 least 1, got {height} x {width}"
            )));
        }
        let columns = self.columns().len();
        let rows = self.rows().len();
        let span_w = width as usize;
        let span_h = height as usize;
        if column + span_w > columns || row + span_h > rows {
            return Err(CadError::InvalidInput(format!(
                "table.merge_out_of_grid: the merge at ({row}, {column}) spans {span_w} x \
                 {span_h}, outside the {rows} x {columns} grid"
            )));
        }
        let covered = merge_coverage(self.rows(), columns);
        if covered.contains(&(row, column)) {
            return Err(CadError::InvalidInput(format!(
                "table.merge_covered: cell ({row}, {column}) already belongs to another merge \
                 range; TABLEDIT UNMERGE it first"
            )));
        }
        for dr in 0..span_h {
            for dc in 0..span_w {
                let (r, c) = (row + dr, column + dc);
                if covered.contains(&(r, c)) {
                    return Err(CadError::InvalidInput(format!(
                        "table.merge_overlap: the merge at ({row}, {column}) reaches ({r}, {c}), \
                         which already belongs to another merge range"
                    )));
                }
            }
        }
        let mut geometry = self.clone();
        {
            let anchor = geometry
                .rows
                .get_mut(row)
                .and_then(|source| source.cells.get_mut(column))
                .ok_or_else(|| out_of_range(row, column, self))?;
            anchor.merge_width = width;
            anchor.merge_height = height;
        }
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Split the merge range covering `(row, column)` back into single cells.
    ///
    /// `(row, column)` may be the anchor or any covered cell (AutoCAD resolves a
    /// click inside a merged cell to its range). A cell that is not merged is
    /// `table.not_merged`: the command then writes nothing rather than a no-op
    /// transaction.
    pub fn with_unmerge(&self, row: usize, column: usize) -> CadResult<Self> {
        let Some((anchor_row, anchor_column)) = self.merge_anchor(row, column) else {
            return Err(out_of_range(row, column, self));
        };
        let span_w = self
            .cell(anchor_row, anchor_column)
            .map(|cell| cell.merge_width)
            .unwrap_or(1);
        let span_h = self
            .cell(anchor_row, anchor_column)
            .map(|cell| cell.merge_height)
            .unwrap_or(1);
        if span_w <= 1 && span_h <= 1 {
            return Err(CadError::InvalidInput(format!(
                "table.not_merged: cell ({row}, {column}) is not part of a merged range"
            )));
        }
        let mut geometry = self.clone();
        if let Some(anchor) = geometry
            .rows
            .get_mut(anchor_row)
            .and_then(|source| source.cells.get_mut(anchor_column))
        {
            anchor.merge_width = 1;
            anchor.merge_height = 1;
        }
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Copy one cell's **format** onto every target cell (`MATCHCELL`).
    ///
    /// The copied fields are the whole per-cell format this build models: text
    /// height, text style identity/name/font, the four border edges (visibility,
    /// double-line spacing, colour and lineweight), the text alignment, the text
    /// rotation and the cell's text colour and background fill.
    ///
    /// Text, merge spans and position are untouched: this is a style copy, not a
    /// content copy. The display is rebuilt once for the whole batch, so N
    /// targets are one rebuild / one transaction. Every address is validated
    /// before the first write (`table.cell_out_of_range`); an empty target list
    /// is `table.matchcell_no_targets`.
    pub fn with_cell_styles(
        &self,
        source: (usize, usize),
        targets: &[(usize, usize)],
    ) -> CadResult<Self> {
        let (source_row, source_column) = source;
        let Some(style_source) = self.cell(source_row, source_column).cloned() else {
            return Err(out_of_range(source_row, source_column, self));
        };
        if targets.is_empty() {
            return Err(CadError::InvalidInput(
                "table.matchcell_no_targets: MATCHCELL needs at least one destination cell".into(),
            ));
        }
        for (row, column) in targets {
            if self.cell(*row, *column).is_none() {
                return Err(out_of_range(*row, *column, self));
            }
        }
        let mut geometry = self.clone();
        for (row, column) in targets {
            let Some(target) = geometry
                .rows
                .get_mut(*row)
                .and_then(|source_row| source_row.cells.get_mut(*column))
            else {
                return Err(out_of_range(*row, *column, self));
            };
            target.text_height = style_source.text_height;
            target.style_name = style_source.style_name.clone();
            target.style = style_source.style;
            target.font = style_source.font.clone();
            target.borders = style_source.borders;
            target.alignment = style_source.alignment;
            target.rotation = style_source.rotation;
            target.text_color = style_source.text_color;
            target.fill_color = style_source.fill_color;
        }
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Set one cell's own appearance — text colour, background fill, border
    /// colour and border lineweight — and rebuild the display once
    /// (`TABLEDIT FORMAT COLOR/FILL/LINEWEIGHT/BORDERCOLOR`).
    ///
    /// Each field is a [`TableFieldEdit`](super::TableFieldEdit): `Keep` leaves
    /// the stored value, `Clear` returns it to "no override" (`COLOR BYLAYER`,
    /// `FILL NONE`) and `Set` stores a value. The border fields reach all four
    /// edges of the cell's resolved border set. The address must be inside the
    /// grid (`table.cell_out_of_range`); on a refusal the receiver is unchanged.
    pub fn with_cell_appearance(
        &self,
        row: usize,
        column: usize,
        edit: TableCellAppearanceEdit,
    ) -> CadResult<Self> {
        let mut geometry = self.clone();
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
        edit.text_color.apply(&mut cell.text_color);
        edit.fill.apply(&mut cell.fill_color);
        edit.apply_borders(&mut cell.borders);
        for border in [
            cell.borders.top,
            cell.borders.right,
            cell.borders.bottom,
            cell.borders.left,
        ] {
            super::validate_border(border)?;
        }
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Set one cell's alignment and/or text rotation and rebuild the display
    /// (`TABLEDIT FORMAT`).
    ///
    /// `None` leaves that field untouched, so the two typed fields combine in one
    /// write (`TABLEDIT FORMAT 1 2 ALIGN TL ROTATION 30`). The address must be
    /// inside the grid (`table.cell_out_of_range`) and a rotation must be finite
    /// (`table.cell_rotation`); on a refusal the receiver is unchanged.
    ///
    /// The rotation is stored **relative to the table frame** (DXF group 145 is
    /// in the table's coordinate system), in radians.
    pub fn with_cell_format(
        &self,
        row: usize,
        column: usize,
        alignment: Option<TableCellAlignment>,
        rotation: Option<f64>,
    ) -> CadResult<Self> {
        let mut geometry = self.clone();
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
        if let Some(rotation) = rotation {
            if !rotation.is_finite() {
                return Err(CadError::InvalidInput(
                    "table.cell_rotation: the cell text rotation must be finite".into(),
                ));
            }
            cell.rotation = rotation;
        }
        if let Some(alignment) = alignment {
            cell.alignment = alignment;
        }
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Replace the table's style snapshot and re-resolve every cell that still
    /// inherited from the old one.
    ///
    /// `resolve` maps a text style **name** to the drawing's `(StyleId, font)`
    /// pair; it is supplied by the command layer, which is the only layer that
    /// owns the drawing's STYLE table (the domain has no database). An empty name
    /// resolves to [`StyleId(0)`](StyleId) with no font, which is exactly what
    /// the importer stores for a cell whose style name is not in the table.
    ///
    /// A cell is "inheriting" when its borders (visibility, spacing, colour and
    /// lineweight), text height, text style name, alignment, text colour and
    /// background fill still equal **its row's** old values (value-identity, see
    /// [`TableStyleValues`]). Such a cell takes the new value for each of those
    /// fields, so an explicit override on the cell survives the style edit. The
    /// row's value set is classified by the style itself, so a `TITLE OFF` that
    /// removes the title row moves the old title row into the header/data set it
    /// now belongs to.
    pub fn with_style_resolved(
        &self,
        style: TableStyleValues,
        resolve: impl Fn(&str) -> (StyleId, Option<String>),
    ) -> CadResult<Self> {
        style.validate()?;
        let old = self.style().clone();
        let row_count = self.rows().len();
        let mut geometry = self.clone();
        for (row_index, row) in geometry.rows.iter_mut().enumerate() {
            let old_row = old.row_style(old.row_kind(row_index, row_count));
            let new_row = style.row_style(style.row_kind(row_index, row_count));
            for cell in &mut row.cells {
                let inherited = old.supplies_row(cell, &old_row);
                if inherited.borders {
                    cell.borders = new_row.borders;
                }
                if inherited.text_height {
                    cell.text_height = new_row.text_height;
                }
                if inherited.alignment {
                    cell.alignment = new_row.alignment;
                }
                if inherited.text_color {
                    cell.text_color = new_row.text_color;
                }
                if inherited.fill_color {
                    cell.fill_color = new_row.fill_color;
                }
                if inherited.text_style {
                    cell.style_name = new_row.text_style_name.clone();
                    let (id, font) = if new_row.text_style_name.is_empty() {
                        (StyleId(0), None)
                    } else {
                        resolve(&new_row.text_style_name)
                    };
                    cell.style = id;
                    cell.font = font;
                }
            }
        }
        geometry.style = style;
        geometry.rebuild_display();
        Ok(geometry)
    }

    /// Rename the style without touching any cell (`TABLE` creation keeps the
    /// `Standard` cells; the cells keep inheriting the new name's defaults).
    pub fn with_style_name(&self, name: &str) -> CadResult<Self> {
        let mut style: TableStyleValues = self.style().clone();
        style.name = name.trim().to_string();
        self.with_style_resolved(style, |_| (StyleId(0), None))
    }
}

/// The set of grid addresses covered by a merge anchor (all but the anchor).
///
/// Shared by the display derivation, the merge writers and the CSV serializer so
/// there is exactly one coverage rule. A malformed span is clamped to the
/// remaining grid: an untrusted source value near `i32::MAX` must never drive a
/// loop.
pub(crate) fn merge_coverage(
    rows: &[crate::TableRowGeometry],
    column_count: usize,
) -> HashSet<(usize, usize)> {
    let mut covered = HashSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (column_index, cell) in row.cells.iter().enumerate() {
            let span_w = span(cell.merge_width, column_count, column_index);
            let span_h = span(cell.merge_height, rows.len(), row_index);
            for dr in 0..span_h {
                for dc in 0..span_w {
                    if dr != 0 || dc != 0 {
                        covered.insert((row_index + dr, column_index + dc));
                    }
                }
            }
        }
    }
    covered
}

/// Clamp a stored merge span to the grid cells remaining from `anchor`.
pub(crate) fn span(value: u32, extent: usize, anchor: usize) -> usize {
    (value.max(1) as usize).min(extent.saturating_sub(anchor).max(1))
}

/// The `table.cell_out_of_range` refusal for a 0-based address.
pub(super) fn out_of_range(row: usize, column: usize, table: &TableGeometry) -> CadError {
    CadError::InvalidInput(format!(
        "table.cell_out_of_range: cell ({row}, {column}) is outside the table's {} rows x {} \
         columns (0-based)",
        table.rows().len(),
        table.columns().len()
    ))
}
