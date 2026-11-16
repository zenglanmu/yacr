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
    /// The per-cell appearance is modelled: the text colour (cell content colour,
    /// group 64), the background fill (groups 63 + 283) and each border edge's
    /// colour/lineweight (groups 65-69 and 275-279) are resolved through the same
    /// cell -> row -> base -> `TABLESTYLE` row precedence and stored on the cell,
    /// so the display and the exporter both carry them. The result is still
    /// `Partial` and the reasons state exactly what is *not* applied.
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
                // Visibility and spacing come from the **highest level that has a
                // style record at all** (a level with no record sets nothing),
                // while a colour/lineweight is only meaningful when it is
                // concrete, so those two resolve per property from the first
                // level that carries a value — a cell style that exists only to
                // hide an edge cannot wipe the row's border colour.
                let level_sets: [Option<CellBorderSet>; 4] = [
                    cell.style.as_ref().map(CellBorderSet::from_cell_style),
                    row.style.as_ref().map(CellBorderSet::from_cell_style),
                    t.base_style.as_ref().map(CellBorderSet::from_cell_style),
                    table_style_borders,
                ];
                if level_sets.iter().any(Option::is_some) {
                    styled = true;
                }
                let borders = level_sets
                    .iter()
                    .flatten()
                    .next()
                    .copied()
                    .unwrap_or_default()
                    .with_resolved_appearance(&level_sets);

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
                // Per-property appearance: unlike the border set (one level wins
                // for all four edges), a colour resolves from the highest level
                // that carries a **concrete** value, so a cell style that exists
                // only to set a text height cannot wipe the row's colour.
                let text_color = resolve_text_color(
                    cell.style.as_ref(),
                    row.style.as_ref(),
                    t.base_style.as_ref(),
                    table_row_style,
                );
                let fill_color = resolve_fill(
                    cell.style.as_ref(),
                    row.style.as_ref(),
                    t.base_style.as_ref(),
                    table_row_style,
                );
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
                    text_color,
                    fill_color,
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
            "table borders drawn from cell styles; merged ranges emit only their anchor text; the \
             double-line border offset direction is assumed inward; border_type/override_flags are \
             not honoured; each cell takes its border visibility/spacing from the highest-priority \
             style level that has a record, and each edge's colour/lineweight from the first level \
             that carries a concrete value"
                .to_string()
        } else {
            "table grid and cell text drawn; merged cells and borders are approximated".to_string()
        }];
        // Named limitations of the modelling itself, kept separate so a caller
        // can see exactly what was not applied.
        reasons.push(
            "per-cell text colour, background fill and border colour/lineweight are stored and \
             drawn, but a cell that turns an inherited background fill *off* over a style row that \
             enables one re-resolves to the row's fill on re-import (the DXF cell record has no \
             'fill explicitly off' value this reader can tell from 'not written'); a per-border \
             line **type** is not modelled (this writer emits no per-cell border line type), and \
             the renderer draws every border one pixel wide, so a stored lineweight reaches the \
             batch but no wider stroke"
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

/// The resolved border for one edge view (visibility, spacing, colour and
/// lineweight).
fn border_view(border: BorderView) -> TableBorder {
    TableBorder {
        visible: !border.invisible,
        double_spacing: border.double_spacing,
        color: border.color,
        lineweight_hundredths: border.lineweight_hundredths,
    }
}

/// The cell/row text colour: the first level that carries a **concrete** colour
/// (an ACI index or a true colour). A symbolic `ByBlock`/`ByLayer`/`None` means
/// "inherit", exactly like the DXF default, so it is skipped rather than stored
/// as black.
fn resolve_text_color(
    cell_style: Option<&acadrust::entities::CellStyle>,
    row_style: Option<&acadrust::entities::CellStyle>,
    base_style: Option<&acadrust::entities::CellStyle>,
    table_row_style: Option<&acadrust::objects::RowCellStyle>,
) -> Option<[u8; 3]> {
    [
        cell_style.map(|style| style.content_color),
        row_style.map(|style| style.content_color),
        base_style.map(|style| style.content_color),
        table_row_style.map(|row| row.text_color),
    ]
    .into_iter()
    .flatten()
    .find_map(crate::style::concrete_rgb)
}

/// The cell/row background fill: the first level that **enables** a fill and
/// carries a concrete colour.
///
/// A level with the fill switched off is "no override" (every source that never
/// wrote the group reads exactly that way), so it is skipped; the lowest level is
/// the `TABLESTYLE` row set, whose fill is the table style's own default.
fn resolve_fill(
    cell_style: Option<&acadrust::entities::CellStyle>,
    row_style: Option<&acadrust::entities::CellStyle>,
    base_style: Option<&acadrust::entities::CellStyle>,
    table_row_style: Option<&acadrust::objects::RowCellStyle>,
) -> Option<[u8; 3]> {
    [
        cell_style.map(|style| (style.fill_enabled, style.background_color)),
        row_style.map(|style| (style.fill_enabled, style.background_color)),
        base_style.map(|style| (style.fill_enabled, style.background_color)),
        table_row_style.map(|row| (row.fill_enabled, row.fill_color)),
    ]
    .into_iter()
    .flatten()
    .find_map(|(enabled, color)| enabled.then(|| crate::style::concrete_rgb(color)).flatten())
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
    snapshot.text_color = data_row.text_color;
    snapshot.fill_color = data_row.fill_color;
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
    out.text_color = crate::style::concrete_rgb(row.text_color);
    // A row record always carries a fill colour; only the enable flag says
    // whether the row paints one, so an index without the flag is not a fill.
    out.fill_color = row
        .fill_enabled
        .then(|| crate::style::concrete_rgb(row.fill_color))
        .flatten();
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
    /// The edge's own colour, when the source carries a concrete one.
    color: Option<[u8; 3]>,
    /// The edge's lineweight in 1/100 mm, when the source carries a real weight
    /// (a symbolic `ByLayer`/`ByBlock`/`Default` is not a value).
    lineweight_hundredths: Option<u16>,
}

impl BorderView {
    /// A visible single border with no double-line spacing and no
    /// colour/lineweight override.
    const VISIBLE: BorderView = BorderView {
        invisible: false,
        double_spacing: 0.0,
        color: None,
        lineweight_hundredths: None,
    };

    fn from_cell_border(border: &acadrust::entities::CellBorder) -> Self {
        BorderView {
            invisible: border.invisible,
            double_spacing: border.double_spacing,
            color: crate::style::concrete_rgb(border.color),
            lineweight_hundredths: lineweight_hundredths(border.line_weight),
        }
    }

    fn from_table_border(border: &acadrust::objects::TableCellBorder) -> Self {
        BorderView {
            invisible: border.is_invisible,
            double_spacing: border.double_line_spacing,
            color: crate::style::concrete_rgb(border.color),
            lineweight_hundredths: lineweight_hundredths(border.line_weight),
        }
    }
}

/// The concrete 1/100 mm weight a DXF border carries, or `None` for a symbolic
/// one (`ByLayer`/`ByBlock`/`Default`, which are not weights).
fn lineweight_hundredths(weight: acadrust::LineWeight) -> Option<u16> {
    match weight {
        acadrust::LineWeight::Value(hundredths) if hundredths >= 0 => Some(hundredths as u16),
        _ => None,
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
    /// The four edges in `[top, right, bottom, left]` order.
    fn edges(&self) -> [BorderView; 4] {
        [self.top, self.right, self.bottom, self.left]
    }

    /// This set's visibility/spacing with every edge's colour and lineweight
    /// taken from the first level (in precedence order) that carries a concrete
    /// value for that edge.
    fn with_resolved_appearance(self, levels: &[Option<CellBorderSet>; 4]) -> Self {
        let resolved = |edge: usize, pick: fn(&BorderView) -> Option<[u8; 3]>| {
            levels
                .iter()
                .flatten()
                .find_map(|level| pick(&level.edges()[edge]))
        };
        let resolved_weight = |edge: usize| {
            levels
                .iter()
                .flatten()
                .find_map(|level| level.edges()[edge].lineweight_hundredths)
        };
        let mut edges = self.edges();
        for (index, edge) in edges.iter_mut().enumerate() {
            edge.color = resolved(index, |view| view.color);
            edge.lineweight_hundredths = resolved_weight(index);
        }
        CellBorderSet {
            top: edges[0],
            right: edges[1],
            bottom: edges[2],
            left: edges[3],
        }
    }

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
