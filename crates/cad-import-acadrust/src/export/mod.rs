//! Lossy Save As export: reverse-map the domain database to an acadrust document.
//!
//! This is a **Save As**, never a round-trip save. The domain database is a
//! resolved display model — `ByLayer` **linetype and lineweight** are flattened,
//! block names and base points are dropped, and several kinds arrive pre-lossy —
//! so the returned [`ExportReport`] records every conversion and every drop
//! explicitly. `ByLayer` **colour** is the exception: it is preserved
//! symbolically on import and written back symbolically, so a layer colour
//! survives the round trip. The caller (host/CLI) owns file I/O; this module only
//! produces bytes.
//!
//! [`ExportFormat::Dwg`] writes the same reverse-mapped document through
//! upstream `acadrust::DwgWriter` instead of `DxfWriter`. That path is
//! **experimental and lossy**, and its only evidence is a synthetic acadrust
//! self-roundtrip — never a claim that AutoCAD or any vendor opens the file.

mod common;
mod dimension;
mod entity;

// The export contract test asserts the variant name.
#[cfg(test)]
pub(crate) use entity::kind_name;
mod hatch;
mod multileader;
mod report;
mod tables;

pub use report::{
    ExportCounts, ExportDiagnostic, ExportFormat, ExportLimits, ExportReport, ExportSpace,
    MappingOutcome,
};

use acadrust::CadDocument;
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{CadError, CadResult, Completeness, DocumentId, SpaceId, Transform3, UnitContext};

use entity::{emit, EntityContext, ExportAccumulator};

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

    // Block definitions not reached by any INSERT expansion are reported, so a
    // block's content is never silently missing from the output. Xref-owned
    // definitions are a separate case: their members are an external drawing,
    // and an INSERT of one is already refused with `export.xref_content_dropped`
    // (and marked expanded). A definition with no INSERT still must not be
    // called a generic unreferenced block, because the reason it is not written
    // is that v1 cannot emit an xref record/path at all.
    for block in request.database.blocks() {
        if acc.expanded_blocks.contains(&block.id) {
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

#[cfg(test)]
mod tests;
