//! Lossy-export report types.
//!
//! This is a *Save As* report, not a round-trip-save report: the domain database
//! is a resolved display model (ByLayer colours flattened, block names and base
//! points dropped, several kinds arriving pre-lossy), so every conversion and
//! every drop is recorded explicitly here rather than silently swallowed.

use cad_domain::{Completeness, ObjectId};

/// Which drawing space is exported. v1 exports model space only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSpace {
    Model,
}

/// The serialisation format. v1 writes ASCII DXF text only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    DxfText,
}

impl ExportFormat {
    /// Lower-case file extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::DxfText => "dxf",
        }
    }

    /// Stable machine name for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::DxfText => "dxf-text",
        }
    }
}

/// Bounds applied to an export.
///
/// Exceeding either bound is an explicit [`cad_domain::CadError`], mirroring
/// [`crate::ImportLimits`] — never an OOM or a silently truncated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportLimits {
    /// Maximum number of output entities before the export fails explicitly.
    pub max_output_entities: usize,
    /// Maximum INSERT explode depth before the export fails explicitly.
    pub max_explode_depth: usize,
}

impl Default for ExportLimits {
    fn default() -> Self {
        ExportLimits {
            max_output_entities: 1_000_000,
            max_explode_depth: 32,
        }
    }
}

/// Counts of how each source geometry mapped onto the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportCounts {
    /// Geometries written without any loss.
    pub exact: usize,
    /// Geometries written with an explicit, reported approximation.
    pub converted: usize,
    /// Geometries omitted with an explicit, reported reason.
    pub dropped: usize,
    /// Total output bytes.
    pub bytes: usize,
}

/// How one source geometry mapped onto the DXF output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingOutcome {
    /// Written faithfully.
    Exact,
    /// Written with one or more explicit approximations.
    Converted(Vec<String>),
    /// Omitted with one or more explicit reasons.
    Dropped(Vec<String>),
}

impl MappingOutcome {
    pub fn is_exact(&self) -> bool {
        matches!(self, MappingOutcome::Exact)
    }

    pub fn is_dropped(&self) -> bool {
        matches!(self, MappingOutcome::Dropped(_))
    }
}

/// One explicit conversion/drop record, or a global note.
///
/// `code` is stable and machine-checkable (for example `export.dropped.shape`);
/// `message` is the human detail. `object` names the source object when known.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDiagnostic {
    pub object: Option<ObjectId>,
    pub code: String,
    pub message: String,
}

/// The lossy-export report.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    pub format: ExportFormat,
    pub counts: ExportCounts,
    /// One entry per converted or dropped geometry, in output order.
    pub entries: Vec<ExportDiagnostic>,
    /// Global notes (ByLayer flattening, unit fallback, spline flags, ...).
    pub notes: Vec<ExportDiagnostic>,
    /// `Complete` **only** when nothing was converted and nothing was dropped.
    pub completeness: Completeness,
}

impl ExportReport {
    /// Build the completeness verdict from the recorded entries.
    pub(crate) fn finish_completeness(&mut self) {
        let converted = self.counts.converted;
        let dropped = self.counts.dropped;
        if converted == 0 && dropped == 0 {
            self.completeness = Completeness::Complete;
            return;
        }
        let reasons: Vec<String> = self.entries.iter().map(|e| e.message.clone()).collect();
        self.completeness = if dropped > 0 {
            Completeness::Missing(reasons)
        } else {
            Completeness::Partial(reasons)
        };
    }
}
