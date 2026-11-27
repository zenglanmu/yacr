//! The shared session half of request-carrying host reads (ADR-0042 D11).
//!
//! [`CadSession::host_read`] is the one place a host — native or wasm — turns a
//! [`HostReadRequest`] into a [`HostReadAnswer`] on the thread that owns the
//! `!Send` document. Today the kinds are a BIM query (backend rules in
//! `cad_query::bim`), the document-level parameter list (ADR-0043 P1), the
//! mechanical FeatureManager tree and the action-recorder state; this module only
//! routes, never filters or counts.
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

use crate::action_recorder::ActionRecorderSnapshot;
use crate::bim::{BimFilterWire, BimQueryRequest, BimQueryResponse};
use crate::display::DisplayResponse;
use crate::host_read::{
    HostReadAnswer, HostReadHost, HostReadRequest, HostReadSeam, HostReads, HOST_READ_UNKNOWN_KIND,
};
use crate::mechanical::MechanicalTreeResponse;
use crate::outliner::OutlinerResponse;
use crate::param::ParamListResponse;
use crate::scene::ScenesResponse;

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
            HostReadRequest::ParamList => Ok(HostReadAnswer::ParamList(self.param_list()?)),
            HostReadRequest::MechanicalTree => {
                Ok(HostReadAnswer::MechanicalTree(self.mechanical_tree()?))
            }
            HostReadRequest::ActionRecorder => {
                Ok(HostReadAnswer::ActionRecorder(self.action_recorder()?))
            }
            HostReadRequest::Scenes => Ok(HostReadAnswer::Scenes(self.scenes())),
            HostReadRequest::Display => Ok(HostReadAnswer::Display(self.display())),
            HostReadRequest::Outliner => Ok(HostReadAnswer::Outliner(self.outliner())),
        }
    }
}

impl CadSession {
    /// The open document's persisted scenes, in save order.
    ///
    /// Host content only: no document is an empty list (the panel's empty state),
    /// never an error; the host read seam's own refusal is what a carrier with no
    /// lane reports.
    pub fn scenes(&self) -> ScenesResponse {
        ScenesResponse::from_host(self.core.scenes())
    }

    /// The open document's Shadows/Fog record.
    ///
    /// Host content only: no document answers the default record (the panels'
    /// "nothing customised" state), never an error; a carrier with no lane still
    /// reports its own refusal at the wire layer.
    pub fn display(&self) -> DisplayResponse {
        DisplayResponse::from_host(self.core.display())
    }

    /// The open document's Outliner tree.
    ///
    /// Host content only: no document answers an empty tree (the panel's empty
    /// state), never an error; a carrier with no lane still reports its own
    /// refusal at the wire layer.
    pub fn outliner(&self) -> OutlinerResponse {
        OutlinerResponse::from_host(self.core.outliner_tree())
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
/// The match stays explicit per kind: adding a kind makes this an
/// exhaustive-match error instead of letting it be answered as BIM. A host that
/// returns the wrong kind is a transport fault, refused by the same stable
/// marker as an unknown request kind.
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
        HostReadAnswer::ParamList(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Display(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to a BIM query"
        )),
    }
}

/// `cad_param_list`: the document-level parameter list under the active
/// configuration (ADR-0043 P1).
///
/// One request-carrying read over the shared seam; the session's handler is
/// [`CadSession::param_list`]. A host that answers a different kind is a
/// transport fault and is refused by the same marker as an unknown request kind.
pub fn param_list(reads: &dyn HostReads) -> Result<ParamListResponse, String> {
    match reads.host_read(&HostReadRequest::ParamList)? {
        HostReadAnswer::ParamList(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Display(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to a parameter list"
        )),
    }
}

/// `cad_action_recorder`: the session's recorder state and visible macros
/// (ADR-0056).
///
/// One request-carrying read over the shared seam; the session's handler is
/// [`CadSession::action_recorder`]. A carrier without a file bridge answers its
/// own `unsupported-runtime: action-recorder` refusal, never an empty list.
pub fn action_recorder(reads: &dyn HostReads) -> Result<ActionRecorderSnapshot, String> {
    match reads.host_read(&HostReadRequest::ActionRecorder)? {
        HostReadAnswer::ActionRecorder(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::ParamList(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Display(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to an action-recorder read"
        )),
    }
}

/// `cad_mechanical_tree`: the mechanical profile's bounded FeatureManager tree.
///
/// One request-carrying read over the shared seam; the session's handler is
/// [`CadSession::mechanical_tree`]. A carrier without a tree lane answers its own
/// `unsupported-runtime: mechanicalTree` refusal, never an empty tree. A host
/// that answers a different kind is a transport fault and is refused by the same
/// marker as an unknown request kind.
pub fn scenes(reads: &dyn HostReads) -> Result<ScenesResponse, String> {
    match reads.host_read(&HostReadRequest::Scenes)? {
        HostReadAnswer::Scenes(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::ParamList(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Display(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to a scenes read"
        )),
    }
}

pub fn mechanical_tree(reads: &dyn HostReads) -> Result<MechanicalTreeResponse, String> {
    match reads.host_read(&HostReadRequest::MechanicalTree)? {
        HostReadAnswer::MechanicalTree(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::ParamList(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Display(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to a mechanical tree"
        )),
    }
}

/// `cad_display`: the open document's Shadows/Fog record.
///
/// One request-carrying read over the shared seam; the session's handler is
/// [`CadSession::display`]. A carrier without a lane answers its own
/// `unsupported-runtime: display` refusal, never a default record.
pub fn display(reads: &dyn HostReads) -> Result<DisplayResponse, String> {
    match reads.host_read(&HostReadRequest::Display)? {
        HostReadAnswer::Display(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::ParamList(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Outliner(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to a display read"
        )),
    }
}

/// `cad_outliner`: the open document's hierarchy.
///
/// One request-carrying read over the shared seam; the session's handler is
/// [`CadSession::outliner`]. A carrier without a lane answers its own
/// `unsupported-runtime: outliner` refusal, never an empty tree.
pub fn outliner(reads: &dyn HostReads) -> Result<OutlinerResponse, String> {
    match reads.host_read(&HostReadRequest::Outliner)? {
        HostReadAnswer::Outliner(response) => Ok(response),
        HostReadAnswer::BimQuery(_)
        | HostReadAnswer::ParamList(_)
        | HostReadAnswer::MechanicalTree(_)
        | HostReadAnswer::ActionRecorder(_)
        | HostReadAnswer::Scenes(_)
        | HostReadAnswer::Display(_) => Err(format!(
            "{HOST_READ_UNKNOWN_KIND}: the host answered a different kind to an outliner read"
        )),
    }
}
