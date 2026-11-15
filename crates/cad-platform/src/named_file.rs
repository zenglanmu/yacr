//! A host-side read of one **user-named** file, outside the drawing open/save
//! funnels.
//!
//! Some CAD commands name a host file directly on the command line rather than
//! opening it as a drawing: `HATCHEDIT PATTERNFILE <path.pat>` loads a user
//! `.pat` pattern library. The command/session layer must stay filesystem-free
//! (it is also compiled to wasm), so the read lives here — the same place the
//! native carriers for XREFs, fonts and plugin entries live — and the host
//! strategy ([`cad_host::HostResources::read_named_file`]) decides whether its
//! runtime can serve it at all.
//!
//! The policy is deliberately the same as an explicit `--font NAME=PATH` entry:
//! the user typed this exact path, so it is read as given (no root containment,
//! which would refuse the common `/usr/share/...` symlink layouts). Two bounds
//! remain: the target must be a **file** (not a directory or device) and it must
//! stay inside [`NAMED_FILE_MAX_BYTES`], so a mistyped path cannot pull a huge
//! binary into memory before it is rejected as "not a pattern library".

use std::path::Path;
use std::sync::Arc;

/// Largest user-named file this build will read into memory (1 MiB).
///
/// A `.pat` library is text: the largest handled here is orders of magnitude
/// smaller (the parser caps a library at 4096 patterns × 64 rows). The bound
/// exists so a mistyped path (`/dev/zero`, a multi-GB archive) is refused by
/// name instead of being read.
pub const NAMED_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// Why a user-named host file was not delivered.
///
/// The variants are kept distinct so a refusal can never claim "missing" for a
/// permission error, or a browser without a filesystem for a missing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedFileError {
    /// This runtime has no filesystem for user-named files (a browser/wasm
    /// carrier without a host bridge).
    Unavailable,
    /// The path does not resolve to an existing regular file.
    Missing(String),
    /// The target exists but could not be read (permissions, I/O, or the
    /// [`NAMED_FILE_MAX_BYTES`] bound).
    Unreadable(String),
}

impl NamedFileError {
    /// The stable detail text (no localized prose; the caller owns the code).
    pub fn reason(&self) -> &str {
        match self {
            Self::Unavailable => "this host has no filesystem for a user-named file",
            Self::Missing(reason) | Self::Unreadable(reason) => reason,
        }
    }
}

impl std::fmt::Display for NamedFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for NamedFileError {}

/// Read one user-named file on a native host.
///
/// `Err(NamedFileError::Unavailable)` on a runtime without a filesystem — never
/// an empty successful read, so the caller can name the missing capability
/// instead of reporting a missing file.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_named_file(path: &Path) -> Result<Arc<[u8]>, NamedFileError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            NamedFileError::Missing(format!("no such file: {}", path.display()))
        } else {
            NamedFileError::Unreadable(format!("metadata failed for {}: {error}", path.display()))
        }
    })?;
    if !metadata.is_file() {
        return Err(NamedFileError::Unreadable(format!(
            "not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > NAMED_FILE_MAX_BYTES {
        return Err(NamedFileError::Unreadable(format!(
            "{} is {} bytes; the limit for a user-named file is {NAMED_FILE_MAX_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        NamedFileError::Unreadable(format!("read failed for {}: {error}", path.display()))
    })?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

/// A wasm/browser carrier has no filesystem for a user-named path.
#[cfg(target_arch = "wasm32")]
pub fn read_named_file(_path: &Path) -> Result<Arc<[u8]>, NamedFileError> {
    Err(NamedFileError::Unavailable)
}

#[cfg(test)]
mod tests;
