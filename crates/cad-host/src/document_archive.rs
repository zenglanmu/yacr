//! The carrier-supplied native document archive (ADR-0037 D1/D16).
//!
//! `cad-store-zip` may only be linked by an `apps/app-*` carrier (architecture
//! gate), so the shared host receives the implementation as a
//! [`cad_platform::DocumentArchive`] trait object. The host itself only decides
//! *when* to decode/encode: a `.ymodel` open/save, or the explicit
//! `unsupported-runtime: ymodel` refusal when no archive was installed.

use super::*;
use cad_platform::DocumentArchive;

impl HostCore {
    /// Install (or clear) the carrier's native document archive.
    ///
    /// The archive is a carrier capability, like [`cad_platform::PlatformFiles`],
    /// so it arrives as a trait object; the shared layers may not link the store.
    /// It is **not** a field of [`HostOptions`]/[`HostPlatform`] because every
    /// carrier constructs those two with an exhaustive struct literal: adding a
    /// field there would break the carriers' (and `cad-ui-core`'s) literals, which
    /// this change does not own. A carrier that never installs one keeps the
    /// explicit `unsupported-runtime: ymodel` refusal on open and on save.
    pub fn set_document_archive(&self, archive: Option<Arc<dyn DocumentArchive>>) {
        *self.archive.borrow_mut() = archive;
    }

    /// The installed native document archive, if the carrier supplied one.
    pub fn document_archive(&self) -> Option<Arc<dyn DocumentArchive>> {
        self.archive.borrow().clone()
    }
}

/// Lowercase hex `sha256` of `bytes`.
///
/// Used only for the `.ymodel` conflict rule (ADR-0037 D12): the hash of the
/// bytes a document was opened from/saved to. The archive's own entry hashes are
/// the store's business; this one is the carrier file's identity for the host.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}
