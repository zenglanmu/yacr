//! TABLE cell-grid conversion and the table-border rendering helpers shared by
//! the entity submodules.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// A TABLE as its first-class cell grid plus per-cell formatting.
    ///
    /// Each cell's effective style (`cell.style` -> `row.style` -> `base_style`
    /// -> the `TableStyle` object's value set for that row's kind) decides which
    /// of its four edges are visible, its text height/style, its text alignment
    /// and its rotation. Cells whose style still equals the table style's values
    /// for their row therefore count as inheriting, which is what `TABLESTYLE`
    /// re-resolves.
    ///
    /// The representation carries one colour per entity and no filled area, so
    /// per-cell text colours, border colours/lineweights and background fills
    /// cannot be applied and are not stored: the result is always `Partial` and
    /// the reason states exactly what is omitted.
    ///
    /// The grid and the merge coverage are unchanged from the pre-table-entity
    /// `Compound` output; the display itself is derived by [`TableGeometry`]
    /// from the neutral definition.
    pub(crate) fn table_semantics(
        &self,
        t: &acadrust::entities::Table,
    ) -> (SemanticGeometry, Completeness) {
        let opaque = || SemanticGeometry::Opaque {
            type_key: "AcDbTable".into(),
            version: 1,
            payload: Vec::new(),
        };
        let origin = p3(t.insertion_point);
        let ux = cad_geometry::normalize(p3(t.horizontal_direction));
        let normal = cad_geometry::normalize(p3(t.normal));
        if cad_geometry::length(ux) < 1e-9 || cad_geometry::length(normal) < 1e-9 {
            return (
                opaque(),
                Completeness::Partial(vec!["table plane is degenerate".into()]),
            );
        }
        let widths: Vec<f64> = t.columns.iter().map(|c| c.width.max(0.0)).collect();
        let total_width: f64 = widths.iter().sum();
        let total_height: f64 = t.rows.iter().map(|r| r.height.max(0.0)).sum();
        if total_width <= 1e-9 || total_height <= 1e-9 {
            return (
                opaque(),
                Completeness::Partial(vec!["table has no positive extent".into()]),
            );
        }

        // Base styling for cells without an override of their own: the table
        // style object reached through `table_style_handle`. Table styles live in
        // `self.acad.objects` (there is no typed table for them). The object
        // carries one value set per row type, and the snapshot's suppression
        // flags say which rows are title/header rows, so each row resolves from
        // its own set instead of from the data row.
        let table_style = t
            .table_style_handle
            .and_then(|handle| self.acad.objects.get(&handle))
            .and_then(|object| match object {
                acadrust::objects::ObjectType::TableStyle(style) => Some(style),
                _ => None,
            });
        let snapshot = table_style_snapshot(table_style);

        // Whether any cell/row/base/table style was found; only then is the
        // border-fidelity reason reported instead of the plain-grid one.
        let mut styled = false;
        let mut unresolved_style = false;
        let mut dropped_extra_contents = false;
        let row_count = t.rows.len();
        let mut rows: Vec<TableRowGeometry> = Vec::with_capacity(row_count);
        for (row_index, row) in t.rows.iter().enumerate() {
            // The row's own value set from the style object, if the object is
            // there at all.
            let table_row_style = table_style
                .map(|style| row_style_for(style, snapshot.row_kind(row_index, row_count)));
            let table_style_borders = table_row_style.map(CellBorderSet::from_row_style);
            let mut cells: Vec<TableCellGeometry> = Vec::with_capacity(row.cells.len());
            for (column_index, cell) in row.cells.iter().enumerate() {
                // Effective style precedence: cell -> row -> base -> table style.
                let borders = if let Some(style) = cell.style.as_ref() {
                    styled = true;
                    CellBorderSet::from_cell_style(style)
                } else if let Some(style) = row.style.as_ref() {
                    styled = true;
                    CellBorderSet::from_cell_style(style)
                } else if let Some(style) = t.base_style.as_ref() {
                    styled = true;
                    CellBorderSet::from_cell_style(style)
                } else if let Some(borders) = table_style_borders {
                    styled = true;
                    borders
                } else {
                    CellBorderSet::default()
                };

                // Text height/style precedence: cell -> row -> base -> resolved
                // TableStyle row -> the cell content -> the importer default. The
                // DXF reader stores group 140/7 on `cell.style`, not on the
                // content (content defaults are 0.18 / ""), so reading the
                // content first would silently lose every source value.
                let content = cell.contents.first();
                let text_height = resolve_text_height(
                    cell.style.as_ref(),
                    row.style.as_ref(),
                    t.base_style.as_ref(),
                    table_row_style,
                    content,
                )
                .unwrap_or(DEFAULT_CELL_TEXT_HEIGHT);
                let style_name = resolve_text_style_name(
                    cell.style.as_ref(),
                    row.style.as_ref(),
                    t.base_style.as_ref(),
                    table_row_style,
                    content,
                )
                .unwrap_or_default();
                let resolved = self.style_ids.get(&style_name).copied();
                if !style_name.is_empty() && resolved.is_none() {
                    // The name is real source data; only the numeric id/font
                    // would have to be invented, so they are not.
                    unresolved_style = true;
                }
                let style = resolved.unwrap_or(StyleId(0));
                let font = if resolved.is_some() {
                    self.style_font(&style_name)
                } else {
                    None
                };
                // Alignment follows the same precedence, but a level only counts
                // when it actually overrides the property: a cell style that
                // exists only to hide a border must not reset the table style's
                // alignment to the DXF default.
                let alignment = resolve_alignment(
                    cell.style.as_ref(),
                    row.style.as_ref(),
                    t.base_style.as_ref(),
                    table_row_style,
                );

                if cell.contents.len() > 1
                    || cell.contents.iter().any(|content| {
                        content.block_handle.is_some() || content.field_handle.is_some()
                    })
                {
                    dropped_extra_contents = true;
                }

                // Only the first content is drawn: `formatted_value` when the
                // source has one, otherwise its raw text.
                let text = content
                    .map(|content| {
                        if !content.value.formatted_value.is_empty() {
                            content.value.formatted_value.clone()
                        } else {
                            content.value.text.clone()
                        }
                    })
                    .unwrap_or_default();
                cells.push(TableCellGeometry {
                    text,
                    text_height,
                    style_name,
                    style,
                    font,
                    alignment,
                    // Group 145 is the cell's own rotation in the table's
                    // coordinate system, i.e. relative to the frame. A
                    // non-finite source value has no placement and resolves to
                    // the neutral rotation.
                    rotation: if cell.rotation.is_finite() {
                        cell.rotation
                    } else {
                        0.0
                    },
                    merge_width: clamp_source_span(cell.merge_width, t.columns.len(), column_index),
                    merge_height: clamp_source_span(cell.merge_height, t.rows.len(), row_index),
                    borders: TableCellBorders {
                        top: border_view(borders.top),
                        right: border_view(borders.right),
                        bottom: border_view(borders.bottom),
                        left: border_view(borders.left),
                    },
                });
            }
            rows.push(TableRowGeometry {
                height: row.height,
                cells,
            });
        }

        let geometry = TableGeometry::from_import_with_style(
            TableParts {
                frame: TableFrame {
                    origin,
                    horizontal: ux,
                    normal,
                },
                columns: t
                    .columns
                    .iter()
                    .map(|c| TableColumnGeometry {
                        width: c.width,
                        name: c.name.clone(),
                    })
                    .collect(),
                rows,
                style_name: t
                    .table_style_handle
                    .and_then(|handle| self.style_name_of_object(handle))
                    .unwrap_or_default(),
                name: t.name.clone(),
                description: t.description.clone(),
            },
            snapshot,
        );
        if geometry.display().is_empty() {
            return (
                opaque(),
                Completeness::Partial(vec!["table has no drawable cells".into()]),
            );
        }
        let mut reasons = vec![if styled {
            "table borders drawn from cell styles; per-cell border colours/lineweights and cell \
             background fills are not applied (one colour per entity, no filled area) and are not \
             stored, so the writer emits the neutral ByBlock/no-fill values; merged ranges emit \
             only their anchor text; the double-line border offset direction is assumed inward; \
             border_type/override_flags are not honoured; each cell takes its whole border set \
             from the highest-priority style"
                .to_string()
        } else {
            "table grid and cell text drawn; merged cells and borders are approximated".to_string()
        }];
        // Named limitations of the modelling itself, kept separate so a caller
        // can see exactly what was not applied.
        reasons.push(
            "per-cell text colour, background fill and border colour/lineweight have no field in \
             this build's table model (one colour per entity, no filled area), so those DXF \
             groups are read and dropped by name rather than stored"
                .to_string(),
        );
        if table_style.is_some() {
            reasons.push(
                "table-style row value sets are applied per row type from the style's title/header \
                 suppression flags; a table that overrides which rows are title/header in \
                 TABLECONTENT data is not recoverable from this entity"
                    .to_string(),
            );
        }
        if dropped_extra_contents {
            reasons.push(
                "cell block content, FIELD references and contents after the first are not drawn; \
                 only the first content's text is kept"
                    .to_string(),
            );
        }
        if (normal.z - 1.0).abs() > 1e-9 {
            reasons.push(
                "the table plane normal is not +Z; cell text is drawn in world XY (rotation 0) \
                 because a flat Text cannot represent a tilted plane"
                    .to_string(),
            );
        }
        if unresolved_style {
            reasons.push(
                "cell text style name(s) not in the drawing's STYLE table are kept by name only; \
                 no style id or font is invented"
                    .to_string(),
            );
        }
        (
            SemanticGeometry::Table(Box::new(geometry)),
            Completeness::Partial(reasons),
        )
    }

    /// The name of a `TableStyle` object, when the handle resolves to one.
    fn style_name_of_object(&self, handle: acadrust::Handle) -> Option<String> {
        match self.acad.objects.get(&handle) {
            Some(acadrust::objects::ObjectType::TableStyle(style)) => Some(style.name.clone()),
            _ => None,
        }
    }
}

/// The neutral border for one resolved edge view.
fn border_view(border: BorderView) -> TableBorder {
    TableBorder {
        visible: !border.invisible,
        double_spacing: border.double_spacing,
    }
}

/// The table's `TABLESTYLE` snapshot: the resolved object, or the named default.
///
/// The style is stored on the entity (there is no drawing-level table-style
/// table in this build; the `MLEADERSTYLE` decision of ADR 0019 C/D). Every
/// field written here is also written back by the exporter, so the snapshot is a
/// real round-trip and not a display-only copy — including the object's three
/// row value sets, one per row type.
fn table_style_snapshot(resolved: Option<&acadrust::objects::TableStyle>) -> TableStyleValues {
    let name = resolved
        .map(|style| style.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TABLE_STYLE_NAME.to_string());
    let mut snapshot = TableStyleValues::named(&name);
    let Some(style) = resolved else {
        return snapshot;
    };
    snapshot.description = style.description.clone();
    snapshot.horizontal_margin = style.horizontal_margin;
    snapshot.vertical_margin = style.vertical_margin;
    snapshot.title_suppressed = style.title_suppressed;
    snapshot.header_suppressed = style.header_suppressed;
    // The data row keeps its own fields so the long-standing accessors and the
    // "inherit from the data row" default stay one value set.
    let data_row = row_style_snapshot(&style.data_row_style);
    snapshot.text_height = data_row.text_height;
    snapshot.text_style_name = data_row.text_style_name;
    snapshot.borders = data_row.borders;
    snapshot.data_row_alignment = data_row.alignment;
    // The other two rows are stored only when the source distinguishes them;
    // `set_row_style` canonicalises "same as the data row" to `None` so value
    // identity decides the linkage (no flag to disagree with the values).
    snapshot.set_row_style(
        TableRowKind::Title,
        row_style_snapshot(&style.title_row_style),
    );
    snapshot.set_row_style(
        TableRowKind::Header,
        row_style_snapshot(&style.header_row_style),
    );
    snapshot
}

/// One `TABLESTYLE` row record as the neutral row value set.
fn row_style_snapshot(row: &acadrust::objects::RowCellStyle) -> TableRowStyle {
    let mut out = TableRowStyle::default();
    if row.text_height.is_finite() && row.text_height > 0.0 {
        out.text_height = row.text_height;
    }
    if !row.text_style_name.is_empty() {
        out.text_style_name = row.text_style_name.clone();
    }
    out.alignment = TableCellAlignment::from_dxf_code(i32::from(row.alignment as i16));
    out.borders = TableCellBorders {
        top: border_view(BorderView::from_table_border(&row.top_border)),
        right: border_view(BorderView::from_table_border(&row.right_border)),
        bottom: border_view(BorderView::from_table_border(&row.bottom_border)),
        left: border_view(BorderView::from_table_border(&row.left_border)),
    };
    out
}

/// The `TABLESTYLE` row record a row of `kind` resolves from.
fn row_style_for(
    style: &acadrust::objects::TableStyle,
    kind: TableRowKind,
) -> &acadrust::objects::RowCellStyle {
    match kind {
        TableRowKind::Title => &style.title_row_style,
        TableRowKind::Header => &style.header_row_style,
        TableRowKind::Data => &style.data_row_style,
    }
}

/// Resolve a cell's effective text alignment from the source levels.
///
/// Unlike the height/style name, an alignment level only wins when it actually
/// overrides the property: a cell style that exists only to hide a border stores
/// the neutral `0` alignment code, which must not shadow the table style row's
/// real alignment. The DXF reader sets the property bit from the group's
/// presence, so `sets` is exactly "the source wrote group 170 here".
fn resolve_alignment(
    cell_style: Option<&acadrust::entities::CellStyle>,
    row_style: Option<&acadrust::entities::CellStyle>,
    base_style: Option<&acadrust::entities::CellStyle>,
    table_row_style: Option<&acadrust::objects::RowCellStyle>,
) -> TableCellAlignment {
    for style in [cell_style, row_style, base_style].into_iter().flatten() {
        if style.sets(acadrust::entities::CellStylePropertyFlags::ALIGNMENT) {
            return TableCellAlignment::from_dxf_code(style.alignment);
        }
    }
    table_row_style
        .map(|row| TableCellAlignment::from_dxf_code(i32::from(row.alignment as i16)))
        .unwrap_or(TableCellAlignment::DEFAULT)
}

/// Resolve a cell's effective text height from source levels, in precedence
/// order. A level only wins when it stores a finite positive height; otherwise
/// the next level is tried, and the caller supplies the importer default.
fn resolve_text_height(
    cell_style: Option<&acadrust::entities::CellStyle>,
    row_style: Option<&acadrust::entities::CellStyle>,
    base_style: Option<&acadrust::entities::CellStyle>,
    table_row_style: Option<&acadrust::objects::RowCellStyle>,
    content: Option<&acadrust::entities::CellContent>,
) -> Option<f64> {
    [
        cell_style.map(|style| style.text_height),
        row_style.map(|style| style.text_height),
        base_style.map(|style| style.text_height),
        table_row_style.map(|style| style.text_height),
        content.map(|content| content.text_height),
    ]
    .into_iter()
    .flatten()
    .find(|height| height.is_finite() && *height > 0.0)
}

/// Resolve a cell's effective text style name from the same precedence order.
fn resolve_text_style_name(
    cell_style: Option<&acadrust::entities::CellStyle>,
    row_style: Option<&acadrust::entities::CellStyle>,
    base_style: Option<&acadrust::entities::CellStyle>,
    table_row_style: Option<&acadrust::objects::RowCellStyle>,
    content: Option<&acadrust::entities::CellContent>,
) -> Option<String> {
    [
        cell_style.map(|style| style.text_style_name.as_str()),
        row_style.map(|style| style.text_style_name.as_str()),
        base_style.map(|style| style.text_style_name.as_str()),
        table_row_style.map(|style| style.text_style_name.as_str()),
        content.map(|content| content.text_style_name.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|name| !name.is_empty())
    .map(str::to_string)
}

/// Clamp an untrusted source merge span to the grid cells remaining from the
/// anchor. The result is at least one and never extends past `extent`.
fn clamp_source_span(span: i32, extent: usize, anchor: usize) -> u32 {
    let max = extent.saturating_sub(anchor).max(1) as u32;
    (span.max(1) as u32).min(max)
}

/// Visibility/spacing of one table cell edge, as needed to draw it.
#[derive(Debug, Clone, Copy)]
struct BorderView {
    invisible: bool,
    double_spacing: f64,
}

impl BorderView {
    /// A visible single border with no double-line spacing.
    const VISIBLE: BorderView = BorderView {
        invisible: false,
        double_spacing: 0.0,
    };

    fn from_cell_border(border: &acadrust::entities::CellBorder) -> Self {
        BorderView {
            invisible: border.invisible,
            double_spacing: border.double_spacing,
        }
    }

    fn from_table_border(border: &acadrust::objects::TableCellBorder) -> Self {
        BorderView {
            invisible: border.is_invisible,
            double_spacing: border.double_line_spacing,
        }
    }
}

/// The four resolved edges of one table cell's effective style.
#[derive(Debug, Clone, Copy)]
struct CellBorderSet {
    top: BorderView,
    right: BorderView,
    bottom: BorderView,
    left: BorderView,
}

impl Default for CellBorderSet {
    fn default() -> Self {
        CellBorderSet {
            top: BorderView::VISIBLE,
            right: BorderView::VISIBLE,
            bottom: BorderView::VISIBLE,
            left: BorderView::VISIBLE,
        }
    }
}

impl CellBorderSet {
    fn from_cell_style(style: &acadrust::entities::CellStyle) -> Self {
        CellBorderSet {
            top: BorderView::from_cell_border(&style.top_border),
            right: BorderView::from_cell_border(&style.right_border),
            bottom: BorderView::from_cell_border(&style.bottom_border),
            left: BorderView::from_cell_border(&style.left_border),
        }
    }

    fn from_row_style(style: &acadrust::objects::RowCellStyle) -> Self {
        CellBorderSet {
            top: BorderView::from_table_border(&style.top_border),
            right: BorderView::from_table_border(&style.right_border),
            bottom: BorderView::from_table_border(&style.bottom_border),
            left: BorderView::from_table_border(&style.left_border),
        }
    }
}

/// Push a non-degenerate line segment between two points.
pub(super) fn push_edge(children: &mut Vec<SemanticGeometry>, start: Point3, end: Point3) {
    if let Some(line) = line_between(start, end) {
        children.push(line);
    }
}
