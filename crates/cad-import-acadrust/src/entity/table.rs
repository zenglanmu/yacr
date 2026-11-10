//! TABLE cell-grid conversion and the table-border rendering helpers shared by
//! the entity submodules.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    /// A TABLE as its first-class cell grid plus cell text.
    ///
    /// Each cell's effective style (`cell.style` -> `row.style` -> `base_style`
    /// -> the `TableStyle` object) decides which of its four edges are visible,
    /// with double-line borders drawn as a second parallel line; merged interior
    /// edges are suppressed. The representation carries one colour per entity,
    /// so per-cell border colours/lineweights and background fills cannot be
    /// applied: the result is always `Partial` and the reason states exactly
    /// what is omitted.
    ///
    /// The grid, the merge coverage and the two degradation reasons are
    /// unchanged from the pre-table-entity `Compound` output; the display itself
    /// is derived by [`TableGeometry`] from the neutral definition.
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

        // Whether any cell/row/base/table style was found; only then is the
        // border-fidelity reason reported instead of the plain-grid one.
        let mut styled = false;
        let mut rows: Vec<TableRowGeometry> = Vec::with_capacity(t.rows.len());
        for row in &t.rows {
            let mut cells: Vec<TableCellGeometry> = Vec::with_capacity(row.cells.len());
            for cell in &row.cells {
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

                // Only the first content is drawn: `formatted_value` when the
                // source has one, otherwise its raw text.
                let mut text = String::new();
                let mut text_height = 2.5;
                let mut style_name = String::new();
                let mut style = StyleId(0);
                let mut font = None;
                if let Some(content) = cell.contents.first() {
                    style_name = content.text_style_name.clone();
                    style = self.style_id(&content.text_style_name);
                    font = self.style_font(&content.text_style_name);
                    text = if !content.value.formatted_value.is_empty() {
                        content.value.formatted_value.clone()
                    } else {
                        content.value.text.clone()
                    };
                    if content.text_height > 0.0 {
                        text_height = content.text_height;
                    }
                }
                cells.push(TableCellGeometry {
                    text,
                    text_height,
                    style_name,
                    style,
                    font,
                    merge_width: cell.merge_width.max(1) as u32,
                    merge_height: cell.merge_height.max(1) as u32,
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

        let geometry = TableGeometry::from_import(TableParts {
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
        });
        if geometry.display().is_empty() {
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
            SemanticGeometry::Table(Box::new(geometry)),
            Completeness::Partial(vec![reason]),
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
