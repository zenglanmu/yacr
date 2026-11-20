//! Adapter from the shared export contract onto the existing DXF/DWG writer.
//!
//! The writer itself ([`super::export`]) is unchanged: this module only builds
//! its [`ExportRequest`](super::ExportRequest) from a shared
//! [`ExportJob`](cad_import_core::ExportJob) and converts the writer's own
//! [`ExportReport`](super::ExportReport) into the format-neutral
//! [`CompatReport`](cad_import_core::CompatReport).
//!
//! # Per-entry status comes from the recorded outcome, never from the code
//!
//! The legacy report records one [`ExportDiagnostic`](super::ExportDiagnostic)
//! per converted or dropped geometry plus its global notes, and keeps the
//! *counts* separately. Each diagnostic now carries the
//! [`ExportOutcome`](super::ExportOutcome) of the decision that produced it, so
//! this adapter reads the status straight from that field:
//!
//! * [`ExportOutcome::Converted`] → `Partial` (a reported approximation);
//! * [`ExportOutcome::Dropped`] → `Unsupported` (a named loss);
//! * [`ExportOutcome::Note`] → `Unverified` (a provenance fact or an evidence
//!   boundary, not a per-geometry verdict).
//!
//! A code namespace cannot carry this. The ACIS payload writer's own `acis.*`
//! codes name a conversion when a refused body is written as its display mesh
//! (`export/body.rs`) and a drop when it has no display geometry at all;
//! `export.xclip_*` codes are drops while `export.converted.*` codes are
//! conversions. The outcome is therefore recorded at the decision site and is
//! copied verbatim here; this module never re-derives it from the string.
//!
//! Status is **per entry** and never considers the report's aggregate counts: a
//! conversion sharing a report with an unrelated drop must not be upgraded to
//! `Unsupported`. `Exact` geometries emit no diagnostic at all; they exist only
//! in `counts.exact`, so no entry is ever `Verified`.

use cad_domain::{CadError, CadResult, SupportStatus};
use cad_import_core::{
    CompatCounts, CompatEntry, CompatReport, Connector, ConnectorDescriptor, ExportCapability,
    ExportJob, ExportOutput, ExportSpace as CoreExportSpace, Exporter, FidelityLevel,
    ImportCapability, ReportDirection,
};

use super::{
    export, AcisVersion, DxfTarget, ExportFormat, ExportLimits, ExportOutcome, ExportReport,
    ExportRequest, ExportSpace,
};

/// Descriptor of the DXF connector.
///
/// Import versions are the container range the reader accepts; export versions
/// are the `$ACADVER` targets the exporter can write. Both are documentation
/// strings and are never parsed. `lossy` is `true` because this is a **Save As**
/// of the resolved display model: colour/linetype/lineweight provenance is
/// preserved symbolically, but every other conversion and drop is reported.
pub const DESCRIPTOR_DXF: ConnectorDescriptor = ConnectorDescriptor {
    id: "dxf",
    name_key: "connector.dxf.name",
    extensions: &["dxf"],
    import: Some(ImportCapability {
        versions: "AC1009..AC1032 (R12-R2018)",
        container: "dxf",
    }),
    export: Some(ExportCapability {
        versions: "AC1024/AC1027/AC1032 (R2010/R2013/R2018)",
        level: FidelityLevel::L1,
        lossy: true,
    }),
    declared_classes: &[
        "AcDbLine",
        "AcDbPolyline",
        "AcDbCircle",
        "AcDbArc",
        "AcDbEllipse",
        "AcDbSpline",
        "AcDbText",
        "AcDbMText",
        "AcDbHatch",
        "AcDbDimension",
        "AcDbBlockReference",
        "AcDb3dSolid",
    ],
};

/// Descriptor of the DWG connector.
///
/// The reader covers the AC1014..AC1032 containers this repository has build
/// evidence for. The writer is the upstream `acadrust::DwgWriter`, always
/// AC1032, and is **experimental**: the only evidence is a synthetic acadrust
/// self-roundtrip, never a vendor opening the file.
pub const DESCRIPTOR_DWG: ConnectorDescriptor = ConnectorDescriptor {
    id: "dwg",
    name_key: "connector.dwg.name",
    extensions: &["dwg"],
    import: Some(ImportCapability {
        versions: "AC1014..AC1032",
        container: "dwg",
    }),
    export: Some(ExportCapability {
        versions: "AC1032 (R2018), experimental",
        level: FidelityLevel::L1,
        lossy: true,
    }),
    declared_classes: DESCRIPTOR_DXF.declared_classes,
};

/// The existing DXF/DWG writer behind the shared [`Exporter`] contract.
///
/// Held by value: it is the ACIS payload version plus the DXF container choice
/// (the same two fields as `ExportOptions`), not document state.
pub struct AcadrustExporter {
    format: ExportFormat,
    acis_version: AcisVersion,
    dxf_target: DxfTarget,
}

impl AcadrustExporter {
    /// Build an exporter for `format` with the caller's payload/container choice.
    pub fn new(format: ExportFormat, acis_version: AcisVersion, dxf_target: DxfTarget) -> Self {
        AcadrustExporter {
            format,
            acis_version,
            dxf_target,
        }
    }

    /// The container this exporter writes.
    pub fn format(&self) -> ExportFormat {
        self.format
    }
}

impl Connector for AcadrustExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        descriptor_for(self.format)
    }
}

impl Exporter for AcadrustExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        descriptor_for(self.format)
    }

    fn format_id(&self) -> &'static str {
        self.format.extension()
    }

    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput> {
        // The registry key must name the container this exporter actually
        // writes. Accepting another key would stamp a descriptor into the report
        // that does not describe the bytes, so the mismatch is refused by name.
        if format_id != self.format.extension() {
            return Err(CadError::Unsupported(format!(
                "export.{format_id}.mismatched_descriptor: exporter writes {}",
                self.format.extension()
            )));
        }
        // The shared job and the writer's request are deliberately mapped field
        // by field: no limit or unit is invented, and `None` units stay `None`
        // (the writer then leaves `$INSUNITS` unset and says so in a note).
        let space = match job.space {
            CoreExportSpace::Model => ExportSpace::Model,
        };
        let limits = ExportLimits {
            max_output_entities: job.limits.max_output_entities,
            max_explode_depth: job.limits.max_explode_depth,
        };
        let (bytes, report) = export(&ExportRequest {
            database: job.database,
            document_id: job.document_id,
            space,
            format: self.format,
            limits,
            units: job.units.cloned(),
            acis_version: self.acis_version,
            dxf_target: self.dxf_target,
        })?;
        Ok(ExportOutput {
            bytes,
            report: compat_report(format_id, &report),
        })
    }
}

/// The descriptor of one writer container.
fn descriptor_for(format: ExportFormat) -> &'static ConnectorDescriptor {
    match format {
        ExportFormat::DxfText => &DESCRIPTOR_DXF,
        ExportFormat::Dwg => &DESCRIPTOR_DWG,
    }
}

/// Convert the writer's own report into the shared one.
///
/// `connector` is the validated registry key (`"dxf"`/`"dwg"`), the same string
/// the exporter's descriptor declares as its id.
fn compat_report(connector: &'static str, report: &ExportReport) -> CompatReport {
    let mut entries = Vec::with_capacity(report.entries.len() + report.notes.len());
    for entry in &report.entries {
        entries.push(CompatEntry {
            connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: entry.object,
            type_key: None,
            status: entry_status(entry.outcome),
            // The writer's own stable marker is the contract and is copied
            // verbatim; status came only from the recorded outcome.
            code: entry.code.clone(),
            message: entry.message.clone(),
        });
    }
    for note in &report.notes {
        entries.push(CompatEntry {
            connector,
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: note.object,
            type_key: None,
            // A note states a provenance fact or an evidence boundary. It is not
            // a verified conversion and not a loss, so it does not change the
            // verdict below.
            status: entry_status(note.outcome),
            code: note.code.clone(),
            message: note.message.clone(),
        });
    }
    CompatReport {
        connector,
        direction: ReportDirection::Export,
        level: FidelityLevel::L1,
        counts: CompatCounts {
            exact: report.counts.exact,
            converted: report.counts.converted,
            dropped: report.counts.dropped,
            bytes: report.counts.bytes,
        },
        entries,
        // The writer reports per-geometry outcomes, not an entity capability
        // table; an empty list is honest, not "everything verified".
        capabilities: Vec::new(),
        completeness: report.completeness.clone(),
    }
}

/// Status for one writer diagnostic, from its recorded outcome.
///
/// The report's aggregate drop count is deliberately not consulted: one drop
/// elsewhere must never relabel a conversion as `Unsupported`.
fn entry_status(outcome: ExportOutcome) -> SupportStatus {
    match outcome {
        ExportOutcome::Converted => SupportStatus::Partial,
        ExportOutcome::Dropped => SupportStatus::Unsupported,
        ExportOutcome::Note => SupportStatus::Unverified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::accumulator::ExportAccumulator;
    use crate::export::report::ExportDiagnostic;
    use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        Completeness, DatabaseId, DocumentId, EntityId, LayerId, ObjectId, Point3, Revision,
        SemanticGeometry, SpaceId,
    };
    use cad_import_core::ExportLimits as CoreExportLimits;

    fn one_line_db() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(1),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                draw_order: 0,
            })
            .unwrap();
        builder.finish().unwrap()
    }

    /// Build the writer's own report from a hand-driven accumulator, so the
    /// status tests run against the exact record the writer emits without a
    /// full export.
    fn report_from(acc: &mut ExportAccumulator) -> ExportReport {
        let mut report = ExportReport {
            format: ExportFormat::DxfText,
            counts: acc.counts,
            entries: std::mem::take(&mut acc.entries),
            notes: std::mem::take(&mut acc.notes),
            completeness: Completeness::Complete,
        };
        report.finish_completeness();
        report
    }

    #[test]
    fn status_follows_the_recorded_outcome() {
        assert_eq!(
            entry_status(ExportOutcome::Converted),
            SupportStatus::Partial
        );
        assert_eq!(
            entry_status(ExportOutcome::Dropped),
            SupportStatus::Unsupported
        );
        assert_eq!(entry_status(ExportOutcome::Note), SupportStatus::Unverified);
    }

    /// `export/body.rs` reports the ACIS refusal code as a **drop** on the
    /// empty-body path and as a **conversion** when the same body is written as
    /// its display mesh. The status must follow the recorded outcome, so a drop
    /// is never `Partial` merely because its code is also used for conversions.
    #[test]
    fn a_dropped_entry_is_unsupported_even_when_it_shares_a_converted_code() {
        const CODE: &str = "acis.curved_face_not_written";
        let mut acc = ExportAccumulator::new();
        acc.record_dropped_code(
            Some(ObjectId(7)),
            CODE,
            vec!["3D body has no display mesh or edge polylines to write".into()],
        );
        acc.record_converted_code(
            Some(ObjectId(8)),
            CODE,
            vec!["body written as its display polyface mesh".into()],
        );
        let report = report_from(&mut acc);
        assert_eq!(report.counts.dropped, 1);
        assert_eq!(report.counts.converted, 1);

        let compat = compat_report("dxf", &report);
        assert!(compat.counts.dropped > 0);
        let status_of = |object: ObjectId| {
            compat
                .entries
                .iter()
                .find(|entry| entry.object == Some(object))
                .unwrap_or_else(|| panic!("no entry for {object:?}"))
                .status
        };
        let dropped = status_of(ObjectId(7));
        assert_eq!(dropped, SupportStatus::Unsupported);
        assert_ne!(dropped, SupportStatus::Partial);
        assert_eq!(status_of(ObjectId(8)), SupportStatus::Partial);
        assert!(
            compat
                .entries
                .iter()
                .any(|entry| entry.status == SupportStatus::Unsupported),
            "a report with counts.dropped > 0 must carry an Unsupported entry"
        );
    }

    #[test]
    fn notes_are_unverified() {
        let mut acc = ExportAccumulator::new();
        acc.notes.push(ExportDiagnostic {
            object: None,
            code: "export.note.units_missing".into(),
            message: "no unit context supplied".into(),
            outcome: ExportOutcome::Note,
        });
        let report = report_from(&mut acc);
        let compat = compat_report("dxf", &report);
        assert_eq!(compat.entries.len(), 1);
        assert_eq!(compat.entries[0].status, SupportStatus::Unverified);
        // A note is not a loss, so it never moves the aggregate verdict.
        assert_eq!(compat.counts.dropped, 0);
        assert_eq!(compat.counts.converted, 0);
    }

    #[test]
    fn a_mismatched_format_id_is_refused_by_name() {
        let database = one_line_db();
        let exporter = AcadrustExporter::new(
            ExportFormat::DxfText,
            AcisVersion::default(),
            DxfTarget::default(),
        );
        let error = exporter
            .export(
                "dwg",
                &ExportJob {
                    database: &database,
                    document_id: DocumentId(1),
                    units: None,
                    space: CoreExportSpace::Model,
                    limits: CoreExportLimits::default(),
                },
            )
            .expect_err("a DXF exporter must not accept the DWG registry key");
        match error {
            CadError::Unsupported(message) => assert_eq!(
                message,
                "export.dwg.mismatched_descriptor: exporter writes dxf"
            ),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn dxf_export_produces_bytes_and_a_shared_report() {
        let database = one_line_db();
        let exporter = AcadrustExporter::new(
            ExportFormat::DxfText,
            AcisVersion::default(),
            DxfTarget::default(),
        );
        let output = exporter
            .export(
                "dxf",
                &ExportJob {
                    database: &database,
                    document_id: DocumentId(1),
                    units: None,
                    space: CoreExportSpace::Model,
                    limits: CoreExportLimits::default(),
                },
            )
            .expect("a one-line model exports");
        assert!(!output.bytes.is_empty(), "the writer produced no bytes");
        assert_eq!(output.report.connector, "dxf");
        assert_eq!(output.report.direction, ReportDirection::Export);
        assert_eq!(output.report.level, FidelityLevel::L1);
        assert_eq!(output.report.counts.exact, 1);
        assert_eq!(output.report.counts.dropped, 0);
        assert_eq!(output.report.counts.bytes, output.bytes.len());
        assert!(
            !matches!(output.report.completeness, Completeness::Unverified),
            "the completeness verdict is carried over, not invented"
        );
        // `units: None` must stay a declared boundary, not a silent unit.
        assert!(
            output
                .report
                .entries
                .iter()
                .any(|entry| entry.code == "export.note.units_missing"),
            "the missing unit context must be reported with its own stable code"
        );
        for entry in &output.report.entries {
            assert_eq!(entry.connector, "dxf");
            assert_eq!(entry.direction, ReportDirection::Export);
        }
        assert_eq!(exporter.format_id(), "dxf");
        // Both `Connector` and `Exporter` expose `descriptor`; the name is the
        // same on purpose, so the test names the trait explicitly.
        assert_eq!(Exporter::descriptor(&exporter).id, "dxf");
        assert_eq!(Connector::descriptor(&exporter).id, "dxf");
    }

    #[test]
    fn dwg_export_reports_the_experimental_gap_instead_of_silence() {
        let database = one_line_db();
        let exporter =
            AcadrustExporter::new(ExportFormat::Dwg, AcisVersion::default(), DxfTarget::R2018);
        let output = exporter
            .export(
                "dwg",
                &ExportJob {
                    database: &database,
                    document_id: DocumentId(1),
                    units: None,
                    space: CoreExportSpace::Model,
                    limits: CoreExportLimits::default(),
                },
            )
            .expect("a one-line model exports");
        assert!(!output.bytes.is_empty());
        assert_eq!(output.report.connector, "dwg");
        assert!(
            output
                .report
                .entries
                .iter()
                .any(|entry| entry.code == "export.note.dwg_experimental"),
            "the DWG evidence boundary must stay in the report"
        );
    }
}
