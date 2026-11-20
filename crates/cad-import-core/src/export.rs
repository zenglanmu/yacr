//! Format-neutral export contract: job, output, and the compatibility report.
//!
//! This is the writer-side twin of [`crate::Importer`]. A caller hands an
//! [`Exporter`] an [`ExportJob`] (the `cad-db` database it must **read only**,
//! the document identity, units, space and limits) and receives an
//! [`ExportOutput`]: the bytes plus a [`CompatReport`] that names every
//! conversion and every drop.
//!
//! `cad-db` stays the sole authority: nothing here mutates a database, and the
//! report is metadata about the projection, never a second document model.
//! `ExportSpace` is model-space only for now, and that is stated instead of
//! silently omitting paper space.

use cad_db::DrawingDatabase;
use cad_domain::{
    CadResult, Completeness, DocumentId, EntityCapability, ObjectId, SupportStatus, UnitContext,
};

use crate::connector::{ConnectorDescriptor, FidelityLevel, ReportDirection};
use crate::ImportReport;

/// Which drawing space an export covers.
///
/// Only [`ExportSpace::Model`] exists in this build; a format that cannot write
/// paper space must report it as a drop rather than pretend it was covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSpace {
    Model,
}

/// Bounds applied to an export.
///
/// Exceeding any bound is an explicit error, never an out-of-memory abort or a
/// silently truncated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportLimits {
    /// Maximum number of output entities before the export fails explicitly.
    pub max_output_entities: usize,
    /// Maximum INSERT explode depth before the export fails explicitly.
    pub max_explode_depth: usize,
    /// Triangle budget for mesh writers.
    ///
    /// A writer that would emit more triangles than this refuses by name
    /// instead of truncating the mesh or exhausting memory. A format writer
    /// that does not emit triangles ignores the field.
    pub max_output_triangles: usize,
}

impl Default for ExportLimits {
    fn default() -> Self {
        ExportLimits {
            max_output_entities: 1_000_000,
            max_explode_depth: 32,
            max_output_triangles: 50_000_000,
        }
    }
}

/// One export request: the read-only source plus the caller's choices.
pub struct ExportJob<'a> {
    /// The document content. Exporters read it and never mutate it.
    pub database: &'a DrawingDatabase,
    pub document_id: DocumentId,
    /// Source unit context when the host has one. `None` means the format must
    /// leave its unit header unset instead of inventing a unit.
    pub units: Option<&'a UnitContext>,
    pub space: ExportSpace,
    pub limits: ExportLimits,
}

/// Counts of how each source geometry mapped onto the output.
///
/// For an export report these are real geometry counts; an import report has no
/// equivalent tally, so it leaves them at zero and relies on `entries` and
/// `completeness`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompatCounts {
    /// Geometries written without any loss.
    pub exact: usize,
    /// Geometries written with an explicit, reported approximation.
    pub converted: usize,
    /// Geometries omitted with an explicit, reported reason.
    pub dropped: usize,
    /// Total output bytes.
    pub bytes: usize,
}

/// One compatibility record: a capability row, a conversion, a drop or a note.
///
/// `code` is the stable, machine-checkable marker copied **verbatim** from the
/// producing reader/writer (`export.converted.line`, `import.spline.flags_dropped`,
/// `compat.capability`, ...); `message` is the human detail.
#[derive(Debug, Clone, PartialEq)]
pub struct CompatEntry {
    /// Connector id that produced this record (`"dxf"`, `"stl"`, ...).
    pub connector: &'static str,
    pub direction: ReportDirection,
    pub level: FidelityLevel,
    /// Source object when the loss is tied to one.
    pub object: Option<ObjectId>,
    /// Type key when the record describes an entity class.
    pub type_key: Option<String>,
    pub status: SupportStatus,
    /// Stable marker owned by this record; never a formatted message.
    ///
    /// Owned rather than `&'static str` so a diagnostic's own code (a runtime
    /// `String` on [`crate::Diagnostic`]) survives instead of being replaced by
    /// a generic adapter label.
    pub code: String,
    /// Human detail for this record.
    pub message: String,
}

/// The per-object compatibility report of one import or export.
///
/// Not `PartialEq`: `EntityCapability` (from `cad-domain`) has no equality
/// contract, and comparing two reports is not a meaningful operation here.
#[derive(Debug, Clone)]
pub struct CompatReport {
    pub connector: &'static str,
    pub direction: ReportDirection,
    pub level: FidelityLevel,
    pub counts: CompatCounts,
    /// One entry per capability row, conversion, drop and global note.
    pub entries: Vec<CompatEntry>,
    /// Capability rows as the reader/report declared them.
    pub capabilities: Vec<EntityCapability>,
    /// Overall verdict: `Complete` only when nothing was converted or dropped.
    pub completeness: Completeness,
}

/// The bytes a writer produced plus its report.
#[derive(Debug)]
pub struct ExportOutput {
    pub bytes: Vec<u8>,
    /// Always describes the same projection as `bytes`.
    pub report: CompatReport,
}

/// A format writer.
///
/// The descriptor is a `const` owned by the adapter; `format_id` is the
/// registry key the caller asked for (`"stl"` and `"stl-ascii"` are two ids of
/// one descriptor). [`Exporter::export`] receives that concrete key so the
/// written report can name the exact requested variant instead of only the
/// descriptor.
pub trait Exporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor;
    /// The descriptor's default registry key (the extension spelling).
    fn format_id(&self) -> &'static str;
    /// Write `job`.
    ///
    /// `format_id` is the concrete registry key the caller selected; a writer
    /// whose descriptor owns several keys must stamp the report from it.
    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput>;
}

impl ImportReport {
    /// Turn this import report into the shared [`CompatReport`].
    ///
    /// Each capability row becomes one `compat.capability` entry whose status is
    /// the **most severe** of its five columns (so a row can never look better
    /// than its weakest stage) and whose message lists all five. Each diagnostic
    /// becomes one entry whose `code` is the diagnostic's own stable marker,
    /// copied verbatim.
    ///
    /// `completeness` is copied verbatim; `counts` stays zero (an import has no
    /// exported-geometry tally).
    pub fn compat(
        &self,
        connector: &'static str,
        level: FidelityLevel,
        direction: ReportDirection,
    ) -> CompatReport {
        let mut entries = Vec::with_capacity(self.capabilities.len() + self.diagnostics.len());
        for capability in &self.capabilities {
            entries.push(CompatEntry {
                connector,
                direction,
                level,
                object: None,
                type_key: Some(capability.type_key.clone()),
                status: capability_status(capability),
                code: "compat.capability".to_string(),
                message: format!(
                    "{}: read={} semantic={} render={} pick={} measure={}",
                    capability.type_key,
                    status_label(capability.read),
                    status_label(capability.semantic),
                    status_label(capability.render),
                    status_label(capability.pick),
                    status_label(capability.measure),
                ),
            });
        }
        for diagnostic in &self.diagnostics {
            entries.push(CompatEntry {
                connector,
                direction,
                level,
                object: diagnostic.object,
                type_key: None,
                // A diagnostic is a declared boundary; it is never evidence that
                // the affected content imported successfully. `Diagnostic` has no
                // outcome field, so the status is inferred from the stable code
                // (see [`diagnostic_status`]).
                status: diagnostic_status(&diagnostic.code, &self.completeness),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
            });
        }
        CompatReport {
            connector,
            direction,
            level,
            counts: CompatCounts::default(),
            entries,
            capabilities: self.capabilities.clone(),
            completeness: self.completeness.clone(),
        }
    }
}

/// The status of one import diagnostic, inferred from its stable code.
///
/// **This is a coarse mapping, not an outcome record.** `cad_domain::Diagnostic`
/// has no outcome field (it is a code plus a message), so the only stable signal
/// available here is the code's own namespace. The rule is deliberately closed
/// and documented rather than heuristic:
///
/// * a code containing one of [`LOSSY_CODE_SUFFIXES`] names content that was
///   dropped, refused, unreadable or never imported → [`SupportStatus::Unsupported`];
/// * otherwise, when the **report-level** completeness is `Partial`/`Missing`,
///   the diagnostic belongs to an import that already declared a loss →
///   [`SupportStatus::Partial`];
/// * otherwise the diagnostic is informational → [`SupportStatus::Unverified`].
///
/// The report's `completeness` remains the authoritative verdict: this function
/// only grades one entry and can never make an import look more complete. A new
/// diagnostic code that does not follow this namespace stays `Partial`/
/// `Unverified` instead of being guessed at.
fn diagnostic_status(code: &str, completeness: &Completeness) -> SupportStatus {
    if LOSSY_CODE_SUFFIXES
        .iter()
        .any(|suffix| code.contains(suffix))
    {
        return SupportStatus::Unsupported;
    }
    match completeness {
        Completeness::Partial(_) | Completeness::Missing(_) => SupportStatus::Partial,
        Completeness::Complete | Completeness::Unverified => SupportStatus::Unverified,
    }
}

/// The closed set of code suffixes that name a real content loss.
const LOSSY_CODE_SUFFIXES: [&str; 8] = [
    ".degenerate",
    ".dropped",
    ".refused",
    ".failed",
    ".too_few",
    ".not_loaded",
    ".not_imported",
    ".unreadable",
];

/// Severity rank used to pick the governing status of one capability row.
fn severity(status: SupportStatus) -> u8 {
    match status {
        SupportStatus::Verified => 0,
        SupportStatus::Partial => 1,
        SupportStatus::Unverified => 2,
        SupportStatus::NotImplemented => 3,
        SupportStatus::Unsupported => 4,
    }
}

/// The most severe of the five capability columns.
fn capability_status(capability: &EntityCapability) -> SupportStatus {
    [
        capability.read,
        capability.semantic,
        capability.render,
        capability.pick,
        capability.measure,
    ]
    .into_iter()
    .max_by_key(|status| severity(*status))
    .unwrap_or(SupportStatus::Unverified)
}

/// Lower-case wire-stable spelling of a support status (report text only).
fn status_label(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::Verified => "verified",
        SupportStatus::Partial => "partial",
        SupportStatus::Unverified => "unverified",
        SupportStatus::NotImplemented => "not_implemented",
        SupportStatus::Unsupported => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DatabaseId, Diagnostic, DocumentId, DocumentIdentity};

    fn capability(type_key: &str, all: SupportStatus, read: SupportStatus) -> EntityCapability {
        EntityCapability {
            type_key: type_key.into(),
            read,
            semantic: all,
            render: all,
            pick: all,
            measure: all,
        }
    }

    #[test]
    fn diagnostic_status_comes_from_the_code_namespace_and_the_report_verdict() {
        // A code in the closed lossy namespace is a real loss, even when the
        // report itself would otherwise look clean.
        assert_eq!(
            diagnostic_status("import.mesh.degenerate", &Completeness::Complete),
            SupportStatus::Unsupported
        );
        assert_eq!(
            diagnostic_status("import.xref.not_loaded", &Completeness::Complete),
            SupportStatus::Unsupported
        );
        // An informational code on a complete import stays unverified.
        assert_eq!(
            diagnostic_status("import.unit.assumed", &Completeness::Complete),
            SupportStatus::Unverified
        );
        // The same informational code inside a partial import inherits the
        // report's partial verdict instead of claiming more.
        assert_eq!(
            diagnostic_status("import.unit.assumed", &Completeness::Partial(vec![])),
            SupportStatus::Partial
        );
    }

    #[test]
    fn compat_maps_capabilities_and_diagnostics_and_keeps_completeness() {
        let report = ImportReport {
            identity: DocumentIdentity::Sha256([0; 32]),
            capabilities: vec![
                capability("AcDbLine", SupportStatus::Verified, SupportStatus::Verified),
                capability(
                    "AcDbSpline",
                    SupportStatus::Partial,
                    SupportStatus::Verified,
                ),
            ],
            completeness: Completeness::Partial(vec!["spline flags".into()]),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(7)),
                code: "import.spline.flags_dropped".into(),
                message: "flags dropped".into(),
            }],
            connector: None,
            parse_ms: None,
            object_xdata: Vec::new(),
        };
        let compat = report.compat("dxf", FidelityLevel::L1, ReportDirection::Import);
        assert_eq!(compat.connector, "dxf");
        assert_eq!(compat.direction, ReportDirection::Import);
        assert_eq!(compat.level, FidelityLevel::L1);
        assert_eq!(compat.completeness, report.completeness);
        assert_eq!(compat.capabilities.len(), 2);
        assert_eq!(compat.entries.len(), 3);
        assert_eq!(compat.entries[0].status, SupportStatus::Verified);
        // The weakest column governs the row.
        assert_eq!(compat.entries[1].status, SupportStatus::Partial);
        assert_eq!(compat.entries[0].code, "compat.capability");
        let diagnostic = &compat.entries[2];
        // The diagnostic's own stable marker survives verbatim; the adapter must
        // not replace it with a generic label.
        assert_eq!(diagnostic.code, "import.spline.flags_dropped");
        assert_eq!(diagnostic.message, "flags dropped");
        assert_eq!(diagnostic.object, Some(ObjectId(7)));
    }

    #[test]
    fn export_limits_defaults_bound_entities_depth_and_triangles() {
        let limits = ExportLimits::default();
        assert_eq!(limits.max_output_entities, 1_000_000);
        assert_eq!(limits.max_explode_depth, 32);
        assert_eq!(limits.max_output_triangles, 50_000_000);
    }

    #[test]
    fn a_job_borrows_the_database_read_only() {
        // Compile-level contract: `ExportJob` holds a shared reference, so no
        // exporter can mutate the authority it was handed.
        use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
        use cad_domain::{EntityId, LayerId, Point3, Revision, SemanticGeometry, SpaceId};
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
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                draw_order: 0,
            })
            .unwrap();
        let database = builder.finish().unwrap();
        let job = ExportJob {
            database: &database,
            document_id: DocumentId(1),
            units: None,
            space: ExportSpace::Model,
            limits: ExportLimits::default(),
        };
        assert_eq!(job.space, ExportSpace::Model);
        assert_eq!(job.database.entity_count(), 1);
    }
}
