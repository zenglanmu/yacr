//! Carrier-supplied native document archive access for the session.
//!
//! Split out of `session/mod.rs` (already at the physical-line gate) but still
//! the one `impl CadSession`. The carrier owns the archive implementation
//! (`cad-store-zip` may only be linked by `apps/app-*`), so the session only
//! forwards it to the host core — exactly the shape
//! [`CadSession::set_solid_modeler`] uses for the kernel seam.
//!
//! A session that never receives one keeps the host's explicit
//! `unsupported-runtime: ymodel` refusal for both open and save.

use std::sync::Arc;

use cad_platform::DocumentArchive;

use super::CadSession;

impl CadSession {
    /// Install (or clear) the carrier's native `.ymodel` archive (ADR-0037 D16).
    ///
    /// Called once by a native carrier after the session is built. Without it,
    /// opening or saving a `.ymodel` refuses by name rather than falling back to
    /// a connector format.
    pub fn set_document_archive(&self, archive: Option<Arc<dyn DocumentArchive>>) {
        self.core.set_document_archive(archive);
    }

    /// The installed native document archive, if the carrier supplied one.
    pub fn document_archive(&self) -> Option<Arc<dyn DocumentArchive>> {
        self.core.document_archive()
    }
}
