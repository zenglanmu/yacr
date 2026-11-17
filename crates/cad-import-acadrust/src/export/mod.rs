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
//!
//! # ACIS payload and DXF container
//!
//! [`ExportRequest::acis_version`] selects the payload header version written
//! into a `3DSOLID` (ACIS 7.0 / SAT 700, the default, or ACIS 21.0 / ASM 21800)
//! and [`ExportRequest::dxf_target`] the DXF container (`R2010`, `R2013`,
//! `R2018`). The two are not free: the matrix is refused **before any writer
//! runs** with a stable code (`export.acis_asm_requires_r2013`,
//! `export.dwg_version_fixed`), and the accepted combinations are:
//!
//! * DXF `R2010` + ACIS 7.0 — inline SAT text (the historical output);
//! * DXF `R2013`/`R2018` + either payload version — a binary SAB/ASM blob in the
//!   `ACDSDATA` section;
//! * DWG (`AC1032` always, so `R2018` is the only accepted target) + either
//!   payload version — a binary SAB/ASM blob in the AcDs section.
//!
//! **Evidence boundary:** the ASM 21800 payload and every `AC1027`+ container
//! are read back only by our own ACIS reader. No third-party CAD application was
//! used to open them, so they are not a vendor-compatibility claim.

mod accumulator;
mod acis;
mod body;
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

use acadrust::entities::acis::SatVersion;
use acadrust::{CadDocument, DxfVersion};
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{CadError, CadResult, Completeness, DocumentId, SpaceId, Transform3, UnitContext};
use serde::{Deserialize, Serialize};

use accumulator::ExportAccumulator;
use entity::{emit, EntityContext};

/// The ACIS payload version this exporter writes.
///
/// `V7_0` (SAT/SAB version number 700) is the historical default and the only
/// payload version this exporter wrote before the selector existed. `V21_0`
/// (21800) is ShapeManager's ASM encoding: `SabWriter` switches its terminator
/// and ShapeManager flag bits, and a DXF can only carry it as a binary SAB blob
/// in `ACDSDATA` (AC1027 / R2013 or later).
///
/// The version changes the **encoding** of the payload, never which bodies are
/// writable: the planar single-shell subset and every `acis.*` refusal code are
/// the same for both versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AcisVersion {
    /// ACIS 7.0 / SAT version 700 (inline SAT text before R2013, SAB after).
    #[default]
    #[serde(rename = "7.0")]
    V7_0,
    /// ACIS 21.0 / ASM version 21800 (SAB only, `ACDSDATA` in a DXF).
    #[serde(rename = "21.0")]
    V21_0,
}

impl AcisVersion {
    /// The `acadrust` SAT version this choice writes.
    ///
    /// ACIS 21.0's header number is `21800`, i.e. `SatVersion` fields
    /// `(218, 0, 0)`. Upstream's `SatVersion::V21_0` constant is `(21, 0, 0)`
    /// whose `sat_version_number()` is `2100` — below the `>= 21800` threshold
    /// `SabWriter` uses to select the ASM encoding — so the ASM header is built
    /// from the documented number instead of that constant.
    pub fn sat_version(self) -> SatVersion {
        match self {
            AcisVersion::V7_0 => SatVersion::V7_0,
            AcisVersion::V21_0 => SatVersion::from_sat_number(21800),
        }
    }

    /// The SAT/SAB header version number (700 / 21800).
    pub fn number(self) -> u32 {
        self.sat_version().sat_version_number()
    }

    /// The wire/UI label (`"7.0"` / `"21.0"`): also the persisted token.
    pub fn label(self) -> &'static str {
        match self {
            AcisVersion::V7_0 => "7.0",
            AcisVersion::V21_0 => "21.0",
        }
    }
}

/// The DXF container version the exporter writes (`$ACADVER`).
///
/// AC1024 (R2010) is the historical default and carries an ACIS 7.0 payload as
/// inline SAT text. AC1027 (R2013) and AC1032 (R2018) carry **any** ACIS payload
/// as a binary SAB blob in the `ACDSDATA` section, which is what makes an ASM
/// 21800 payload expressible at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DxfTarget {
    /// R2010 / AC1024.
    #[default]
    #[serde(rename = "2010")]
    R2010,
    /// R2013 / AC1027.
    #[serde(rename = "2013")]
    R2013,
    /// R2018 / AC1032.
    #[serde(rename = "2018")]
    R2018,
}

impl DxfTarget {
    /// The `acadrust` DXF version this choice writes.
    pub fn dxf_version(self) -> DxfVersion {
        match self {
            DxfTarget::R2010 => DxfVersion::AC1024,
            DxfTarget::R2013 => DxfVersion::AC1027,
            DxfTarget::R2018 => DxfVersion::AC1032,
        }
    }

    /// The wire/UI label (`"2010"` / `"2013"` / `"2018"`): also the persisted
    /// token.
    pub fn label(self) -> &'static str {
        match self {
            DxfTarget::R2010 => "2010",
            DxfTarget::R2013 => "2013",
            DxfTarget::R2018 => "2018",
        }
    }

    /// Whether this container stores ACIS modeler geometry as a binary SAB blob
    /// in `ACDSDATA` (`AC1027`/R2013 and later) instead of inline SAT text.
    pub fn stores_sab(self) -> bool {
        self != DxfTarget::R2010
    }
}

/// The host-settable export choices (ACIS payload version + DXF container).
///
/// Deliberately **not** document state: this is a Save-As option, so it never
/// touches `cad-db`, has no revision and no history record. The default is the
/// pre-selector behaviour (`ACIS 7.0` in a `R2010` DXF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportOptions {
    pub acis_version: AcisVersion,
    pub dxf_target: DxfTarget,
}

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
    /// The ACIS payload version to write (SAT/SAB header version 700 / 21800).
    pub acis_version: AcisVersion,
    /// The DXF container version (`$ACADVER`). A DWG is always AC1032 because
    /// the DWG writer derives its version from the document; a `Dwg` request
    /// with any other target is refused with `export.dwg_version_fixed`.
    pub dxf_target: DxfTarget,
}

/// Reverse-map `request.database` to DXF or DWG bytes plus an explicit report.
pub fn export(request: &ExportRequest<'_>) -> CadResult<(Vec<u8>, ExportReport)> {
    // The container/payload matrix is refused here, before a single document
    // record or writer is touched: a refused combination never degrades to
    // another version and never falls back to POLYFACE.
    if request.format == ExportFormat::DxfText
        && request.dxf_target == DxfTarget::R2010
        && request.acis_version == AcisVersion::V21_0
    {
        return Err(CadError::InvalidInput(format!(
            "export.acis_asm_requires_r2013: an ACIS {} (ASM, version {}) payload cannot be \
             written as inline SAT text in a pre-R2013 DXF; choose dxfVersion 2013 or 2018",
            request.acis_version.label(),
            request.acis_version.number()
        )));
    }
    if request.format == ExportFormat::Dwg && request.dxf_target != DxfTarget::R2018 {
        return Err(CadError::InvalidInput(format!(
            "export.dwg_version_fixed: a DWG is written as AC1032 (R2018) by the upstream DWG \
             writer, whose version comes from the document, so dxfVersion {} does not apply; \
             choose 2018",
            request.dxf_target.label()
        )));
    }
    let mut doc = CadDocument::with_version(request.dxf_target.dxf_version());
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
        // The format only picks the ACIS payload encoding for an exact body; it
        // no longer decides *whether* one is written. R2010 DXF carries ACIS 7.0
        // SAT text inline; R2013/R2018 DXF carries the chosen ACIS version as a
        // binary SAB blob in `ACDSDATA`; DWG carries it in the AcDs section. A
        // body outside the planar single-shell subset falls back to its POLYFACE
        // display cache in every combination, under the ACIS writer's own
        // `acis.*_not_written` code.
        format: request.format,
        acis_version: request.acis_version,
        dxf_target: request.dxf_target,
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
    // fabricated by us) and route it to model space. The accumulator already
    // knows whether a `3DSOLID` was queued — including a body emitted inside a
    // block definition and drained into its `BLOCK_RECORD` below, which never
    // passes through this loop.
    let has_solid3d = acc.written_solid3d;
    for entity in std::mem::take(&mut acc.entities) {
        doc.add_entity(entity)
            .map_err(|error| CadError::InvalidInput(format!("add entity failed: {error}")))?;
    }

    // A `3DSOLID` carries its ACIS payload in one of two container forms, and the
    // report names the exact combination instead of leaving the choice implicit:
    //
    // * R2010 (AC1024): inline SAT text, which only ACIS 7.0 can be (an ASM 21800
    //   payload is refused by the matrix at the top of this function);
    // * R2013/R2018 (AC1027/AC1032): a binary SAB blob of the chosen ACIS version
    //   in `ACDSDATA`.
    //
    // The version is a document-wide header, so it is pinned here for a `3DSOLID`
    // in model space *or* in a written block definition.
    //
    // This is a **DXF rule only**. A DWG is always AC1032 (the DWG writer derives
    // its version from the document) precisely so the upstream writer emits the
    // AcDs section, and its payload is the binary SAB blob; the pin must never
    // fire for DWG.
    if request.format == ExportFormat::DxfText && has_solid3d {
        doc.version = request.dxf_target.dxf_version();
        if request.dxf_target.stores_sab() {
            report.notes.push(ExportDiagnostic {
                object: None,
                code: "export.note.acis_sab_dxf".into(),
                message: format!(
                    "a 3DSOLID was written with its ACIS {acis} payload as a binary SAB blob \
                     (SAB header version {number}) in the DXF {container} ACDSDATA section, which \
                     is where {container} stores modeler geometry. The payload is rebuilt from the \
                     retained exact B-Rep, not the original SAT/SAB byte stream (source records, \
                     history and ACIS attributes are not retained), and no third-party CAD reader \
                     was run to verify it: only this exporter's own ACIS reader round-trips the \
                     file",
                    acis = request.acis_version.label(),
                    number = request.acis_version.number(),
                    container = request.dxf_target.dxf_version().as_str(),
                ),
            });
        } else {
            report.notes.push(ExportDiagnostic {
                object: None,
                code: "export.note.acis_sat_text_version".into(),
                message: format!(
                    "a 3DSOLID was written with an inline ACIS {acis} SAT text payload (SAT header \
                     version {number}), so the whole file is written as {container} (R2010): from \
                     AC1027 (R2013) on the payload moves into a binary SAB blob in ACDSDATA, and \
                     R2010 carries SAT text only; no third-party CAD reader was run to verify the \
                     payload: only this exporter's own ACIS reader round-trips it",
                    acis = request.acis_version.label(),
                    number = request.acis_version.number(),
                    container = request.dxf_target.dxf_version().as_str(),
                ),
            });
        }
    }

    // The DWG counterpart of the note above: the payload is there, it is binary,
    // and it lives in the AcDs section rather than inline. It names the chosen
    // ACIS payload version and states the same provenance/evidence boundaries.
    //
    // Derivation, and why it is sound rather than a guess: this is decided from
    // (format, a `3DSOLID` was queued), not from acadrust's
    // `needs_acds_section()`, which is `version.r2013_plus(dxf_version)` with
    // `dxf_version == CadDocument::version` (`object_writer/mod.rs:178,570`).
    // A DWG is written from the document this function built: `CadDocument::new()`
    // is AC1032, the matrix above only admits `DxfTarget::R2018` for a DWG and the
    // pin above is `DxfText`-only, so no path lowers a DWG's version — AC1032 is
    // `r2013_plus`, hence `needs_acds_section()` is true whenever this note fires,
    // and the writer's `write_solid3d` then always takes the `write_acis_empty` +
    // `queue_sab_entry` branch (`entities.rs:5187-5198`).
    // Deriving from the bytes instead would not be cheaper: the report is built
    // before the writer runs, and the read-back tests assert the reader's
    // "AcDs: attached N SAB blob(s)" notification, which fails if this ever drifts.
    if request.format == ExportFormat::Dwg && has_solid3d {
        report.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.acis_sab_dwg".into(),
            message: format!(
                "a 3DSOLID was written with its ACIS {acis} payload as a binary SAB blob (SAB \
                 header version {number}) in the DWG AC1032 AcDs (AcDsPrototype_1b) section, which \
                 is where R2013 and later store modeler geometry. The payload is rebuilt from the \
                 retained exact B-Rep, not the original SAT/SAB byte stream (source records, \
                 history and ACIS attributes are not retained), and no third-party CAD reader was \
                 run to verify it: only this exporter's own ACIS reader round-trips the file",
                acis = request.acis_version.label(),
                number = request.acis_version.number(),
            ),
        });
    }

    let bytes = match request.format {
        ExportFormat::DxfText => acadrust::io::dxf::DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(|error| CadError::InvalidInput(format!("DXF write failed: {error}")))?,
        // Upstream acadrust DWG writer. The same reverse-mapped, already-lossy
        // document is the input, so DWG inherits every recorded conversion/drop.
        // The version stays the document's own AC1032 (the DXF target above is a
        // DXF-only choice: a DWG carries ACIS in the AcDs section, and the matrix
        // at the top of this function admits only R2018 for a DWG).
        // Experimental: no vendor/AutoCAD verification exists for this output.
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
/// double) for the version this exporter requests (AC1032, or AC1024 when a
/// `3DSOLID` is written so its ACIS payload stays inline SAT text), so every
/// finite positive scale is representable. Both sanctioned writes into the database —
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
