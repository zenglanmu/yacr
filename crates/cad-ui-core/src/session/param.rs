//! The session entry of the document-level parameter list (ADR-0043 P1).
//!
//! [`CadSession`] owns the `HostCore`; this module is the thin bridge between the
//! request-carrying host-read seam and [`cad_host::HostCore::param_list`]. It
//! adds no filtering or ordering: the host reads the authoritative table and the
//! active configuration's status.
//!
//! There is no dedicated command and no React consumer in P1; the list is a
//! wire-only read over [`crate::host_read::HostReadRequest::ParamList`].

use crate::param::ParamListResponse;

use super::diagnostics::cad_error_text;
use super::CadSession;

impl CadSession {
    /// The document-level parameter list under the active configuration.
    ///
    /// No document is an empty answer (`active: None`, `rows: []`, `total: 0`),
    /// never an error: the caller asked a real question.
    pub fn param_list(&self) -> Result<ParamListResponse, String> {
        self.core
            .param_list()
            .map(ParamListResponse::from_host)
            .map_err(|error| cad_error_text(&error))
    }
}
