//! External-reference (XREF) metadata and provenance.
//!
//! Spec v2.0 §7.3, §16.2: an external reference is a drawing-level attachment,
//! not geometry. The importer records what the source declared and what the host
//! decided; nothing in this module performs I/O or path resolution. The actual
//! path policy lives in `cad-resources::check_xref_path` and the host is the
//! only layer allowed to read a referenced file.
//!
//! The metadata is import-only: it is populated through
//! [`crate::DrawingDatabaseBuilder::insert_external_reference`], never through a
//! [`crate::DrawingTransaction`], and therefore never raises the revision or
//! appears in a [`crate::ChangeSet`]. Once built, the database exposes it
//! read-only like every other table.

use cad_domain::BlockId;

/// Outcome of trying to make one external reference available.
///
/// The status is a **contract**, not a placeholder: a reference that could not
/// be loaded keeps its actual reason instead of being silently reported as
/// loaded or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReferenceStatus {
    /// The referenced drawing was found and its content is attached.
    Loaded,
    /// The referenced file does not exist under any allowed root.
    Missing,
    /// This runtime has no filesystem/network access for external references
    /// (for example a browser/wasm carrier without a host bridge).
    BrowserUnavailable,
    /// The reference failed the lexical path policy (unsafe/out-of-root path).
    RefusedPath,
    /// The reference exists but was deliberately not loaded (for example an
    /// unloaded xref the user has disabled).
    Unloaded,
    /// Loading this reference would form a reference cycle.
    Cycle,
    /// Loading this reference would exceed the nested-reference depth limit.
    DepthExceeded,
    /// An overlay reference that was skipped because overlays are not resolved
    /// in this context.
    SkippedOverlay,
    /// The reference uses a feature this build cannot decode (unsupported
    /// version, encrypted content, ...).
    Unsupported,
}

impl ExternalReferenceStatus {
    /// Stable wire/diagnostic spelling. Never localized prose.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Missing => "missing",
            Self::BrowserUnavailable => "browser-unavailable",
            Self::RefusedPath => "refused-path",
            Self::Unloaded => "unloaded",
            Self::Cycle => "cycle",
            Self::DepthExceeded => "depth-exceeded",
            Self::SkippedOverlay => "skipped-overlay",
            Self::Unsupported => "unsupported",
        }
    }

    /// Whether the reference's content is attached to the database.
    ///
    /// Only [`Self::Loaded`] is true; every other status is an explicit
    /// non-loaded state and must stay visible in diagnostics.
    pub fn is_loaded(self) -> bool {
        matches!(self, Self::Loaded)
    }
}

/// Import-time metadata of one external reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalReference {
    /// The xref block name (the key in the database's reference map).
    pub name: String,
    /// The path exactly as it was read from the source file, before any
    /// trimming, folding or host resolution. Kept verbatim so a refusal can be
    /// diagnosed against the original spelling.
    pub raw_path: String,
    /// The lexically normalized path the importer accepted for this reference
    /// (the result of `cad_resources::check_xref_path`). It is **not** a
    /// filesystem-canonical path: the importer never canonicalizes, so a
    /// symlink or `..`-free relative spelling is recorded as folded. Provenance
    /// only; it is not a promise that the file still exists.
    pub resolved_path: Option<String>,
    /// Provenance: the block whose member entities are owned by this xref, when
    /// the importer materialized one.
    pub block: Option<BlockId>,
    /// Whether the source marked this attachment as an overlay (as opposed to a
    /// regular attachment).
    pub overlay: bool,
    /// Nesting depth: `0` for an attachment in the host drawing, `1` for an
    /// attachment inside a referenced drawing, and so on.
    pub depth: u32,
    /// What happened when the reference was (or was not) loaded.
    pub status: ExternalReferenceStatus,
    /// Stable code or host detail for a non-loaded status. Never localized
    /// prose: UI text is produced from the status and this code at the UI layer.
    pub reason: Option<String>,
}
