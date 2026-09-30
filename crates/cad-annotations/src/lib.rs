//! Versioned sidecar format and annotation business rules, never DWG writing.
use cad_db::{Annotation, AnnotationDatabase, ChangeSet};
use cad_domain::*;
use std::collections::BTreeMap;
pub const SCHEMA_VERSION: u32 = 1;
pub struct ViewBookmark {
    pub name: String,
    pub viewport: ViewportId,
    pub space: SpaceId,
    pub camera_transform: Transform3,
}
pub struct AnnotationFile {
    pub schema_version: u32,
    pub application_version: String,
    pub document_fingerprint: DocumentIdentity,
    pub document_name_hint: String,
    pub unit_context: UnitContext,
    pub annotations: Vec<Annotation>,
    pub view_bookmarks: Vec<ViewBookmark>,
    /// Raw JSON for unknown fields; future codec must validate and preserve it.
    pub extensions_json: BTreeMap<String, String>,
}
pub enum FingerprintPolicy {
    RejectMismatch,
    ExplicitCoordinateMapping(Transform3),
    ImportUnanchored,
}
pub enum AnnotationCommand {
    Create(Annotation),
    Update(Annotation),
    Delete(AnnotationId),
}
pub struct AnnotationService;
impl AnnotationService {
    pub fn apply(
        &self,
        _database: &mut AnnotationDatabase,
        _command: AnnotationCommand,
    ) -> CadResult<ChangeSet> {
        pending("annotations.command_via_transaction")
    }
    pub fn decode(
        &self,
        _json: &[u8],
        _identity: &DocumentIdentity,
        _policy: FingerprintPolicy,
    ) -> CadResult<AnnotationFile> {
        pending("annotations.decode_and_migrate")
    }
    pub fn encode(&self, _file: &AnnotationFile) -> CadResult<Vec<u8>> {
        pending("annotations.encode")
    }
}
