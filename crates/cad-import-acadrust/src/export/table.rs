//! TABLE export: a real DXF `ACAD_TABLE` entity built from the domain grid.
//!
//! The entity is written from the domain definition
//! (`frame + columns + rows + style/name/description`), never from the derived
//! display: the display is what this build renders, the `ACAD_TABLE` record is
//! what AutoCAD recomputes. Row heights, column widths/names, cell text, cell
//! text height and style name, merge spans and the table name/description are
//! written from the model.
//!
//! Honest deviations (always `Converted`, never `Exact`):
//!
//! * neither the `TABLESTYLE` object nor the table style name is written (the
//!   model keeps only a name), so re-import uses the Standard style instead of
//!   the source name;
//! * block content and FIELD references are not representable (the model keeps
//!   only the first content's text), so they are not written;
//! * per-cell border colours, lineweights, line types and background fills are
//!   lost (one colour per entity), and the DXF writer carries no double-line
//!   border spacing, so a doubled edge is written as a single visible edge.
//!
//! A block-embedded table is written at its placed coordinates: the accumulated
//! INSERT transform is applied through [`cad_db::transform_geometry`]
//! (translation + rotation + uniform scale), and a placement this grid cannot
//! represent (a tilt, mirror, or non-uniform scale) is recorded as a named
//! `Dropped` reason rather than written at block-local coordinates.

use acadrust::entities::{CellStyle, EntityType, Table as AcadTable, TableCell};
use acadrust::CadDocument;
use cad_db::{transform_geometry, DbEntity};
use cad_domain::*;

use super::common::{common_for, v3};
use super::entity::{kind_name, EntityContext, ExportAccumulator};
use super::report::MappingOutcome;
use super::tables::ensure_text_style;

/// Named conversion reason: neither the `TABLESTYLE` object nor its name is
/// written.
const REASON_STYLE_OBJECT: &str =
    "the TABLESTYLE object is not written; the table style name is also not written, so re-import \
     uses the Standard style";
/// Named conversion reason: block content / fields have no representation.
const REASON_CONTENT_KINDS: &str =
    "cell block content and FIELD references are not representable; only the first content's \
     text is written";
/// Named conversion reason: per-cell appearance and double spacing are dropped.
const REASON_BORDER_ATTRIBUTES: &str =
    "per-cell border colours, lineweights, line types and background fills are not written (one \
     colour per entity), and double-line border spacing is not written (a doubled edge becomes a \
     single visible edge)";
/// Named conversion reason: acadrust's DXF writer stores the linked-data
/// name/description on a separate `TABLECONTENT` object it does not emit for a
/// plain `TABLE` entity, so the values are dropped by the writer.
const REASON_NAME_DESCRIPTION: &str =
    "the linked table-data name and description are not written by the DXF writer (it stores \
     them on a separate TABLECONTENT object this export does not emit)";

/// Write a TABLE entity from the domain definition.
pub(super) fn emit_table(
    doc: &mut CadDocument,
    entity: &DbEntity,
    table: &TableGeometry,
    transform: &Transform3,
    ctx: &EntityContext,
    acc: &mut ExportAccumulator,
) -> CadResult<()> {
    let placed;
    let table: &TableGeometry = if transform == &Transform3::identity() {
        table
    } else {
        match transform_geometry(&SemanticGeometry::Table(Box::new(table.clone())), transform) {
            Ok(SemanticGeometry::Table(placed_table)) => {
                placed = *placed_table;
                &placed
            }
            // `transform_geometry` only ever returns the same variant it was
            // given; this arm is defensive and never reached.
            Ok(_) => {
                acc.record(
                    Some(entity.object.id),
                    "transform",
                    MappingOutcome::Dropped(vec![
                        "table transform returned a different geometry variant".to_string(),
                    ]),
                );
                return Ok(());
            }
            Err(error) => {
                acc.record(
                    Some(entity.object.id),
                    "transform",
                    MappingOutcome::Dropped(vec![format!(
                        "cannot place {} under an insert transform: {error}",
                        kind_name(&SemanticGeometry::Table(Box::new(table.clone())))
                    )]),
                );
                return Ok(());
            }
        }
    };
    let object = Some(entity.object.id);
    let rows = table.rows().len().max(1);
    let columns = table.columns().len().max(1);
    let frame = table.frame();
    let mut out = AcadTable::new(v3(frame.origin), rows, columns);
    out.horizontal_direction = v3(frame.horizontal);
    out.normal = v3(frame.normal);
    let (common, mut reasons) = common_for(doc, entity, ctx);
    out.common = common;
    out.name = table.name().to_string();
    out.description = table.description().to_string();

    for (column_index, column) in table.columns().iter().enumerate() {
        if let Some(target) = out.columns.get_mut(column_index) {
            target.width = column.width;
            target.name = column.name.clone();
        }
    }
    for (row_index, row) in table.rows().iter().enumerate() {
        if let Some(target) = out.rows.get_mut(row_index) {
            target.height = row.height;
        }
        for (column_index, cell) in row.cells.iter().enumerate() {
            if let Some(target) = out
                .rows
                .get_mut(row_index)
                .and_then(|row| row.cells.get_mut(column_index))
            {
                apply_cell(target, cell);
            }
        }
    }
    // Keep the header's merged-range list consistent with the cells' spans.
    out.sync_merged_ranges_from_cells();

    // Every distinct cell text style must exist so group 7/340 resolve on
    // re-import. Collect first (one entry per distinct name), then ensure each
    // exactly once: the previous nested `break` only left the inner loop and
    // registered at most one style per row.
    let mut styles: std::collections::BTreeMap<String, Option<String>> =
        std::collections::BTreeMap::new();
    for row in table.rows() {
        for cell in &row.cells {
            if cell.style_name.is_empty() {
                continue;
            }
            match styles.get_mut(&cell.style_name) {
                Some(font) => {
                    if font.is_none() {
                        *font = cell.font.clone();
                    }
                }
                None => {
                    styles.insert(cell.style_name.clone(), cell.font.clone());
                }
            }
        }
    }
    for (name, font) in &styles {
        let (_name, notes) = ensure_text_style(doc, name, font.as_deref());
        reasons.extend(notes);
    }

    acc.push(EntityType::Table(Box::new(out)), ctx)?;
    reasons.push(REASON_STYLE_OBJECT.to_string());
    reasons.push(REASON_CONTENT_KINDS.to_string());
    reasons.push(REASON_BORDER_ATTRIBUTES.to_string());
    reasons.push(REASON_NAME_DESCRIPTION.to_string());
    acc.record(object, "table", MappingOutcome::Converted(reasons));
    Ok(())
}

/// Copy one neutral cell into the acadrust cell record.
///
/// The borders are written through a `CellStyle`: with `override_flags == 0`
/// the DXF writer emits all four edge visibilities, which is exactly what the
/// model resolved. Text height and style name ride on the same style record.
fn apply_cell(target: &mut TableCell, cell: &TableCellGeometry) {
    if !cell.text.is_empty() {
        target.set_text(&cell.text);
        if let Some(content) = target.contents.first_mut() {
            content.text_height = cell.text_height;
            content.text_style_name = cell.style_name.clone();
        }
    }
    target.merge_width = cell.merge_width.max(1) as i32;
    target.merge_height = cell.merge_height.max(1) as i32;

    let mut style = CellStyle::new();
    style.text_style_name = cell.style_name.clone();
    style.text_height = cell.text_height;
    style.top_border.invisible = !cell.borders.top.visible;
    style.right_border.invisible = !cell.borders.right.visible;
    style.bottom_border.invisible = !cell.borders.bottom.visible;
    style.left_border.invisible = !cell.borders.left.visible;
    target.style = Some(style);
}
