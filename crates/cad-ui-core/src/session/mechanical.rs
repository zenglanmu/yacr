//! The session entry of the mechanical FeatureManager tree read.
//!
//! [`CadSession`] owns the `HostCore`; this module is the thin bridge between
//! [`crate::host_read::HostReadRequest::MechanicalTree`] and
//! [`cad_host::HostCore::mechanical_tree`]. It adds no ordering or filtering: the
//! host projects the document's feature table and body rows and bounds the page.
//!
//! There is no dedicated drawing command and no write path — the tree is a
//! read-only projection of the document the session already owns. The projection
//! is infallible by construction (it reads rows, never evaluates a feature), so
//! unlike the parameter list it has no `param.*` error to translate.

use crate::mechanical::MechanicalTreeResponse;

use super::CadSession;

impl CadSession {
    /// The open document's bounded mechanical tree.
    ///
    /// No document is an explicit empty answer (`revision: 0`, `rows: []`), never
    /// an error: the caller asked a real question and "nothing is open" is its
    /// real answer.
    pub fn mechanical_tree(&self) -> Result<MechanicalTreeResponse, String> {
        Ok(MechanicalTreeResponse::from_host(
            self.core.mechanical_tree(),
        ))
    }
}
