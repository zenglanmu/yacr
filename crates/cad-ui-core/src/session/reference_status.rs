//! The authoritative external-reference (XREF) rows of the frozen wire.
//!
//! [`CadReferenceStatus`] is a pure projection of the drawing's own
//! [`cad_db::ExternalReference`] records: every field is copied verbatim, so a
//! missing/refused/cyclic reference stays visible with its real status and
//! reason instead of being hidden or invented. Nothing here reads host state.

use serde::Serialize;

// Referenced only by intra-doc links in this module.
#[allow(unused_imports)]
use super::status::CadStatus;

/// One row of the [`CadStatus::references`] block: the authoritative
/// external-reference (XREF) metadata projected onto the frozen wire.
///
/// Every field is a 1:1 copy of the drawing's [`cad_db::ExternalReference`],
/// never re-derived: `path` is the raw source spelling (`raw_path`),
/// `resolved_path` is only what the host recorded when it loaded the file,
/// `status` is exactly [`cad_db::ExternalReferenceStatus::as_str`] (kebab-case,
/// never inferred from geometry or the name) and `reason` is the stable
/// code/detail the host or importer recorded, never localized prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadReferenceStatus {
    /// The xref block name (the database's reference-map key).
    pub name: String,
    /// The path exactly as it was stored in the source file, before any host
    /// resolution.
    pub path: String,
    /// The display/canonical path the host resolved and accepted, when loaded.
    pub resolved_path: Option<String>,
    /// Stable kebab-case status code (`loaded`, `missing`,
    /// `browser-unavailable`, `refused-path`, `unloaded`, `cycle`,
    /// `depth-exceeded`, `skipped-overlay`, `unsupported`).
    pub status: String,
    /// Stable code/detail for a non-loaded status; never localized prose.
    pub reason: Option<String>,
    /// Whether the source marked this attachment as an overlay.
    pub overlay: bool,
    /// Nesting depth (`0` for an attachment in the host drawing).
    pub depth: u32,
}

impl CadReferenceStatus {
    /// Project one authoritative reference verbatim; no field is guessed.
    pub(super) fn from_reference(reference: &cad_db::ExternalReference) -> Self {
        Self {
            name: reference.name.clone(),
            path: reference.raw_path.clone(),
            resolved_path: reference.resolved_path.clone(),
            // The database's status is the contract: only its own `as_str`
            // spelling is emitted, so the wire can never drift from the enum.
            status: reference.status.as_str().to_string(),
            reason: reference.reason.clone(),
            overlay: reference.overlay,
            depth: reference.depth,
        }
    }
}
