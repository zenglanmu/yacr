//! The desktop half of the BIM query/summary wire (ADR-0042 D11).
//!
//! A BIM read takes a caller-supplied filter, so — unlike `cad_layers`/`cad_fonts`
//! — it cannot be answered from the owner-tick mirror. The desktop serves it
//! through the host's [`BimHost`] seam, which marshals onto the `!Send` session
//! thread; a host that installs no seam keeps the shared
//! [`BIM_UNSUPPORTED`] refusal.
//!
//! This module exists (rather than living in `session::wire`) because `wire.rs`
//! is at the 1000-line file gate: the trait impl, the builder and the two wire
//! entry points are cohesive here, while `wire.rs` keeps only the struct field.

use crate::bim::{
    BimFilterWire, BimHost, BimQueryRequest, BimQueryResponse, BimReads, BimSeam,
    BimSummaryResponse, BIM_UNSUPPORTED,
};

use super::wire::QueueReads;

impl<'a> QueueReads<'a> {
    /// Attach the host's BIM query seam (a no-op when it installed none).
    pub fn with_bim_host(mut self, host: Option<&'a std::sync::Arc<dyn BimHost>>) -> Self {
        self.bim = BimSeam(host);
        self
    }
}

impl BimReads for QueueReads<'_> {
    fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        match self.bim.host() {
            Some(host) => host.bim_query(request),
            None => Err(BIM_UNSUPPORTED.to_string()),
        }
    }

    fn bim_summary(&self, request: &BimQueryRequest) -> Result<BimSummaryResponse, String> {
        match self.bim.host() {
            Some(host) => host.bim_summary(request),
            None => Err(BIM_UNSUPPORTED.to_string()),
        }
    }
}

/// `cad_bim_query`: one filtered, paged BIM query (ADR-0042 D11).
///
/// The arguments are the decoded wire fields (`filter`, `offset`, `limit`); the
/// filter's `value.kind` is validated when the request reaches the host, whose
/// answer carries the explicit `bim.filter.unsupported_value` refusal when the
/// kind is unknown.
pub fn bim_query(
    reads: &dyn BimReads,
    filter: BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<BimQueryResponse, String> {
    reads.bim_query(&BimQueryRequest::new(filter, offset, limit))
}

/// `cad_bim_summary`: counts over one filtered BIM set (ADR-0042 D11).
///
/// Same request object as [`bim_query`]; a summary is one logical row, so the
/// host answers the whole count and ignores the page window.
pub fn bim_summary(
    reads: &dyn BimReads,
    filter: BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<BimSummaryResponse, String> {
    reads.bim_summary(&BimQueryRequest::new(filter, offset, limit))
}
