//! TABLE cell-grid conversion and the table-border rendering helpers shared by
//! the entity submodules.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// A TABLE as its cell grid plus cell text.
    ///
    /// Each cell's effective style (`cell.style` -> `row.style` -> `base_style`
    /// -> the `TableStyle` object) decides which of its four edges are visible,
    /// with double-line borders drawn as a second parallel line; merged interior
    /// edges are suppressed. The representation carries one colour per entity,
    /// so per-cell border colours/lineweights and background fills cannot be
    /// applied: the result is always `Partial` and the reason states exactly
    /// what is omitted.
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
        let uy = cad_geometry::normalize(cad_geometry::cross(normal, ux));
        let corner = |x: f64, y: f64| {
            cad_geometry::add(
                origin,
                cad_geometry::add(cad_geometry::scale(ux, x), cad_geometry::scale(uy, y)),
            )
        };
        let widths: Vec<f64> = t.columns.iter().map(|c| c.width.max(0.0)).collect();
        let total_width: f64 = widths.iter().sum();
        let total_height: f64 = t.rows.iter().map(|r| r.height.max(0.0)).sum();
        if total_width <= 1e-9 || total_height <= 1e-9 {
            return (
                opaque(),
                Completeness::Partial(vec!["table has no positive extent".into()]),
            );
        }
        let mut children: Vec<SemanticGeometry> = Vec::new();

        // Base styling for cells without an override of their own: the table
        // style object reached through `table_style_handle`. Table styles live in
        // `self.acad.objects` (there is no typed table for them); its data-row
        // style is used because the entity does not record title/header row
        // counts here, an approximation reported in the reason below.
        let table_style_borders: Option<CellBorderSet> = t
            .table_style_handle
            .and_then(|handle| self.acad.objects.get(&handle))
            .and_then(|object| match object {
                acadrust::objects::ObjectType::TableStyle(style) => {
                    Some(CellBorderSet::from_row_style(&style.data_row_style))
                }
                _ => None,
            });

        // Cells covered by a merge anchor (all but the top-left cell). Their
        // interior edges belong to the merged range and are not drawn.
        let mut covered: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        for (row_index, row) in t.rows.iter().enumerate() {
            for (column_index, cell) in row.cells.iter().enumerate() {
                let span_w = cell.merge_width.max(1) as usize;
                let span_h = cell.merge_height.max(1) as usize;
                for dr in 0..span_h {
                    for dc in 0..span_w {
                        if dr != 0 || dc != 0 {
                            covered.insert((row_index + dr, column_index + dc));
                        }
                    }
                }
            }
        }
        // Whether any cell/row/base/table style was found; only then is the
        // border-fidelity reason reported instead of the plain-grid one.
        let mut styled = false;
        let mut y = 0.0;
        for (row_index, row) in t.rows.iter().enumerate() {
            let height = row.height.max(0.0);
            let mut x = 0.0;
            for (column_index, cell) in row.cells.iter().enumerate() {
                let width = widths.get(column_index).copied().unwrap_or(0.0);
                if covered.contains(&(row_index, column_index)) {
                    x += width;
                    continue;
                }
                // A merge anchor spans its covered columns/rows.
                let span_w: f64 = (0..cell.merge_width.max(1) as usize)
                    .map(|dc| widths.get(column_index + dc).copied().unwrap_or(0.0))
                    .sum();
                let span_h: f64 = (0..cell.merge_height.max(1) as usize)
                    .map(|dr| {
                        t.rows
                            .get(row_index + dr)
                            .map(|r| r.height.max(0.0))
                            .unwrap_or(0.0)
                    })
                    .sum();

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
                // Each cell contributes the four edges its effective style
                // leaves visible; a double border adds a second parallel line.
                push_cell_borders(&mut children, &corner, x, y, span_w, span_h, &borders);

                if let Some(content) = cell.contents.first() {
                    let text = if !content.value.formatted_value.is_empty() {
                        content.value.formatted_value.clone()
                    } else {
                        content.value.text.clone()
                    };
                    if !text.is_empty() {
                        let text_height = if content.text_height > 0.0 {
                            content.text_height
                        } else {
                            2.5
                        };
                        children.push(SemanticGeometry::Text {
                            text,
                            position: corner(x + span_w * 0.5, y + span_h * 0.5),
                            style: self.style_id(&content.text_style_name),
                            height: text_height,
                            rotation: 0.0,
                            font: self.style_font(&content.text_style_name),
                            h_align: TextAlignH::Center,
                            v_align: TextAlignV::Middle,
                        });
                    }
                }
                x += width;
            }
            y += height;
        }
        if children.is_empty() {
            return (
                opaque(),
                Completeness::Partial(vec!["table has no drawable cells".into()]),
            );
        }
        let reason = if styled {
            "table borders drawn from cell styles; per-cell border colours/lineweights and cell \
             background fills are not applied (one colour per entity); merged ranges emit only \
             their anchor text; table-style header/title row borders are approximated by the \
             data-row style; double-line border offset direction is assumed inward; \
             border_type/override_flags are not honoured; each cell takes its whole border set \
             from the highest-priority style"
                .to_string()
        } else {
            "table grid and cell text drawn; merged cells and borders are approximated".to_string()
        };
        (
            SemanticGeometry::Compound(children),
            Completeness::Partial(vec![reason]),
        )
    }
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
    borders: &CellBorderSet,
) {
    let bottom = borders.bottom;
    if !bottom.invisible {
        push_edge(children, corner(x, y), corner(x + width, y));
        if bottom.double_spacing > 0.0 {
            let s = bottom.double_spacing;
            push_edge(children, corner(x, y + s), corner(x + width, y + s));
        }
    }
    let top = borders.top;
    if !top.invisible {
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
    if !left.invisible {
        push_edge(children, corner(x, y), corner(x, y + height));
        if left.double_spacing > 0.0 {
            let s = left.double_spacing;
            push_edge(children, corner(x + s, y), corner(x + s, y + height));
        }
    }
    let right = borders.right;
    if !right.invisible {
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
pub(super) fn push_edge(children: &mut Vec<SemanticGeometry>, start: Point3, end: Point3) {
    if let Some(line) = line_between(start, end) {
        children.push(line);
    }
}
