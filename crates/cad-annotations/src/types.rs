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
