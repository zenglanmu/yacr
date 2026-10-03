//! Public sidecar types and the annotation service handle.
//!
//! Spec v2.0 §3.4, §16.3: annotations live in their own versioned JSON bound to
//! a content fingerprint, are only mutated through the annotation database's
//! transaction path, and preserve unknown fields so a newer writer's data
//! survives an older reader.

use std::collections::BTreeMap;

use cad_db::Annotation;
use cad_domain::*;

pub const SCHEMA_VERSION: u32 = 1;

/// Oldest annotation schema version this build can deterministically migrate to
/// [`SCHEMA_VERSION`].
///
/// No historical schema has shipped yet, so this equals [`SCHEMA_VERSION`]:
/// anything lower has no defined migration and is reported as
/// [`CadError::CorruptData`] rather than being guessed at.
pub const MIN_SCHEMA_VERSION: u32 = 1;

/// Reserved top-level key inside [`AnnotationFile::extensions_json`] that
/// carries the preserved unknown fields of nested annotation objects.
///
/// It is a private side-band: a decoded file exposes it in `extensions_json`,
/// but [`AnnotationService::encode`] expands it back into the annotation
/// objects instead of emitting it as an ordinary top-level field. A reader that
/// does not understand it still preserves it verbatim, so nested unknown data
/// survives the default decode/encode path.
pub const NESTED_EXTENSIONS_KEY: &str = "yacr.nested_extensions";

/// Version of the [`NESTED_EXTENSIONS_KEY`] side-band payload.
pub const NESTED_EXTENSIONS_VERSION: u32 = 1;

/// Top-level schema fields that extension data must never override.
pub(crate) const KNOWN_TOP_LEVEL: [&str; 7] = [
    "schema_version",
    "application_version",
    "document_fingerprint",
    "document_name_hint",
    "unit_context",
    "annotations",
    "view_bookmarks",
];

/// Unknown JSON fields preserved for one annotation and its direct nested
/// objects.
///
/// Each value is raw JSON text, exactly like [`AnnotationFile::extensions_json`],
/// so it round-trips verbatim. A key that collides with a known field of the
/// same object is refused on encode ([`CadError::InvalidInput`]) rather than
/// silently dropped; a value that is not valid JSON is
/// [`CadError::CorruptData`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnnotationExtensions {
    /// Unknown keys on the annotation object itself.
    pub annotation: BTreeMap<String, String>,
    /// Unknown keys on the nested `geometry` object.
    pub geometry: BTreeMap<String, String>,
    /// Unknown keys on the nested `style` object.
    pub style: BTreeMap<String, String>,
    /// Unknown keys on the annotation-level `precision` object.
    pub precision: BTreeMap<String, String>,
    /// Unknown keys on a measurement geometry's nested `precision` object.
    pub measurement_precision: BTreeMap<String, String>,
}

impl AnnotationExtensions {
    /// Whether no unknown nested field was preserved.
    pub fn is_empty(&self) -> bool {
        self.annotation.is_empty()
            && self.geometry.is_empty()
            && self.style.is_empty()
            && self.precision.is_empty()
            && self.measurement_precision.is_empty()
    }
}

/// Unknown fields of every annotation in a file, keyed by annotation id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NestedExtensions {
    pub annotations: BTreeMap<AnnotationId, AnnotationExtensions>,
}

impl NestedExtensions {
    /// Whether no annotation preserved an unknown nested field.
    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewBookmark {
    pub name: String,
    pub viewport: ViewportId,
    pub space: SpaceId,
    pub camera_transform: Transform3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationFile {
    pub schema_version: u32,
    pub application_version: String,
    pub document_fingerprint: DocumentIdentity,
    pub document_name_hint: String,
    pub unit_context: UnitContext,
    pub annotations: Vec<Annotation>,
    pub view_bookmarks: Vec<ViewBookmark>,
    /// Raw JSON for unknown top-level fields, preserved verbatim.
    pub extensions_json: BTreeMap<String, String>,
    /// Unknown fields preserved inside each annotation's nested JSON objects,
    /// keyed by annotation id.
    ///
    /// The private top-level [`NESTED_EXTENSIONS_KEY`] side-band in
    /// `extensions_json` is the wire representation; this map is the typed view.
    /// On encode the map is authoritative; on decode both are populated. A
    /// plain `decode` → `encode` round trip is therefore lossless.
    pub nested_extensions: NestedExtensions,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FingerprintPolicy {
    RejectMismatch,
    ExplicitCoordinateMapping(Transform3),
    ImportUnanchored,
}

/// A deterministic, diagnosable account of how a decoded file's fingerprint was
/// reconciled with the open drawing (audit B10).
///
/// Decoding never silently discards a mismatch: the caller can inspect this
/// report (via [`AnnotationService::decode_with_report`]) to explain an import
/// before committing it.
#[derive(Clone, Debug, PartialEq)]
pub struct FingerprintReport {
    /// The fingerprint stored in the sidecar.
    pub file_fingerprint: DocumentIdentity,
    /// The identity of the drawing the file is being decoded against.
    pub open_fingerprint: DocumentIdentity,
    /// Whether the two fingerprints were identical.
    pub matched: bool,
    /// Whether the returned file was rebound to the open drawing's identity
    /// (true exactly when a mismatch was accepted by policy).
    pub rebound: bool,
    /// The policy that was applied.
    pub policy: FingerprintPolicy,
    /// Number of anchors that were detached because the drawing did not match.
    pub anchors_detached: usize,
    /// Whether annotation geometry was transformed by an explicit mapping.
    pub geometry_transformed: bool,
    /// Number of view bookmarks transformed by an explicit mapping.
    pub bookmarks_transformed: usize,
}

/// The decoded file together with the fingerprint reconciliation report.
#[derive(Clone, Debug)]
pub struct DecodeOutcome {
    pub file: AnnotationFile,
    pub fingerprint: FingerprintReport,
}

pub enum AnnotationCommand {
    Create(Annotation),
    Update(Annotation),
    Delete(AnnotationId),
}

#[derive(Default)]
pub struct AnnotationService;
