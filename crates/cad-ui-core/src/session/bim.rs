//! The session entry of the BIM query wire (ADR-0042 D11).
//!
//! [`CadSession`] is the one place a host (native or wasm) owns absent the
//! transport differences: it holds the `HostCore` whose `QueryService` builds the
//! BIM index. This module is the thin bridge between the wire request
//! ([`crate::bim::BimQueryRequest`]) and the host read
//! ([`cad_host::HostCore::bim_query`]), so both transports call the same
//! function.
//!
//! The one command answers **both** halves — the page and the facet summary — so
//! there is no second session entry: a caller that only wants counts sends the
//! same request with `limit: 0` and reads `summary`.
//!
//! Nothing here filters, sorts or counts: that is `cad_query`'s job. This module
//! only parses the wire filter, extracts the identity the page was answered from
//! and maps the backend page into the camelCase wire shape.

use cad_domain::CadError;

use crate::bim::{query_response, BimQueryRequest, BimQueryResponse, BIM_STALE};

use super::diagnostics::cad_error_text;
use super::CadSession;

/// The wire text of a backend BIM read failure.
///
/// Every `CadError` except the stale-document case is the backend's own reason and
/// travels verbatim (the filter budget
/// `bim.filter.too_many_terms` is one of them). A page bound to another document
/// gets the stable [`BIM_STALE`] marker, because "re-run against the open
/// document" is the caller's only correct reaction.
fn bim_error(error: &CadError) -> String {
    match error {
        CadError::StaleResult => BIM_STALE.to_string(),
        other => cad_error_text(other),
    }
}

impl CadSession {
    /// One filtered, paged BIM query over the open document, with its summary.
    ///
    /// No document → a successful empty page (`total: 0`) with an empty summary,
    /// never an error: the panel then shows its empty state, which is a real
    /// answer.
    ///
    /// The request's `offset`/`limit` page the **rows**; the summary block counts
    /// every matched component regardless of the window, so `limit: 0` still
    /// answers the facet counts.
    pub fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        let filter = request.filter.to_filter()?;
        let answer = self
            .core
            .bim_query(&filter, request.offset, request.limit)
            .map_err(|error| bim_error(&error))?;
        Ok(query_response(
            answer.page,
            &answer.summary,
            answer.database,
        ))
    }
}
