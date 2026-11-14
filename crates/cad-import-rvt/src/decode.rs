//! Unit 1: Revit bytes -> the GLB `rvt` emits, plus its honest coverage facts.
//!
//! Every API used here was read from the `rvt` **0.22.0** sources, not guessed:
//!
//! * `RevitFile::open_bytes_with_limits` (`src/reader.rs:468`) consumes a
//!   `Vec<u8>` and rejects anything that is not an OLE/CFB container whose
//!   first eight bytes are `D0 CF 11 E0 A1 B1 1A E1`.
//! * `ifc::run_export` (`src/ifc/export_pipeline.rs:535`) builds the
//!   [`ExportRun`]: the `IfcModel`, its `ExportDiagnostics` (including
//!   `input.release_support` and `confidence`), the streams a resource limit
//!   left unread, and the step notes. `ExportOptions::default()` is `rvt-ifc`
//!   with no switch.
//! * `ifc::gltf::model_to_glb` (`src/ifc/gltf.rs:814`) emits a single-file GLB.
//!   The element nodes are in **feet with +Z up**; one root node's matrix
//!   [`rvt::ifc::gltf::Z_UP_FEET_TO_Y_UP_METRES`] (`src/ifc/gltf.rs:242`)
//!   scales by `0.3048` and swaps the up axis, so the GLB itself is glTF's
//!   **metres, +Y up**. That is the units finding recorded in the ADR and in
//!   [`crate::DIAG_UNITS_METRES`].
//!
//! Family files are refused here, by bytes rather than by extension: the
//! `ProjectInformation` stream exists only on project files (`rvt`
//! `src/project_information.rs:1-3`: "project files only … Families carry
//! their Atom metadata as plain XML in `PartAtom`"), and a family's `PartAtom`
//! carries an `<A:family>` type table. The stable refusal is
//! [`crate::FAMILY_NO_GEOMETRY`].

use cad_domain::{CadError, CadResult};
use cad_import_core::ImportLimits;
use rvt::ifc::gltf::model_to_glb;
use rvt::ifc::{run_export, ExportOptions, SourceCoverageStatus};
use rvt::reader::OpenLimits;
use rvt::streams::{PART_ATOM, PROJECT_INFORMATION};
use rvt::RevitFile;

use crate::{
    DECODE_FAILED, FAMILY_NO_GEOMETRY, NOT_A_REVIT_FILE, RELEASE_UNSUPPORTED, RVT_OPEN_LIMITS,
};

/// What a container turns out to be once its streams are inspected.
///
/// This is deliberately a byte-level classification: the import contract
/// carries bytes, not a path, so the `.rfa`/`.rft` extension is not available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// A project or project template: it carries `ProjectInformation`.
    Project,
    /// A family/family template: metadata, previews and types, no geometry.
    Family,
    /// Neither signal was readable (a project missing `ProjectInformation`, or
    /// a damaged family). It is decoded as a project: if it has no bodies, the
    /// caller refuses with [`crate::NO_GEOMETRY`] instead of guessing.
    Unknown,
}

/// Classify an opened document from two independent byte-level signals.
///
/// A non-empty family type table (`PartAtom`'s `<A:family>` entries) is direct
/// family evidence, as is an `<A:family>` element in the raw Atom XML. Only
/// when neither is present does `ProjectInformation` decide the answer.
pub fn classify_document(
    has_project_information: bool,
    family_type_count: usize,
    atom_declares_family_table: bool,
) -> DocumentKind {
    if family_type_count > 0 || atom_declares_family_table {
        DocumentKind::Family
    } else if has_project_information {
        DocumentKind::Project
    } else {
        DocumentKind::Unknown
    }
}

/// Whether an Atom document declares an `<A:family>` type table.
///
/// `rvt` writes the family type table as `<A:family>` (namespace prefix `A`).
/// Both prefixed and unprefixed spellings are accepted; the match is
/// case-insensitive because XML prefixes are case-sensitive but the upstream
/// writer has used both `A` and `a` spellings across samples.
pub fn part_atom_declares_family_table(xml: &str) -> bool {
    let lower = xml.to_ascii_lowercase();
    lower.contains("<a:family") || lower.contains("<family")
}

/// The result of unit 1: GLB bytes plus the facts the report must not lose.
#[derive(Debug, Clone)]
pub struct RvtDecode {
    /// Single-file GLB. Empty geometry is still a valid, parseable GLB (one
    /// root node, no meshes); the caller decides whether that is a refusal.
    pub glb: Vec<u8>,
    pub revit_version: Option<u32>,
    /// `rvt`'s release status: `verified`, `partial`, `experimental`,
    /// `unsupported` or `unknown`, from its built-in support matrix.
    pub release_status: String,
    pub release_capability: Option<String>,
    /// `ExportConfidenceSummary.level` (`scaffold`, `proxy_only`,
    /// `typed_no_geometry`, `diagnostic_partial`, `geometry`).
    pub confidence_level: String,
    pub confidence_score: f32,
    /// Whether any exported building element carries geometry.
    pub has_geometry: bool,
    /// `source_coverage.status`: `unset`, `unknown` or `measured`.
    pub coverage_status: String,
    /// Fractions are only present when `rvt` had a trustworthy denominator.
    pub coverage_decoded: Option<f32>,
    pub coverage_exported: Option<f32>,
    pub coverage_geometry: Option<f32>,
    pub coverage_notes: Option<String>,
    pub building_elements: usize,
    pub geometry_elements: usize,
    pub unread_streams: usize,
    /// `(reason, count)` pairs from `ExportDiagnostics::skipped`.
    pub skipped: Vec<(String, usize)>,
    pub unsupported_features: Vec<String>,
    pub warning_count: usize,
    /// `ExportNote` display strings, in step order.
    pub notes: Vec<String>,
}

/// Decode Revit bytes and emit the GLB `rvt` would write.
///
/// A family (`DocumentKind::Family`) and an unsupported release are refusals,
/// not empty successes. `cancelled` is checked before and after the expensive
/// `rvt` run; it cannot interrupt inside `run_export` (0.22.0 exposes
/// cancellation only through its own `WalkerControl`, which has no adapter for
/// an external `Fn` here).
/// Streams whose absence is fatal, given what the container turned out to be.
///
/// `rvt` declares twelve "invariant" streams, but `PartAtom` is a **family**
/// metadata stream: a real project saves `ProjectInformation` and may omit
/// `PartAtom` entirely. The Autodesk Revit 2024 basic sample project does
/// exactly that, and demanding all twelve wrongly rejected it as
/// `not_a_revit_file`.
///
/// Everything else stays fatal: a container missing `Global`/`ElemTable`/
/// `PartitionTable` really is not a readable Revit document. The decision is
/// made from bytes only — the import contract carries no path, so the
/// `.rvt`/`.rfa` extension cannot be consulted.
pub fn fatal_missing_streams(missing: Vec<&'static str>, is_project: bool) -> Vec<&'static str> {
    missing
        .into_iter()
        .filter(|name| !(is_project && *name == PART_ATOM))
        .collect()
}

pub fn decode_to_glb(
    bytes: Vec<u8>,
    limits: &ImportLimits,
    cancelled: &dyn Fn() -> bool,
) -> CadResult<RvtDecode> {
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let open_limits = OpenLimits {
        max_file_bytes: limits.max_file_bytes as u64,
        ..OpenLimits::default()
    };
    let mut file = RevitFile::open_bytes_with_limits(bytes, open_limits).map_err(map_open_error)?;

    let missing = fatal_missing_streams(
        file.missing_required_streams(),
        file.stream_names()
            .iter()
            .any(|name| name == PROJECT_INFORMATION),
    );
    if !missing.is_empty() {
        return Err(CadError::CorruptData(format!(
            "{NOT_A_REVIT_FILE}: the OLE container is missing the invariant Revit stream(s) {}; \
             it has a compound-file signature but is not a readable Revit document",
            missing.join(", ")
        )));
    }

    match classify_document(
        file.stream_names()
            .iter()
            .any(|name| name == PROJECT_INFORMATION),
        family_type_count(&mut file),
        part_atom_declares_family_table(&part_atom_xml(&mut file)),
    ) {
        DocumentKind::Family => {
            return Err(CadError::Unsupported(format!(
                "{FAMILY_NO_GEOMETRY}: this is a Revit family / family template (.rfa/.rft): it \
                 carries metadata, previews and types but no family geometry, so there is nothing \
                 to display; use the .rvt project file that places the family"
            )));
        }
        DocumentKind::Project | DocumentKind::Unknown => {}
    }

    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let run = run_export(&mut file, &ExportOptions::default()).map_err(|error| match error {
        rvt::Error::Cancelled => CadError::Cancelled,
        rvt::Error::TooLarge { what, size, limit } => CadError::InvalidInput(format!(
            "{RVT_OPEN_LIMITS}: rvt reading {what} needs {size} bytes, over the {limit} byte limit"
        )),
        other => CadError::CorruptData(format!(
            "{DECODE_FAILED}: rvt 0.22.0 could not export this file: {other}"
        )),
    })?;
    if cancelled() {
        return Err(CadError::Cancelled);
    }

    let release = run.diagnostics.input.release_support.clone();
    let release_status = release
        .as_ref()
        .map(|support| support.status.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let release_capability = release.and_then(|support| support.capability);
    if release_status == "unsupported" {
        return Err(CadError::Unsupported(format!(
            "{RELEASE_UNSUPPORTED}: rvt 0.22.0 has no element-record reader for Revit release {} \
             (supported range 2008..=2027); the file is refused rather than shown as an empty model",
            run.diagnostics
                .input
                .revit_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )));
    }

    let glb = model_to_glb(&run.model);
    if !glb.starts_with(b"glTF") {
        return Err(CadError::Invariant(
            "rvt 0.22.0's model_to_glb returned bytes without the glTF magic".into(),
        ));
    }

    let coverage = run.diagnostics.source_coverage.as_ref();
    let coverage_status = coverage
        .map(|coverage| match coverage.status {
            SourceCoverageStatus::Unset => "unset",
            SourceCoverageStatus::Unknown => "unknown",
            SourceCoverageStatus::Measured => "measured",
            _ => "unknown",
        })
        .unwrap_or("unknown")
        .to_string();
    let skipped = run
        .diagnostics
        .skipped
        .iter()
        .map(|item| (item.reason.clone(), item.count))
        .collect();
    Ok(RvtDecode {
        glb,
        revit_version: run.diagnostics.input.revit_version,
        release_status,
        release_capability,
        confidence_level: run.diagnostics.confidence.level.clone(),
        confidence_score: run.diagnostics.confidence.score,
        has_geometry: run.diagnostics.confidence.has_geometry,
        coverage_status,
        coverage_decoded: coverage.and_then(|coverage| coverage.decoded_element_fraction),
        coverage_exported: coverage.and_then(|coverage| coverage.exported_element_fraction),
        coverage_geometry: coverage.and_then(|coverage| coverage.geometry_element_fraction),
        coverage_notes: coverage.and_then(|coverage| coverage.notes.clone()),
        building_elements: run.diagnostics.exported.building_elements,
        geometry_elements: run.diagnostics.exported.building_elements_with_geometry,
        unread_streams: run.unread.len(),
        skipped,
        unsupported_features: run.diagnostics.unsupported_features.clone(),
        warning_count: run.diagnostics.warnings.len(),
        // `anonymized()` replaces the project text `RoomPhaseFallback` can carry.
        notes: run
            .notes
            .iter()
            .map(|note| note.anonymized().to_string())
            .collect(),
    })
}

/// The number of family types `PartAtom` declares, or zero when it cannot be
/// read (which is not evidence of a family).
fn family_type_count(file: &mut RevitFile) -> usize {
    match file.part_atom() {
        Ok(atom) => atom.family_types().map(|types| types.len()).unwrap_or(0),
        Err(_) => 0,
    }
}

/// The raw Atom XML, or an empty string when the stream cannot be parsed.
fn part_atom_xml(file: &mut RevitFile) -> String {
    file.part_atom()
        .map(|atom| atom.raw_xml)
        .unwrap_or_default()
}

fn map_open_error(error: rvt::Error) -> CadError {
    match error {
        rvt::Error::Cancelled => CadError::Cancelled,
        rvt::Error::TooLarge { what, size, limit } => CadError::InvalidInput(format!(
            "{RVT_OPEN_LIMITS}: rvt reading {what} needs {size} bytes, over the {limit} byte limit"
        )),
        rvt::Error::NotACfbFile => CadError::CorruptData(format!(
            "{NOT_A_REVIT_FILE}: the input does not start with the OLE/CFB magic \
             D0 CF 11 E0 A1 B1 1A E1"
        )),
        other => CadError::CorruptData(format!(
            "{DECODE_FAILED}: rvt 0.22.0 could not open the container: {other}"
        )),
    }
}
