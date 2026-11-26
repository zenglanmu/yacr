//! Carrier-supplied persistence access for the session.
//!
//! Split out of `session/mod.rs` (already at the physical-line gate) but still
//! the one `impl CadSession`. It carries the two seams a carrier installs after
//! the session is built, both trait objects because the shared layers may not
//! link the implementations:
//!
//! * [`CadSession::set_document_archive`] — the native `.ymodel` container
//!   (`cad-store-zip`, linkable only by an `apps/app-*` carrier);
//! * [`CadSession::set_persistence`] — the recovery-snapshot store (ADR-0037
//!   D12), a carrier-private directory that a browser/SAF carrier does not have.
//!
//! Both forward to the host core, exactly the shape
//! [`CadSession::set_solid_modeler`] uses for the kernel seam. A session that
//! never receives an archive keeps the host's explicit `unsupported-runtime:
//! ymodel` refusal for open and save; a session that never receives a recovery
//! store simply writes no snapshot (it does not pretend to have one).

use std::sync::Arc;

use cad_platform::{DocumentArchive, Persistence};

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

    /// Install (or clear) the carrier's recovery-snapshot store (ADR-0037 D12).
    ///
    /// Called once by a carrier that has a durable private directory. Without it
    /// the host writes no snapshot at all: "this carrier cannot keep one" is not
    /// something this layer can distinguish from "recovery failed", so it stays
    /// silent rather than guessing.
    pub fn set_persistence(&self, persistence: Option<Arc<dyn Persistence>>) {
        self.core.set_persistence(persistence);
    }

    /// The installed recovery store, if the carrier supplied one.
    pub fn persistence_store(&self) -> Option<Arc<dyn Persistence>> {
        self.core.persistence_store()
    }

    /// Install (or clear) the carrier's mesh modeler (ADR-0061 P-MESH-1a/1b).
    ///
    /// The same carrier-owned, trait-only composition as
    /// [`Self::set_solid_modeler`]: `app-linux` injects the real
    /// `ManifoldMeshModeler`; every other carrier passes nothing and mesh
    /// booleans/repairs refuse with `unsupported-runtime: mesh-modeler`.
    pub fn set_mesh_modeler(&self, modeler: Option<Arc<dyn cad_kernel_adapter::MeshModeler>>) {
        self.core
            .controller
            .borrow_mut()
            .application
            .set_mesh_modeler(modeler);
    }
}
