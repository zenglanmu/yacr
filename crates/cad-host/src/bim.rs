//! BIM query host read (ADR-0042 D11).
//!
//! The backend (`cad_query::bim`) owns the index, the filter semantics, the row
//! order, the paging and the counters. This module is the host's read entry: it
//! binds the query to the **currently open document** and its revision, and it
//! answers an explicit empty page when no document is open.
//!
//! # One read, two backend calls
//!
//! The wire has **one** command (`cad_bim_query`): a page of rows plus the facet
//! summary of the same filtered set. The host computes both from one bind, so a
//! round trip cannot mix two revisions: [`HostCore::bim_query`] runs the paged
//! query and the single-row summary against the same `QueryRequest` document and
//! revision. The summary is not a second wire command.
//!
//! # Why the query service is the session's own
//!
//! `HostController::application.query` is the same [`QueryService`] the rest of
//! the application uses, so a BIM read shares the one cached `(DatabaseId,
//! Revision)` index instead of building a second one. The application replaces
//! the service whenever the document content is replaced, which is exactly the
//! freshness the wire promises: a page is bound to the revision it was answered
//! from.
//!
//! # No document
//!
//! A host with no open document answers a successful empty page (`total: 0`) and
//! an empty summary — never an error and never a panic. On the wire that is
//! `databaseId: 0`, `revision: 0`, `rows: []`: the frontend's empty state, which
//! is a real answer ("nothing is open"), not a fabricated document.
//!
//! # Errors are the backend's
//!
//! Nothing is caught or rewritten here. An over-budget filter
//! (`bim.filter.too_many_terms`) and a page bound to another document
//! (`CadError::StaleResult`) travel out unchanged; the wire layer names the
//! latter with its own stable marker.

use cad_app::cad_query::bim::{BimFilter, BimRow, BimSummary};
use cad_app::cad_query::{QueryPage, QueryRequest};
use cad_domain::{CadResult, DatabaseId, RequestId, Revision};

use crate::HostCore;

/// One BIM read: the page, the same filter's summary and the database identity.
///
/// The page type itself ([`QueryPage`]) carries the request window and the
/// revision, but not the [`DatabaseId`]; the wire needs both numbers. `database`
/// is `None` when no document is open (the page is empty, the revision is 0 and
/// the summary is the default/empty one in that case).
///
/// `summary` counts the **same filtered components** as `page.total`
/// (`summary.total == page.total`): it is not a document-wide tally and not a
/// count of the current page's rows.
#[derive(Debug)]
pub struct BimPage {
    /// The database the answer came from; `None` with no open document.
    pub database: Option<DatabaseId>,
    /// The backend page: rows, total and the request window actually used.
    pub page: QueryPage<BimRow>,
    /// Component counts and facet buckets over the same filtered set.
    pub summary: BimSummary,
}

impl HostCore {
    /// One filtered, paged BIM query over the open document, with its summary.
    ///
    /// `offset` and `limit` are carried verbatim into the page's [`QueryRequest`];
    /// `limit == 0` therefore means "only the total" exactly like every other
    /// paged read (the `QueryPage` contract, not a BIM special case). The summary
    /// is a single logical row, so it is always asked for with its own
    /// `offset: 0, limit: 1` window: `limit == 0` still answers the facet counts.
    pub fn bim_query(&self, filter: &BimFilter, offset: usize, limit: usize) -> CadResult<BimPage> {
        let controller = self.controller.borrow();
        let request = |revision: Revision, offset: usize, limit: usize| QueryRequest {
            document: controller.document_id,
            revision,
            // The wire carries no correlation id (the page is answered
            // synchronously); the field exists for the queued request path.
            request: RequestId(0),
            offset,
            limit,
        };
        let Some(database) = controller.drawing() else {
            // No document is an explicit empty page, not a failure and not a
            // fabricated database identity.
            return Ok(BimPage {
                database: None,
                page: QueryPage {
                    request: request(Revision(0), offset, limit),
                    rows: Vec::new(),
                    total: 0,
                },
                summary: BimSummary::default(),
            });
        };
        let revision = database.revision();
        let query = &controller.application.query;
        let page = query.bim_query(database.as_ref(), filter, request(revision, offset, limit))?;
        let summary_page = query.bim_summary(database.as_ref(), filter, request(revision, 0, 1))?;
        Ok(BimPage {
            database: Some(database.id()),
            page,
            // The summary travels as one logical row; a page with no row can only
            // mean the caller asked for a zero window, which this call never does.
            summary: summary_page.rows.into_iter().next().unwrap_or_default(),
        })
    }
}
