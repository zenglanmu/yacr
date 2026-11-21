//! The desktop half of the BIM query wire (ADR-0042 D11).
//!
//! A BIM read takes a caller-supplied filter, so — unlike `cad_layers`/`cad_fonts`
//! — it cannot be answered from the owner-tick mirror. The desktop serves it
//! through the host's [`BimQueryHost`] seam, which marshals onto the `!Send`
//! session thread; a host that installs no seam keeps the shared
//! [`BIM_UNSUPPORTED`] refusal.
//!
//! There is one wire command (`cad_bim_query`): the answer carries the page and
//! the facet summary, so the desktop makes one session hop per request. The
//! former summary command and its second hop are gone.
//!
//! This module exists (rather than living in `session::wire`) because `wire.rs`
//! is at the 1000-line file gate: the trait impl, the builder and the wire entry
//! point are cohesive here, while `wire.rs` keeps only the struct field.

use crate::bim::{
    BimFilterWire, BimQueryHost, BimQueryReads, BimQueryRequest, BimQueryResponse, BimQuerySeam,
    BIM_UNSUPPORTED,
};

use super::wire::QueueReads;

impl<'a> QueueReads<'a> {
    /// Attach the host's BIM query seam (a no-op when it installed none).
    pub fn with_bim_query_host(
        mut self,
        host: Option<&'a std::sync::Arc<dyn BimQueryHost>>,
    ) -> Self {
        self.bim = BimQuerySeam(host);
        self
    }
}

impl BimQueryReads for QueueReads<'_> {
    fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        match self.bim.host() {
            Some(host) => host.bim_query(request),
            None => Err(BIM_UNSUPPORTED.to_string()),
        }
    }
}

/// `cad_bim_query`: one filtered, paged BIM query with its summary (ADR-0042 D11).
///
/// The arguments are the decoded wire fields (`filter`, `offset`, `limit`); the
/// filter's `value.kind` is validated when the request reaches the host, whose
/// answer carries the explicit `bim.filter.unsupported_value` refusal when the
/// kind is unknown. `limit: 0` answers only the total — and still the summary.
pub fn bim_query(
    reads: &dyn BimQueryReads,
    filter: BimFilterWire,
    offset: usize,
    limit: usize,
) -> Result<BimQueryResponse, String> {
    reads.bim_query(&BimQueryRequest::new(filter, offset, limit))
}
