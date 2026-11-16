//! TABLE export: a real DXF `ACAD_TABLE` entity built from the domain grid.
//!
//! The entity is written from the domain definition
//! (`frame + columns + rows + style/name/description`), never from the derived
//! display: the display is what this build renders, the `ACAD_TABLE` record is
//! what AutoCAD recomputes. Row heights, column widths/names, cell text, cell
//! text height and style name, cell alignment and text rotation, merge spans and
//! the table name/description are written from the model.
//!
//! Honest deviations (always `Converted`, never `Exact`):
//!
//! * the `TABLESTYLE` object **is** written (all three row value sets — data,
//!   title and header — plus the margins and the title/header suppression flags)
//!   and the entity points at it, so the style name, its per-row values and the
//!   title/header row classification round-trip;
//! * block content and FIELD references are not representable (the model keeps
//!   only the first content's text), so they are not written;
//! * per-cell text colours, border colours, lineweights, line types and
//!   background fills are not part of the model (one colour per entity, no
//!   filled area), so the writer emits this build's neutral `ByBlock`/no-fill
//!   values; the conversion reason states that instead of implying a round trip;
//! * a cell's border double-line spacing and border type have no per-cell group
//!   in the writer this build links against (it emits no per-cell `GRIDFORMAT`),
//!   so they re-resolve from the style object on re-import.
//!
//! A block-embedded table is written at its placed coordinates: the accumulated
//! INSERT transform is applied through [`cad_db::transform_geometry`]
//! (translation + rotation + uniform scale), and a placement this grid cannot
//! represent (a tilt, mirror, or non-uniform scale) is recorded as a named
//! `Dropped` reason rather than written at block-local coordinates.

use acadrust::entities::{CellEdgeFlags, CellStyle, EntityType, Table as AcadTable, TableCell};
use acadrust::objects::{ObjectType, TableCellBorder, TableStyle as AcadTableStyle};
use acadrust::{CadDocument, Handle};
use cad_db::{transform_geometry, DbEntity};
use cad_domain::*;

use super::common::{common_for, v3};
use super::entity::{kind_name, EntityContext, ExportAccumulator};
use super::report::MappingOutcome;
use super::tables::ensure_text_style;

/// Named conversion reason: block content / fields have no representation.
const REASON_CONTENT_KINDS: &str =
    "cell block content and FIELD references are not representable; only the first content's \
     text is written";
/// Named conversion reason: the per-cell appearance fields this build does not
/// model.
const REASON_CELL_APPEARANCE: &str =
    "per-cell text colour, border colour/lineweight and background fill are not part of this \
     build's table model (one colour per entity, no filled area), so the writer emits the neutral \
     ByBlock/no-fill values and a source value cannot survive";
/// Named conversion reason: the per-cell border geometry the writer cannot emit.
const REASON_CELL_GRID: &str =
    "a cell's border double-line spacing, border type and margin overrides are not written (the \
     DXF writer emits no per-cell GRIDFORMAT for a plain TABLE entity), so a re-import resolves \
     them from the table style";
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

    // Write (or reuse) the `TABLESTYLE` object and point the entity at it, so
    // the style name and its values survive the round trip. One record per
    // distinct name (the MLEADERSTYLE rule, ADR 0019 D); a second table with the
    // same name but different values is reported, never silently overwritten.
    let (style_handle, style_notes) = ensure_table_style(doc, table.style());
    out.table_style_handle = Some(style_handle);
    reasons.extend(style_notes);

    acc.push(EntityType::Table(Box::new(out)), ctx)?;
    reasons.push(REASON_CONTENT_KINDS.to_string());
    reasons.push(REASON_CELL_APPEARANCE.to_string());
    reasons.push(REASON_CELL_GRID.to_string());
    reasons.push(REASON_NAME_DESCRIPTION.to_string());
    acc.record(object, "table", MappingOutcome::Converted(reasons));
    Ok(())
}

/// Named conversion reason: two `TABLESTYLE` fields have no DXF group in the
/// writer this build uses.
const REASON_STYLE_TEXT_LOSSES: &str =
    "the table style description and its double-line border spacing are not written (the DXF \
     writer emits neither a description group nor a style-border spacing); re-import resolves \
     both to their defaults";

/// The text style each of the three row records points at.
struct RowTextStyle {
    style_name: String,
    handle: Handle,
}

/// Write (or reuse) the `TABLESTYLE` object `style` names and return its handle.
///
/// The object goes into the document's `ACAD_TABLESTYLE` dictionary — the place
/// AutoCAD and this reader look for it — with the three row value sets (data,
/// title, header) the display resolved. A record that already exists with the
/// **same** values is reused as is (the document's `Standard` object is created
/// by `CadDocument::new`); one with different values for the same name keeps the
/// first snapshot and reports the disagreement.
fn ensure_table_style(doc: &mut CadDocument, style: &TableStyleValues) -> (Handle, Vec<String>) {
    let mut reasons = Vec::new();
    // Every row record names its own text style, so each distinct name must
    // exist in the STYLE table with a handle the record can point at.
    let mut resolved: Vec<RowTextStyle> = Vec::new();
    for kind in [
        TableRowKind::Data,
        TableRowKind::Title,
        TableRowKind::Header,
    ] {
        let row = style.row_style(kind);
        let text_style = if row.text_style_name.trim().is_empty() {
            "Standard".to_string()
        } else {
            row.text_style_name.trim().to_string()
        };
        let (_name, notes) = ensure_text_style(doc, &text_style, None);
        reasons.extend(notes);
        let handle = text_style_handle(doc, &text_style);
        resolved.push(RowTextStyle {
            style_name: text_style,
            handle,
        });
    }

    match existing_table_style_handle(doc, &style.name) {
        Some(handle) => {
            let existing = doc.objects.get(&handle).and_then(|object| match object {
                ObjectType::TableStyle(existing) => Some(existing),
                _ => None,
            });
            let matches =
                existing.is_some_and(|existing| table_style_matches(existing, style, &resolved));
            let untouched_default = existing.is_some_and(is_document_default_style);
            if !matches && untouched_default {
                // The document's own `Standard` record (created by
                // `CadDocument::new`) has never been written by a table, so this
                // table's snapshot replaces it in place: the first table that
                // names a style owns that record.
                let mut record = table_style_record(style, &resolved);
                record.handle = handle;
                record.owner_handle = existing
                    .map(|existing| existing.owner_handle)
                    .unwrap_or(Handle::NULL);
                doc.objects.insert(handle, ObjectType::TableStyle(record));
                reasons.push(REASON_STYLE_TEXT_LOSSES.to_string());
            } else if !matches {
                reasons.push(format!(
                    "the TABLESTYLE record '{}' was written from another table's snapshot; this \
                     table's style values differ, so re-import resolves this table to the first \
                     snapshot",
                    style.name
                ));
            }
            (handle, reasons)
        }
        None => {
            let mut record = table_style_record(style, &resolved);
            let handle = doc.allocate_handle();
            record.handle = handle;
            match table_style_dictionary(doc) {
                Some(owner) => {
                    record.owner_handle = owner;
                    if let Some(ObjectType::Dictionary(dictionary)) = doc.objects.get_mut(&owner) {
                        dictionary.add_entry(style.name.clone(), handle);
                    }
                }
                None => reasons.push(
                    "the ACAD_TABLESTYLE dictionary is missing; the TABLESTYLE record is written \
                     with a null owner"
                        .into(),
                ),
            }
            doc.objects.insert(handle, ObjectType::TableStyle(record));
            reasons.push(REASON_STYLE_TEXT_LOSSES.to_string());
            (handle, reasons)
        }
    }
}

/// The `ACAD_TABLESTYLE` dictionary handle, when the document has one.
fn table_style_dictionary(doc: &CadDocument) -> Option<Handle> {
    let root = doc.header.named_objects_dict_handle;
    match doc.objects.get(&root) {
        Some(ObjectType::Dictionary(dictionary)) => dictionary
            .entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("ACAD_TABLESTYLE"))
            .map(|(_, handle)| *handle),
        _ => None,
    }
}

/// The handle of the `TABLESTYLE` record already carrying `name`.
fn existing_table_style_handle(doc: &CadDocument, name: &str) -> Option<Handle> {
    doc.objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::TableStyle(style) if style.name.eq_ignore_ascii_case(name) => Some(*handle),
            _ => None,
        })
}

/// The handle of a text-style table entry, allocating one when it has none.
///
/// The DXF writer only fills null table-entry handles on its own copy, so a
/// style reference needs a stable value here.
fn text_style_handle(doc: &mut CadDocument, name: &str) -> Handle {
    let needs_handle = doc
        .text_styles
        .get(name)
        .map(|entry| entry.handle.is_null())
        .unwrap_or(false);
    if needs_handle {
        let handle = doc.allocate_handle();
        if let Some(entry) = doc.text_styles.get_mut(name) {
            entry.handle = handle;
        }
    }
    doc.text_styles
        .get(name)
        .map(|entry| entry.handle)
        .unwrap_or(Handle::NULL)
}

/// Build the `TABLESTYLE` record from the entity's snapshot.
///
/// One `AcDbTableStyle` row record per row type, written in the order the DXF
/// reader expects them back (data, title, header), which is also the order
/// acadrust's writer emits them in.
fn table_style_record(style: &TableStyleValues, text_styles: &[RowTextStyle]) -> AcadTableStyle {
    let mut record = AcadTableStyle::new(&style.name);
    record.description = style.description.clone();
    record.horizontal_margin = style.horizontal_margin;
    record.vertical_margin = style.vertical_margin;
    record.title_suppressed = style.title_suppressed;
    record.header_suppressed = style.header_suppressed;
    let data = &mut record.data_row_style;
    write_row_style(data, &style.data_row(), &text_styles[0]);
    // The inside borders are the separators between cells of a range; the model
    // has one border set per cell edge, so the inside edges repeat the row's
    // outer edges rather than being left at the record default.
    data.horizontal_inside_border = table_cell_border(style.borders.bottom);
    data.vertical_inside_border = table_cell_border(style.borders.right);
    let title = &mut record.title_row_style;
    write_row_style(
        title,
        &style.row_style(TableRowKind::Title),
        &text_styles[1],
    );
    title.horizontal_inside_border =
        table_cell_border(style.row_style(TableRowKind::Title).borders.bottom);
    title.vertical_inside_border =
        table_cell_border(style.row_style(TableRowKind::Title).borders.right);
    let header = &mut record.header_row_style;
    write_row_style(
        header,
        &style.row_style(TableRowKind::Header),
        &text_styles[2],
    );
    header.horizontal_inside_border =
        table_cell_border(style.row_style(TableRowKind::Header).borders.bottom);
    header.vertical_inside_border =
        table_cell_border(style.row_style(TableRowKind::Header).borders.right);
    record
}

/// Copy one neutral row value set into an `AcDbTableStyle` row record.
fn write_row_style(
    target: &mut acadrust::objects::RowCellStyle,
    row: &TableRowStyle,
    text_style: &RowTextStyle,
) {
    target.text_height = row.text_height;
    target.text_style_name = text_style.style_name.clone();
    target.text_style_handle = (!text_style.handle.is_null()).then_some(text_style.handle);
    target.alignment = acadrust::objects::CellAlignment::from(row.alignment.dxf_code() as i16);
    target.top_border = table_cell_border(row.borders.top);
    target.right_border = table_cell_border(row.borders.right);
    target.bottom_border = table_cell_border(row.borders.bottom);
    target.left_border = table_cell_border(row.borders.left);
}

/// One style-object border from the neutral edge.
fn table_cell_border(border: TableBorder) -> TableCellBorder {
    let mut out = TableCellBorder::new();
    out.is_invisible = !border.visible;
    out.double_line_spacing = border.double_spacing;
    out
}

/// Whether an existing `TABLESTYLE` record already holds this snapshot.
///
/// All three row records are compared on the fields the writer emits, so a
/// record written from a *different* table's snapshot (including a different
/// title/header row set) is reported instead of silently reused.
fn table_style_matches(
    existing: &AcadTableStyle,
    style: &TableStyleValues,
    text_styles: &[RowTextStyle],
) -> bool {
    existing.name.eq_ignore_ascii_case(&style.name)
        && existing.horizontal_margin == style.horizontal_margin
        && existing.vertical_margin == style.vertical_margin
        && existing.title_suppressed == style.title_suppressed
        && existing.header_suppressed == style.header_suppressed
        && row_style_matches(&existing.data_row_style, &style.data_row(), &text_styles[0])
        && row_style_matches(
            &existing.title_row_style,
            &style.row_style(TableRowKind::Title),
            &text_styles[1],
        )
        && row_style_matches(
            &existing.header_row_style,
            &style.row_style(TableRowKind::Header),
            &text_styles[2],
        )
}

/// Compare one written row record on the fields the writer emits.
fn row_style_matches(
    existing: &acadrust::objects::RowCellStyle,
    row: &TableRowStyle,
    text_style: &RowTextStyle,
) -> bool {
    existing.text_height == row.text_height
        && existing
            .text_style_name
            .eq_ignore_ascii_case(&text_style.style_name)
        && i32::from(existing.alignment as i16) == row.alignment.dxf_code()
        && border_matches(&existing.top_border, row.borders.top)
        && border_matches(&existing.right_border, row.borders.right)
        && border_matches(&existing.bottom_border, row.borders.bottom)
        && border_matches(&existing.left_border, row.borders.left)
}

/// Compare only the border fields the writer emits: the visibility. The style
/// object's double-line spacing has no group in this writer, so comparing it
/// would report a difference that no re-import can ever reproduce.
fn border_matches(existing: &TableCellBorder, border: TableBorder) -> bool {
    existing.is_invisible == !border.visible
}

/// Whether a `TABLESTYLE` record is still the one `CadDocument::new` created.
///
/// The document's untouched `Standard` record is the place the first table's
/// snapshot is written; a record that no longer equals those defaults was
/// written by an earlier entity of this export and keeps its values. **All three**
/// row records are checked: a record whose title/header row was written by an
/// earlier table is not "untouched" even when its data row still matches.
fn is_document_default_style(style: &AcadTableStyle) -> bool {
    let default = AcadTableStyle::standard();
    style.name.eq_ignore_ascii_case(&default.name)
        && style.description.is_empty()
        && style.horizontal_margin == default.horizontal_margin
        && style.vertical_margin == default.vertical_margin
        && !style.title_suppressed
        && !style.header_suppressed
        && row_is_record_default(&style.data_row_style, &default.data_row_style)
        && row_is_record_default(&style.title_row_style, &default.title_row_style)
        && row_is_record_default(&style.header_row_style, &default.header_row_style)
}

/// Whether one row record still equals the `CadDocument::new` default for it.
fn row_is_record_default(
    row: &acadrust::objects::RowCellStyle,
    default: &acadrust::objects::RowCellStyle,
) -> bool {
    row.text_height == default.text_height
        && row
            .text_style_name
            .eq_ignore_ascii_case(&default.text_style_name)
        && row.alignment == default.alignment
        && !row.top_border.is_invisible
        && !row.right_border.is_invisible
        && !row.bottom_border.is_invisible
        && !row.left_border.is_invisible
}

/// Copy one neutral cell into the acadrust cell record.
///
/// The borders, the text height/style and the cell alignment are written through
/// a `CellStyle`: with `override_flags == 0` the DXF writer emits the alignment
/// (group 170), the text height/style (groups 140/7), the four edge visibilities
/// (groups 285/286/288/289) and this build's neutral `ByBlock`/no-fill colour
/// values, which is exactly what the model resolved. The cell's own text
/// rotation rides on the cell record itself (group 145). A doubled edge also
/// writes its `double_spacing` and the `Double` border type, so a reader that
/// looks at the record sees the values this build resolved — the DXF **writer**
/// emits no per-cell `GRIDFORMAT`, which the entity's conversion reason states.
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
    target.rotation = cell.rotation;

    let mut style = CellStyle::new();
    style.text_style_name = cell.style_name.clone();
    style.text_height = cell.text_height;
    style.alignment = cell.alignment.dxf_code();
    let apply = |border: TableBorder| {
        let mut out = acadrust::entities::CellBorder::new();
        out.invisible = !border.visible;
        out.double_spacing = border.double_spacing;
        out.border_type = if border.double_spacing > 0.0 {
            acadrust::entities::BorderType::Double
        } else {
            acadrust::entities::BorderType::Single
        };
        out
    };
    style.top_border = apply(cell.borders.top);
    style.right_border = apply(cell.borders.right);
    style.bottom_border = apply(cell.borders.bottom);
    style.left_border = apply(cell.borders.left);
    // The four edges are marked applied because that is the record a writer that
    // emits per-cell borders would need. The DXF writer this build links against
    // emits no per-cell GRIDFORMAT at all, which the entity's conversion reason
    // states; keeping the record complete means a reader that does see a
    // `CellStyle` reads the values this build resolved, not defaults.
    style.applied_border_edges =
        CellEdgeFlags::TOP | CellEdgeFlags::RIGHT | CellEdgeFlags::BOTTOM | CellEdgeFlags::LEFT;
    target.style = Some(style);
}
