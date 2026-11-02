//! Host-side file access for external references (XREFs).
//!
//! Spec v2.0 §7.3, §16.2: `cad-resources` and `cad-import-acadrust` are
//! platform-neutral and must stay wasm-clean. They may decide *whether* a
//! reference path is allowed ([`crate::check_xref_path`]) and *what* to do with
//! the bytes, but they never open a file. The concrete carrier (native host,
//! browser bridge, Android bridge) implements this trait.
//!
//! The `Ok(None)` shape is deliberately distinct from an error: it means the
//! carrier has no filesystem at all (a browser without a host bridge), which is
//! reported as `browser-unavailable` rather than "missing file". The error
//! variants keep the same distinction for a carrier that *does* have a
//! filesystem: a missing file, a refused canonical path and an unreadable file
//! must never be collapsed into one another.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// Why one external-reference read did not return bytes.
///
/// The importer maps these variants to distinct terminal statuses so a
/// symlink-escape refusal is never reported as a plain missing file:
///
/// * [`Self::Missing`] → `Missing`
/// * [`Self::Refused`] → `RefusedPath`
/// * [`Self::Unreadable`] → `Unsupported` (with the real reason preserved)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrefSourceError {
    /// The carrier resolved the request but there is no such file.
    Missing(String),
    /// The carrier found a real target but its policy forbids reading it
    /// (for example a canonical path/symlink outside the allowed roots).
    Refused(String),
    /// The target exists but could not be decoded/read (permissions, metadata,
    /// I/O error). This is not a policy refusal and not "not found".
    Unreadable(String),
}

impl XrefSourceError {
    /// The stable/detail text recorded as the reference's `reason`.
    pub fn reason(&self) -> &str {
        match self {
            Self::Missing(reason) | Self::Refused(reason) | Self::Unreadable(reason) => reason,
        }
    }
}

impl fmt::Display for XrefSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for XrefSourceError {}

/// Read one external-reference file.
///
/// Implementations must be callable from the importer's thread and must not
/// silently return empty bytes for a missing file: a missing/unreadable file is
/// an `Err`, a carrier without filesystem is `Ok(None)`.
pub trait XrefFileSource: Send + Sync {
    /// Read the bytes at an **already lexically approved** path.
    ///
    /// `Ok(Some(bytes))` is the file content. `Ok(None)` means this runtime has
    /// no filesystem/network access for external references (for example a
    /// browser/wasm carrier without a host bridge); the importer records
    /// `BrowserUnavailable` and never treats it as an empty drawing. An `Err`
    /// keeps the specific failure ([`XrefSourceError`]) so the importer can
    /// report `Missing`, `RefusedPath` or `Unsupported` honestly.
    fn read(&self, path: &Path) -> Result<Option<Arc<[u8]>>, XrefSourceError>;
}
