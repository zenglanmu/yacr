//! BIM query/summary host reads (ADR-0042 D11).
//!
//! The backend (`cad_query::bim`) owns the index, the filter semantics, the row
//! order, the paging and the counters. This module is the host's read entry: it
//! binds the query to the **currently open document** and its revision, and it
//! answers an explicit empty page when no document is open.
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
//! A host with no open document answers a successful empty page (`total: 0`) —
//! never an error and never a panic. On the wire that is `databaseId: 0`,
//! `revision: 0`, `rows: []`: the frontend's empty state, which is a real answer
//! ("nothing is open"), not a fabricated document.
//!
//! # Errors are the backend's
//!
//! Nothing is caught or rewritten here. An over-budget filter
//! (`bim.filter.too_many_terms`) and a page bound to another document
//! (`CadError::StaleResult`) travel out unchanged; the wire layer names the
//! latter with its own stable marker.

use cad_app::cad_query::bim::{BimFilter, BimRow, BimSummary};
use cad_app::cad_query::{QueryPage, QueryRequest, QueryService};
use cad_db::DrawingDatabase;
use cad_domain::{CadResult, DatabaseId, RequestId, Revision};

use crate::HostCore;

/// One BIM read: the page plus the database identity it was answered from.
///
/// The page type itself ([`QueryPage`]) carries the request window and the
/// revision, but not the [`DatabaseId`]; the wire needs both numbers. `database`
/// is `None` when no document is open (the page is empty and the revision is 0 in
/// that case).
#[derive(Debug)]
pub struct BimPage<T> {
    /// The database the answer came from; `None` with no open document.
    pub database: Option<DatabaseId>,
    /// The backend page: rows, total and the request window actually used.
    pub page: QueryPage<T>,
}

impl HostCore {
    /// One filtered, paged BIM query over the open document.
    ///
    /// `offset` and `limit` are carried verbatim into the [`QueryRequest`];
    /// `limit == 0` therefore means "only the total" exactly like every other
    /// paged read (the `QueryPage` contract, not a BIM special case).
    pub fn bim_query(
        &self,
        filter: &BimFilter,
        offset: usize,
        limit: usize,
    ) -> CadResult<BimPage<BimRow>> {
        self.bim_page(offset, limit, |query, database, request| {
            query.bim_query(database, filter, request)
        })
    }

    /// Counts over the components `filter` selects, as one logical row.
    ///
    /// The backend summary is a single row, so `offset`/`limit` still page it
    /// like any other [`QueryPage`] (`limit == 0` answers no row at all).
    pub fn bim_summary(
        &self,
        filter: &BimFilter,
        offset: usize,
        limit: usize,
    ) -> CadResult<BimPage<BimSummary>> {
        self.bim_page(offset, limit, |query, database, request| {
            query.bim_summary(database, filter, request)
        })
    }

    /// Bind one query to the open document (or answer the explicit empty page).
    fn bim_page<T>(
        &self,
        offset: usize,
        limit: usize,
        run: impl FnOnce(&QueryService, &DrawingDatabase, QueryRequest) -> CadResult<QueryPage<T>>,
    ) -> CadResult<BimPage<T>> {
        let controller = self.controller.borrow();
        let request = |revision: Revision| QueryRequest {
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
                    request: request(Revision(0)),
                    rows: Vec::new(),
                    total: 0,
                },
            });
        };
        let page = run(
            &controller.application.query,
            database.as_ref(),
            request(database.revision()),
        )?;
        Ok(BimPage {
            database: Some(database.id()),
            page,
        })
    }
}
