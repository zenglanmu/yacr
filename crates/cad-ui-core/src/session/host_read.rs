//! The shared session half of request-carrying host reads (ADR-0042 D11).
//!
//! [`CadSession::host_read`] is the one place a host — native or wasm — turns a
//! [`HostReadRequest`] into a [`HostReadAnswer`] on the thread that owns the
//! `!Send` document. Today the only kind is a BIM query, whose backend rules live
//! in `cad_query::bim`; this module only routes, never filters or counts.
//!
//! Desktop carriers do not call the session directly: the registry row goes
//! through [`HostReads`], which [`super::wire::QueueReads`] forwards to the host's
//! [`HostReadSeam`] (the closure that marshals onto this thread). A carrier with
//! no seam keeps the kind's own named refusal from
//! [`crate::host_read::no_seam_refusal`].
//!
//! # Adding a kind
//!
//! Add the match arm here (the session method) and the wire row that builds the
//! request; **no per-carrier file changes**. `cad_bim_query` is such a row: its
//! name, arguments and reply shape are unchanged, and
//! [`bim_query`] is the adapter from its three decoded fields.

use std::sync::Arc;

use crate::bim::{BimFilterWire, BimQueryRequest, BimQueryResponse};
use crate::host_read::{HostReadAnswer, HostReadHost, HostReadRequest, HostReadSeam, HostReads};

use super::wire::QueueReads;
use super::CadSession;

impl CadSession {
    /// Serve one request-carrying read over the open document.
    ///
    /// No document is a successful empty answer for a BIM query (`total: 0` with
    /// an empty summary), never an error: the panel then shows its empty state.
    pub fn host_read(&self, request: &HostReadRequest) -> Result<HostReadAnswer, String> {
        match request {
            HostReadRequest::BimQuery {
                filter,
                offset,
                limit,
            } => Ok(HostReadAnswer::BimQuery(self.bim_query(
                &BimQueryRequest::new(filter.clone(), *offset, *limit),
            )?)),
        }
    }
}

impl<'a> QueueReads<'a> {
    /// Attach the host's host-read seam (a no-op when it installed none).
    pub fn with_host_read_host(mut self, host: Option<&'a Arc<dyn HostReadHost>>) -> Self {
        self.host_read = HostReadSeam(host);
        self
    }
}

impl HostReads for QueueReads<'_> {
    fn host_read(&self, request: &HostReadRequest) -> Result<HostReadAnswer, String> {
        self.host_read.read(request)
    }
}

/// `cad_bim_query`: one filtered, paged BIM query with its summary.
///
/// The arguments are the decoded wire fields (`filter`, `offset`, `limit`); the
/// filter's `value.kind` is validated when the request reaches the host, whose
/// answer carries the explicit `bim.filter.unsupported_value` refusal when the
/// kind is unknown. `limit: 0` answers only the total — and still the summary.
///
/// The match has one arm because the envelope has one kind: adding a kind makes
/// this an exhaustive-match error instead of letting it be answered as BIM.
pub fn bim_query(
    reads: &dyn HostReads,
    filter: BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<BimQueryResponse, String> {
    match reads.host_read(&HostReadRequest::BimQuery {
        filter,
        offset,
        limit,
    })? {
        HostReadAnswer::BimQuery(response) => Ok(response),
    }
}
