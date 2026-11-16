//! Lossy Save As export: reverse-map the domain database to an acadrust document.
//!
//! This is a **Save As**, never a round-trip save. The domain database is a
//! resolved display model — `ByLayer` **linetype and lineweight** are flattened
//! and several kinds arrive pre-lossy — so the returned [`ExportReport`] records
//! every conversion and every drop explicitly. `ByLayer` **colour** is the
//! exception: it is preserved symbolically on import and written back
//! symbolically, so a layer colour survives the round trip, and block identity
//! is preserved by writing real `BLOCK_RECORD`s and `INSERT` records (see the
//! `insert` module). The caller (host/CLI) owns file I/O; this module only
//! produces bytes.
//!
//! [`ExportFormat::Dwg`] writes the same reverse-mapped document through
//! upstream `acadrust::DwgWriter` instead of `DxfWriter`. That path is
//! **experimental and lossy**, and its only evidence is a synthetic acadrust
//! self-roundtrip — never a claim that AutoCAD or any vendor opens the file.

mod accumulator;
mod common;
mod dimension;
mod entity;

// The export contract test asserts the variant name.
#[cfg(test)]
pub(crate) use entity::kind_name;
mod hatch;
mod insert;
mod multileader;
mod report;
mod solid_record;
mod table;
mod tables;

pub use report::{
    ExportCounts, ExportDiagnostic, ExportFormat, ExportLimits, ExportReport, ExportSpace,
    MappingOutcome,
};

use acadrust::CadDocument;
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{CadError, CadResult, Completeness, DocumentId, SpaceId, Transform3, UnitContext};

use accumulator::ExportAccumulator;
use entity::{emit, EntityContext};

/// A lossy Save As export request.
///
/// v1 exports model space only. The bytes are ASCII DXF text
/// ([`ExportFormat::DxfText`]) or experimental binary DWG
/// ([`ExportFormat::Dwg`]).
pub struct ExportRequest<'a> {
    pub database: &'a DrawingDatabase,
    pub document_id: DocumentId,
    pub space: ExportSpace,
    pub format: ExportFormat,
    pub limits: ExportLimits,
    /// The source unit context (from the host `Document`). `None` leaves
    /// `$INSUNITS` at Unitless and adds a note; coordinates are never rescaled.
    pub units: Option<UnitContext>,
}

/// Reverse-map `request.database` to DXF or DWG bytes plus an explicit report.
pub fn export(request: &ExportRequest<'_>) -> CadResult<(Vec<u8>, ExportReport)> {
    let mut doc = CadDocument::new();
    let mut report = ExportReport {
        format: request.format,
        counts: ExportCounts::default(),
        entries: Vec::new(),
        notes: Vec::new(),
        completeness: Completeness::Complete,
    };

    if request.format == ExportFormat::Dwg {
        // A global note, not an entry: this is not a per-geometry loss and must
        // not turn a clean mapping into `Partial`. It states the evidence
        // boundary once, in the report the caller shows to the user.
        report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.dwg_experimental".into(),
            message: "DWG output is produced by the upstream acadrust DwgWriter from the same \
                      lossy reverse-mapped document as the DXF export; the conversion is \
                      experimental and lossy, and it is NOT verified against AutoCAD or any \
                      vendor (the only evidence is a synthetic acadrust self-roundtrip)"
                .into(),
        });
    }

    write_units(&mut doc, request, &mut report);
    write_point_display_style(&mut doc, request.database);
    write_linetype_scale(&mut doc, request.database, &mut report);
    write_header_points(&mut doc, request.database, &mut report);
    write_scale_list(&mut doc, request.database, &mut report);
    tables::write_tables(&mut doc, request.database, &mut report);

    // Colour, lineweight, linetype and transparency **provenance** is preserved
    // on import, so all four are written back symbolically where the source was
    // symbolic: acadrust omits group 62 for `Color::ByLayer` (the DXF encoding
    // of 256 / by layer), an entity linetype of `BYLAYER` (group 6) plus an
    // omitted group 370 (acadrust writes group 370 only when the weight is not
    // `LineWeight::ByLayer`, whose DXF code is `-1`) and an omitted group 440
    // (acadrust writes group 440 only when the transparency is not
    // `Transparency::ByLayer`) all mean "take the layer's", and the layer table
    // carries the actual colour, linetype, weight and `AcCmTransparency` XDATA.
    // Byte-identical explicit values are written as explicit values, and an
    // unresolved `ByBlock` stays symbolic.
    report.notes.push(ExportDiagnostic {
        object: None,
        code: "export.note.style_resolved".into(),
        message: "ByLayer colour, linetype, lineweight and transparency are written symbolically \
                  (no group 62 = DXF colour 256, group 6 = BYLAYER, group 370 omitted = -1, group 440 \
                  omitted = transparency ByLayer), so a layer colour, linetype, lineweight or \
                  transparency change survives a save/load; explicit entity values are written as \
                  stored; unresolved ByBlock values stay symbolic"
            .into(),
    });

    let ctx = EntityContext {
        database: request.database,
        limits: request.limits,
    };
    let mut acc = ExportAccumulator::new();

    // Model-space entities, emitted in draw order.
    let mut model: Vec<&DbEntity> = request
        .database
        .entities()
        .filter(|e| e.space == SpaceId::Model)
        .collect();
    model.sort_by_key(|e| e.draw_order);
    for entity in model {
        let mut stack = Vec::new();
        emit(
            &mut doc,
            entity,
            &Transform3::identity(),
            0,
            &ctx,
            &mut acc,
            &mut stack,
        )?;
    }

    // Paper-space entities are deferred in v1 and never silently omitted.
    for entity in request.database.entities() {
        if let SpaceId::Paper(_) = entity.space {
            acc.record(
                Some(entity.object.id),
                "paper_space",
                MappingOutcome::Dropped(vec![
                    "paper-space entity export is deferred (v1 exports model space)".into(),
                ]),
            );
        }
    }

    // Block definitions not reached by any written block reference are reported,
    // so a block's content is never silently missing from the output. Definitions
    // that were written as `BLOCK_RECORD`s, or that were reached and exploded,
    // are handled. Xref-owned definitions are a separate case: their members are
    // an external drawing, and an INSERT of one is already refused with
    // `export.xref_content_dropped` (and marked expanded). A definition with no
    // INSERT still must not be called a generic unreferenced block, because the
    // reason it is not written is that v1 cannot emit an xref record/path at all.
    for block in request.database.blocks() {
        if acc.written_blocks.contains_key(&block.id)
            || acc.expanded_blocks.contains(&block.id)
            || acc.refused_blocks.contains_key(&block.id)
        {
            continue;
        }
        if let Some(reference) = request.database.external_reference_for_block(block.id) {
            acc.record_dropped_code(
                None,
                "export.xref_definition_not_written",
                vec![format!(
                    "external reference '{}' (raw path '{}', status '{}') block {:?} is not \
                     written: the v1 DXF exporter emits no xref attachment record or path",
                    reference.name,
                    reference.raw_path,
                    reference.status.as_str(),
                    block.id
                )],
            );
            continue;
        }
        acc.record(
            None,
            "unreferenced_block",
            MappingOutcome::Dropped(vec![format!(
                "unreferenced block definition {:?} is not exported",
                block.id
            )]),
        );
    }

    report.entries.extend(acc.entries);
    report.notes.extend(acc.notes);
    report.counts = acc.counts;

    // Add every mapped entity to the document (handles are allocated here, never
    // fabricated by us) and route it to model space.
    for entity in std::mem::take(&mut acc.entities) {
        doc.add_entity(entity)
            .map_err(|error| CadError::InvalidInput(format!("add entity failed: {error}")))?;
    }

    let bytes = match request.format {
        ExportFormat::DxfText => acadrust::io::dxf::DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(|error| CadError::InvalidInput(format!("DXF write failed: {error}")))?,
        // Upstream acadrust DWG writer. The same reverse-mapped, already-lossy
        // document is the input, so DWG inherits every recorded conversion/drop.
        // Version follows `CadDocument::version` (AC1032 here). Experimental:
        // no vendor/AutoCAD verification exists for this output.
        ExportFormat::Dwg => acadrust::DwgWriter::write_to_vec(&doc)
            .map_err(|error| CadError::InvalidInput(format!("DWG write failed: {error}")))?,
    };
    report.counts.bytes = bytes.len();
    report.finish_completeness();
    Ok((bytes, report))
}

fn write_units(doc: &mut CadDocument, request: &ExportRequest<'_>, report: &mut ExportReport) {
    let _ = request.space;
    match request.units.as_ref().map(|u| &u.source) {
        Some(unit) => match tables::unit_insunits(unit) {
            Some(code) => doc.header.insertion_units = code,
            None => report.notes.push(ExportDiagnostic {
                object: None,
                code: "export.note.units".into(),
                message: format!("source unit {unit:?} has no $INSUNITS code; written as Unitless"),
            }),
        },
        None => report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.units_missing".into(),
            message: "no unit context supplied; $INSUNITS left at Unitless".into(),
        }),
    }
}

/// Carry the drawing-global POINT display style (`$PDMODE` / `$PDSIZE`) into the
/// output document.
///
/// Both values are the database's **raw** header values (the display layer is the
/// single interpreter of the mode bits and the size's sign), and the acadrust DXF
/// writer emits both header variables from these two fields. Without this the
/// exporter wrote the writer's defaults, so a `PDMODE`/`PDSIZE` write — or an
/// imported style — was dropped by a SAVE→OPEN cycle. Nothing is converted and no
/// diagnostic is emitted: this is a preserved value, not a lossy mapping.
///
/// The boundary is still the acadrust writer plus **our own** reader (internal
/// consistency), never a vendor-compatibility claim.
fn write_point_display_style(doc: &mut CadDocument, database: &DrawingDatabase) {
    doc.header.point_display_mode = database.point_display_mode();
    doc.header.point_display_size = database.point_display_size();
}

/// Carry the drawing-global linetype scale (`$LTSCALE`) into the output document.
///
/// The value is the database's stored header value, written verbatim: it is a
/// preserved value, not a lossy mapping, so a `LTSCALE` write — or an imported
/// scale — is not dropped by a SAVE→OPEN cycle. Nothing is converted and, for a
/// representable value, no diagnostic is emitted.
///
/// Representability: the DXF writer emits `$LTSCALE` unconditionally (group 40,
/// double) for the AC1032 output this exporter always requests, so every finite
/// positive scale is representable. Both sanctioned writes into the database —
/// [`DrawingDatabaseBuilder::set_linetype_scale`] and `LTSCALE`'s
/// `DrawingDatabase::set_linetype_scale` — refuse a non-finite or non-positive
/// value, and the importer sanitises a corrupt source header to the DWG default
/// of `1.0`, so no current path can reach the guard below. It is kept as the
/// explicit rule for a future unchecked write: an unrepresentable value is
/// **declared** in the report and the header keeps the DXF default, never
/// written as `0`/`NaN` and never silently swapped.
///
/// [`DrawingDatabaseBuilder::set_linetype_scale`]: cad_db::DrawingDatabaseBuilder::set_linetype_scale
fn write_linetype_scale(
    doc: &mut CadDocument,
    database: &DrawingDatabase,
    report: &mut ExportReport,
) {
    let scale = database.linetype_scale();
    if !scale.is_finite() || scale <= 0.0 {
        report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.linetype_scale".into(),
            message: format!(
                "stored linetype scale {scale} is not a positive finite number, so $LTSCALE is \
                 left at the DXF default 1.0"
            ),
        });
        return;
    }
    doc.header.linetype_scale = scale;
}

/// Carry the drawing header's vector values into the output document:
/// `$INSBASE` (model-space insertion base point), `$LIMMIN` and `$LIMMAX`
/// (model-space limits).
///
/// The database stores these raw and reports `None` until a command writes them
/// (see `cad_db::drawing::header_geometry`), and the acadrust DXF writer emits
/// all three variables unconditionally. A stored value is therefore preserved
/// verbatim — no conversion, no note — exactly like `$LTSCALE`/`$PDMODE`. An
/// **absent** value leaves the writer's default (origin / a zero-size limit)
/// instead of inventing a header the database never held; the `BASE`/`LIMITS`
/// read forms name that absence for the user.
///
/// One boundary is declared here rather than hidden: the limits are two
/// independent header variables, so a half-stored pair (one corner written, the
/// other never set) still writes both variables and the missing corner is the
/// DXF default. That state is reported (`export.note.limits_half_stored`) instead
/// of being called a preserved pair.
///
/// The **import** side of this build does not read `$INSBASE`/`$LIMMIN`/`$LIMMAX`
/// (a separate, still-open read gap), so this function makes a value written in
/// this build survive SAVE→OPEN; it does not restore a value a source file
/// carried.
fn write_header_points(
    doc: &mut CadDocument,
    database: &DrawingDatabase,
    report: &mut ExportReport,
) {
    if let Some(base) = database.insertion_base() {
        doc.header.model_space_insertion_base =
            acadrust::types::Vector3::new(base.x, base.y, base.z);
    }
    if let Some([x, y]) = database.limits_min() {
        doc.header.model_space_limits_min = acadrust::types::Vector2::new(x, y);
    }
    if let Some([x, y]) = database.limits_max() {
        doc.header.model_space_limits_max = acadrust::types::Vector2::new(x, y);
    }
    if database.limits_min().is_some() != database.limits_max().is_some() {
        report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.limits_half_stored".into(),
            message: format!(
                "the drawing stores only one limits corner (model limits {}); $LIMMIN and $LIMMAX                  are both written, so the missing corner is the DXF default rather than a value                  this database held",
                database
                    .model_limits()
                    .map(|(min, max)| format!("{} {} to {} {}", min[0], min[1], max[0], max[1]))
                    .unwrap_or_else(|| "incomplete".to_string())
            ),
        });
    }
}

/// Carry the drawing's named annotation scales (`ACAD_SCALELIST`) into the output
/// document.
///
/// The database owns the table ([`cad_db::DrawingDatabase::scales`], written
/// after import by `SCALELISTEDIT`), and the acadrust DXF writer emits
/// `ObjectType::Scale` records and their dictionaries, so a scale created,
/// renamed or deleted in this build survives SAVE→OPEN. Entries are written in
/// table-id order under `ACAD_SCALELIST` with AutoCAD's `*A<n>` key spelling, and
/// every row keeps its name and its raw paper/drawing pair (nothing is
/// normalised).
///
/// Two declared boundaries:
///
/// * a row whose ratio this build could not have created (a non-finite component
///   from a corrupt source) is **not** written and is named in the report
///   (`export.note.scale_unwritable`) instead of poisoning the OBJECTS section;
/// * the active annotation scale (`CANNOSCALE`) itself is **not** written,
///   because the acadrust DXF writer deliberately skips that header variable for
///   vendor compatibility (see its `$CANNOSCALE` comment). The table round-trips;
///   the header's current name does not, which is why the importer reports an
///   unresolvable one by name instead of guessing.
fn write_scale_list(doc: &mut CadDocument, database: &DrawingDatabase, report: &mut ExportReport) {
    let mut rows: Vec<&cad_db::Scale> = database.scales().collect();
    rows.sort_by_key(|row| row.id.0);
    if rows.is_empty() {
        return;
    }
    let dictionary = match scale_list_dictionary(doc) {
        Some(handle) => handle,
        None => {
            let handle = doc.allocate_handle();
            let root = doc.header.named_objects_dict_handle;
            let mut dict = acadrust::objects::Dictionary::new();
            dict.handle = handle;
            dict.owner = root;
            doc.objects
                .insert(handle, acadrust::objects::ObjectType::Dictionary(dict));
            if let Some(acadrust::objects::ObjectType::Dictionary(root_dict)) =
                doc.objects.get_mut(&root)
            {
                root_dict.add_entry("ACAD_SCALELIST", handle);
            }
            handle
        }
    };
    for (index, row) in rows.into_iter().enumerate() {
        if !row.is_well_formed() {
            report.notes.push(ExportDiagnostic {
                object: None,
                code: "export.note.scale_unwritable".into(),
                message: format!(
                    "annotation scale '{}' has a non-finite ratio ({}/{}), so it is not written to                      ACAD_SCALELIST; the other rows are",
                    row.name, row.paper_units, row.drawing_units
                ),
            });
            continue;
        }
        let handle = doc.allocate_handle();
        let mut scale =
            acadrust::objects::Scale::new(&row.name, row.paper_units, row.drawing_units);
        scale.handle = handle;
        scale.owner_handle = dictionary;
        doc.objects
            .insert(handle, acadrust::objects::ObjectType::Scale(scale));
        if let Some(acadrust::objects::ObjectType::Dictionary(dict)) =
            doc.objects.get_mut(&dictionary)
        {
            dict.add_entry(format!("*A{}", index + 1), handle);
        }
    }
}

/// The `ACAD_SCALELIST` dictionary handle, when the document already has one.
fn scale_list_dictionary(doc: &CadDocument) -> Option<acadrust::types::Handle> {
    let root = doc.header.named_objects_dict_handle;
    let Some(acadrust::objects::ObjectType::Dictionary(root_dict)) = doc.objects.get(&root) else {
        return None;
    };
    root_dict
        .entries
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("ACAD_SCALELIST"))
        .map(|(_, handle)| *handle)
}

#[cfg(test)]
mod tests;
