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
//! * the per-cell appearance **is** written: content colour (group 64),
//!   background fill (63 + 283) and each border edge's colour/lineweight
//!   (65-69, 275-279), plus the same four fields per `TABLESTYLE` row record
//!   (62/63/283 and 64-69/274-279). Colours are stored as sRGB and written back
//!   as the nearest ACI index — the only colour form these groups carry for a
//!   plain TABLE entity — so a palette colour round-trips exactly and a true
//!   colour canonicalises to the nearest palette entry, which the conversion
//!   reason names. A per-border line **type** is not modelled at all;
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
/// Named conversion reason: how a stored colour is written back.
const REASON_CELL_APPEARANCE: &str =
    "cell/row colours are stored as sRGB and written as the nearest ACI index (the form these DXF \
     groups carry for a plain TABLE); a palette colour round-trips exactly, a true colour \
     canonicalises to the nearest palette entry, and a per-border line type is not modelled at all";
/// Named conversion reason: a stored border lineweight is not drawn wider.
const REASON_CELL_LINEWEIGHT: &str =
    "a stored border lineweight is written back exactly (1/100 mm, groups 275-279) but this build \
     draws every line one pixel wide, so the width is a stored/exported value rather than a drawn \
     stroke";
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
    reasons.push(REASON_CELL_LINEWEIGHT.to_string());
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
    target.text_color = dxf_color(row.text_color);
    target.fill_color = dxf_color(row.fill_color);
    target.fill_enabled = row.fill_color.is_some();
}

/// One style-object border from the resolved edge.
fn table_cell_border(border: TableBorder) -> TableCellBorder {
    let mut out = TableCellBorder::new();
    out.is_invisible = !border.visible;
    out.double_line_spacing = border.double_spacing;
    out.color = dxf_color(border.color);
    out.line_weight = dxf_lineweight(border.lineweight_hundredths);
    out
}

/// The DXF colour for one stored sRGB value: the nearest ACI index, or `ByBlock`
/// (the neutral "inherit" value) when the model stores no override.
///
/// The cell/row colour groups carry an ACI index; a true colour would need the
/// paired `420`-`427` groups, which the entity-cell writer does not emit for a
/// plain TABLE. Writing the nearest index keeps the record self-consistent (the
/// style-object writer emits index `7` for any `Color::Rgb`), so the colour is
/// canonicalised here, once, and the conversion reason names it.
fn dxf_color(rgb: Option<[u8; 3]>) -> acadrust::Color {
    match rgb {
        None => acadrust::Color::ByBlock,
        Some([r, g, b]) => acadrust::Color::Index(
            acadrust::Color::Rgb { r, g, b }
                .approximate_index()
                .clamp(1, 255) as u8,
        ),
    }
}

/// The DXF lineweight for a stored 1/100 mm weight: the same integer, or
/// `ByLayer` (the neutral "inherit" value) when the model stores none.
///
/// Written back **verbatim**: the DXF groups 275-279 carry a 1/100 mm integer,
/// so the stored value is the written value and no rounding happens on save.
/// `TableGeometry::new` refuses a weight outside the documented `0..=211` range;
/// an imported value is never silently clamped here.
fn dxf_lineweight(hundredths: Option<u16>) -> acadrust::LineWeight {
    match hundredths {
        None => acadrust::LineWeight::ByLayer,
        Some(value) => acadrust::LineWeight::Value(i16::try_from(value).unwrap_or(i16::MAX)),
    }
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
        && existing.text_color == dxf_color(row.text_color)
        && (existing.fill_enabled, existing.fill_color)
            == (row.fill_color.is_some(), dxf_color(row.fill_color))
        && border_matches(&existing.top_border, row.borders.top)
        && border_matches(&existing.right_border, row.borders.right)
        && border_matches(&existing.bottom_border, row.borders.bottom)
        && border_matches(&existing.left_border, row.borders.left)
}

/// Compare only the border fields the writer emits: visibility, colour and
/// lineweight. The style object's double-line spacing has no group in this
/// writer, so comparing it would report a difference that no re-import can ever
/// reproduce.
fn border_matches(existing: &TableCellBorder, border: TableBorder) -> bool {
    existing.is_invisible == !border.visible
        && existing.color == dxf_color(border.color)
        && existing.line_weight == dxf_lineweight(border.lineweight_hundredths)
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
        && row.text_color == default.text_color
        && row.fill_enabled == default.fill_enabled
        && row.fill_color == default.fill_color
        && !row.top_border.is_invisible
        && !row.right_border.is_invisible
        && !row.bottom_border.is_invisible
        && !row.left_border.is_invisible
        && row.top_border.color == default.top_border.color
        && row.right_border.color == default.right_border.color
        && row.bottom_border.color == default.bottom_border.color
        && row.left_border.color == default.left_border.color
}

/// Copy one cell into the acadrust cell record.
///
/// The borders, the text height/style, the cell alignment and the per-cell
/// appearance are written through a `CellStyle`: with `override_flags == 0` the
/// DXF writer emits the alignment (group 170), the text height/style (groups
/// 140/7), the content colour (64), the background colour and its enabled flag
/// (63/283), the four edge visibilities (285/286/288/289), colours (65-69) and
/// lineweights (275-279), which is exactly what the model resolved (a symbolic
/// `ByBlock` / no-fill value is written for an absent override). The cell's own
/// text rotation rides on the cell record itself (group 145). A doubled edge also
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
    style.content_color = dxf_color(cell.text_color);
    style.background_color = dxf_color(cell.fill_color);
    style.fill_enabled = cell.fill_color.is_some();
    let apply = |border: TableBorder| {
        let mut out = acadrust::entities::CellBorder::new();
        out.invisible = !border.visible;
        out.double_spacing = border.double_spacing;
        out.border_type = if border.double_spacing > 0.0 {
            acadrust::entities::BorderType::Double
        } else {
            acadrust::entities::BorderType::Single
        };
        out.color = dxf_color(border.color);
        out.line_weight = dxf_lineweight(border.lineweight_hundredths);
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

#[cfg(test)]
mod tests {
    //! The per-cell/row appearance round trip: a stored colour, fill, border
    //! colour and border lineweight must be **written as the DXF groups** and
    //! read back unchanged, and a cell/row without an override must write the
    //! neutral `ByBlock`/no-fill/`ByLayer` values (so a table that never set one
    //! keeps its pre-appearance file byte-for-byte).

    use super::*;
    use crate::tests_multileader_fixtures::{export_one, import_bytes};

    const RED: [u8; 3] = [255, 0, 0];
    const YELLOW: [u8; 3] = [255, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const GREEN: [u8; 3] = [0, 255, 0];

    fn cell(text: &str) -> TableCellGeometry {
        TableCellGeometry {
            text: text.to_string(),
            text_height: 2.5,
            style_name: "Standard".into(),
            style: StyleId(0),
            font: None,
            alignment: TableCellAlignment::DEFAULT,
            rotation: 0.0,
            merge_width: 1,
            merge_height: 1,
            borders: TableCellBorders {
                top: TableBorder::VISIBLE,
                right: TableBorder::VISIBLE,
                bottom: TableBorder::VISIBLE,
                left: TableBorder::VISIBLE,
            },
            text_color: None,
            fill_color: None,
        }
    }

    /// A 1 x 2 table whose first cell carries every per-cell appearance field and
    /// whose style carries per-row appearance too. The second cell stays neutral.
    fn styled_table() -> TableGeometry {
        let mut styled = cell("styled");
        styled.text_color = Some(RED);
        styled.fill_color = Some(YELLOW);
        for border in [
            &mut styled.borders.top,
            &mut styled.borders.right,
            &mut styled.borders.bottom,
            &mut styled.borders.left,
        ] {
            border.color = Some(BLUE);
            border.lineweight_hundredths = Some(50);
        }
        let mut style = TableStyleValues::named("Report");
        // Both other row types are suppressed, so every row of this table is a
        // data row: the plain cell inherits the data row's appearance and the
        // title value set is carried only as the style's own record.
        style.title_suppressed = true;
        style.header_suppressed = true;
        style.text_color = Some(GREEN);
        style.fill_color = Some(RED);
        style.borders.top.color = Some(YELLOW);
        style.borders.top.lineweight_hundredths = Some(35);
        let mut title = style.data_row();
        title.text_color = Some(BLUE);
        title.fill_color = None;
        style.set_row_style(TableRowKind::Title, title);
        TableGeometry::from_import_with_style(
            TableParts {
                frame: TableFrame::default(),
                columns: vec![
                    TableColumnGeometry {
                        width: 4.0,
                        name: "A".into(),
                    },
                    TableColumnGeometry {
                        width: 4.0,
                        name: "B".into(),
                    },
                ],
                rows: vec![TableRowGeometry {
                    height: 3.0,
                    cells: vec![styled, cell("plain")],
                }],
                style_name: "Report".into(),
                name: String::new(),
                description: String::new(),
            },
            style,
        )
    }

    fn written_table(bytes: &[u8]) -> acadrust::entities::Table {
        let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
            .expect("reader")
            .read()
            .expect("written DXF parses");
        let table = document
            .entities()
            .find_map(|entity| match entity {
                acadrust::EntityType::Table(table) => Some(table.as_ref().clone()),
                _ => None,
            })
            .expect("one TABLE entity");
        table
    }

    fn written_style(bytes: &[u8], name: &str) -> acadrust::objects::TableStyle {
        let document = acadrust::DxfReader::from_reader(std::io::Cursor::new(bytes.to_vec()))
            .expect("reader")
            .read()
            .expect("written DXF parses");
        let style = document
            .objects
            .iter()
            .find_map(|(_, object)| match object {
                ObjectType::TableStyle(style) if style.name.eq_ignore_ascii_case(name) => {
                    Some(style.clone())
                }
                _ => None,
            })
            .expect("the TABLESTYLE record");
        style
    }

    #[test]
    fn the_per_cell_appearance_is_written_as_its_dxf_groups() {
        let (bytes, report) = export_one(SemanticGeometry::Table(Box::new(styled_table())));
        let written = written_table(&bytes);
        let style = written.rows[0].cells[0]
            .style
            .as_ref()
            .expect("the styled cell keeps a CellStyle record");
        // Group 64 / 63 / 283: content colour, background colour, fill enabled.
        assert_eq!(style.content_color, acadrust::Color::Index(1), "red");
        assert_eq!(style.background_color, acadrust::Color::Index(2), "yellow");
        assert!(style.fill_enabled);
        // Groups 69/275 (top), 65/279 (right), 66/276 (bottom), 68/278 (left).
        for border in [
            &style.top_border,
            &style.right_border,
            &style.bottom_border,
            &style.left_border,
        ] {
            assert_eq!(border.color, acadrust::Color::Index(5), "blue");
            assert_eq!(border.line_weight, acadrust::LineWeight::Value(50));
        }

        // The neutral cell writes the neutral values, so nothing is implied.
        let plain = written.rows[0].cells[1]
            .style
            .as_ref()
            .expect("every cell gets a CellStyle record");
        assert_eq!(plain.content_color, acadrust::Color::ByBlock);
        assert_eq!(plain.background_color, acadrust::Color::ByBlock);
        assert!(!plain.fill_enabled);
        assert_eq!(plain.top_border.color, acadrust::Color::ByBlock);
        assert_eq!(plain.top_border.line_weight, acadrust::LineWeight::ByLayer);

        // The style object carries its row value sets (groups 62/63/283 and
        // 64-69/274-279 of each row record).
        let record = written_style(&bytes, "Report");
        assert_eq!(record.data_row_style.text_color, acadrust::Color::Index(3));
        assert_eq!(record.data_row_style.fill_color, acadrust::Color::Index(1));
        assert!(record.data_row_style.fill_enabled);
        assert_eq!(
            record.data_row_style.top_border.color,
            acadrust::Color::Index(2)
        );
        assert_eq!(
            record.data_row_style.top_border.line_weight,
            acadrust::LineWeight::Value(35)
        );
        assert_eq!(record.title_row_style.text_color, acadrust::Color::Index(5));
        assert!(!record.title_row_style.fill_enabled);

        // The conversion report no longer claims the fields are dropped.
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.message.contains("nearest ACI index")),
            "{report:?}"
        );
    }

    #[test]
    fn the_per_cell_and_per_row_appearance_round_trips() {
        let source = styled_table();
        let (bytes, _report) = export_one(SemanticGeometry::Table(Box::new(source.clone())));
        let reimported = import_bytes(&bytes);
        let entity = reimported
            .model_space()
            .into_iter()
            .find(|entity| entity.object.type_key == "AcDbTable")
            .expect("AcDbTable re-imported");
        let SemanticGeometry::Table(table) = &entity.geometry else {
            panic!("expected a Table variant");
        };

        // Per cell: colour, fill and every edge's colour/lineweight.
        let styled = table.cell(0, 0).expect("styled cell");
        assert_eq!(styled.text_color, Some(RED));
        assert_eq!(styled.fill_color, Some(YELLOW));
        for border in [
            styled.borders.top,
            styled.borders.right,
            styled.borders.bottom,
            styled.borders.left,
        ] {
            assert_eq!(border.color, Some(BLUE));
            assert_eq!(border.lineweight_hundredths, Some(50));
        }
        // The plain cell inherited the data row's appearance (it never had an
        // override of its own), and the inherited values are real values.
        let plain = table.cell(0, 1).expect("plain cell");
        assert_eq!(plain.text_color, Some(GREEN));
        assert_eq!(plain.fill_color, Some(RED));
        assert_eq!(plain.borders.top.color, Some(YELLOW));
        assert_eq!(plain.borders.top.lineweight_hundredths, Some(35));

        // Per style row: the data row and the distinguished title row.
        let style = table.style();
        assert_eq!(style.text_color, Some(GREEN));
        assert_eq!(style.fill_color, Some(RED));
        assert_eq!(style.borders.top.color, Some(YELLOW));
        assert_eq!(style.borders.top.lineweight_hundredths, Some(35));
        let title = style.row_style(TableRowKind::Title);
        assert_eq!(title.text_color, Some(BLUE));
        assert_eq!(title.fill_color, None);
        // The header row never differed, so it stays linked to the data row.
        assert_eq!(style.header_row, None);

        // The derived display follows the re-imported appearance, so the colour
        // is not merely stored.
        let appearances = table.display_appearances();
        assert!(
            appearances
                .iter()
                .any(|appearance| appearance.color == Some(YELLOW)),
            "the fill colour reaches the display: {appearances:?}"
        );
        assert!(appearances
            .iter()
            .any(|appearance| appearance.color == Some(RED)));
    }
}
