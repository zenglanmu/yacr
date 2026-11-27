//! **Request-carrying host reads**: one tagged envelope, one seam (ADR-0042 D11).
//!
//! Two families of reads reach the shared command layer:
//!
//! * **owner-tick projections** ([`crate::session::ControlReads`]) — no
//!   arguments, mirrored ahead of time (`cad_settings`, `cad_layers`,
//!   `cad_fonts`, the plugin-API snapshot). A `dom-canvas` carrier answers from
//!   its live session, a desktop carrier from the last tick's mirror.
//! * **request-carrying reads** ([`HostReads`]) — the caller supplies the
//!   question (a filter, a page window, …), so no mirror can precompute it and
//!   the host has to run it on the thread that owns the `!Send` session.
//!
//! Before this module the second family had its own trait trio per request kind
//! (a per-kind host trait, reads trait and seam newtype), which forced **one seam
//! file per carrier per kind**. Now there is exactly one seam shape:
//!
//! ```text
//! wire row ─► HostReads::host_read(&HostReadRequest)
//!                       │  desktop: HostReadSeam ─► HostReadHost (marshals to the session thread)
//!                       │  wasm:    CadSession::host_read (same thread, direct)
//!                       ▼
//!              HostReadAnswer (tagged by the same `kind`)
//! ```
//!
//! # Adding a request kind
//!
//! Today [`HostReadRequest`] has four variants: `bimQuery`, `paramList`,
//! `mechanicalTree` and `actionRecorder`. Each
//! `kind` is spelled once, in its module's constant (`BIM_QUERY_KIND`,
//! `PARAM_LIST_KIND`); the serialized tag and the decoder read that constant, and
//! a test pins them together.
//!
//! **A new kind must not add a per-carrier file.** Add a variant here plus the
//! matching [`HostReadAnswer`] variant, give it its own honest
//! [`no_seam_refusal`] word, and route it in
//! [`crate::session::host_read`]. The four carrier seam files keep one
//! `HostReadFn` closure each and never learn the new kind: the marshalling and
//! the refusal live here, the session method lives in
//! [`crate::session::CadSession::host_read`].
//!
//! # Refusals
//!
//! * No seam installed → the kind's own marker ([`BIM_UNSUPPORTED`] for a BIM
//!   query): "this carrier has no lane", never an empty answer that would read
//!   as "this document has no data".
//! * A `kind` this build does not know → [`HOST_READ_UNKNOWN_KIND`]. The
//!   decoder names it instead of coercing the request into the one kind it does
//!   know.

use std::sync::Arc;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::action_recorder::{ActionRecorderSnapshot, ACTION_RECORDER_UNSUPPORTED};
use crate::bim::{BimFilterWire, BimQueryResponse, BIM_DEFAULT_LIMIT, BIM_UNSUPPORTED};
use crate::display::{DisplayResponse, DISPLAY_KIND, DISPLAY_UNSUPPORTED};
use crate::mechanical::{
    MechanicalTreeResponse, MECHANICAL_TREE_KIND, MECHANICAL_TREE_UNSUPPORTED,
};
use crate::param::{ParamListResponse, PARAM_LIST_KIND, PARAM_LIST_UNSUPPORTED};
use crate::scene::{ScenesResponse, SCENES_KIND, SCENES_UNSUPPORTED};

/// The wire spelling of the BIM request kind.
///
/// `camelCase`, like every other wire key. This constant is the **only** place
/// the spelling lives: it is the serialized `kind` tag and the decoder's
/// accepted value, and `a_known_kind_round_trips` pins both.
pub const BIM_QUERY_KIND: &str = "bimQuery";

/// The wire spelling of the action-recorder state read.
pub const ACTION_RECORDER_KIND: &str = "actionRecorder";

/// Stable refusal for a `kind` this build does not know.
///
/// Distinct from "the kind exists but this carrier cannot ask it"
/// ([`BIM_UNSUPPORTED`]): the request itself is malformed/mismatched, so the
/// caller must not retry the same envelope on another carrier.
pub const HOST_READ_UNKNOWN_KIND: &str = "unsupported-runtime: hostRead.kind";

/// One request-carrying read, self-describing by `kind`.
///
/// The variant fields are the wire fields of the underlying command
/// (`cad_bim_query` keeps `filter`/`offset`/`limit` and its reply shape); the
/// envelope adds only the `kind` tag, and the wire command itself is unchanged.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum HostReadRequest {
    /// `kind: "bimQuery"` — one filtered, paged BIM query.
    BimQuery {
        /// What to select (the same filter `cad_bim_query` decodes).
        filter: BimFilterWire,
        /// Rows to skip.
        offset: usize,
        /// Maximum rows; `0` requests only the total.
        limit: usize,
    },
    /// `kind: "paramList"` — the document-level parameter list under the active
    /// configuration (ADR-0043 P1). No fields.
    ParamList,
    /// `kind: "mechanicalTree"` — the mechanical profile's bounded FeatureManager
    /// tree (features in evaluation order, then bodies). No fields.
    MechanicalTree,
    /// `kind: "actionRecorder"` — the session's recorder state and its visible
    /// macros (ADR-0056). No fields.
    ActionRecorder,
    /// `kind: "scenes"` — the open document's persisted scenes (SketchUp pages).
    /// No fields.
    Scenes,
    /// `kind: "display"` — the open document's Shadows/Fog record. No fields.
    Display,
}

impl HostReadRequest {
    /// The wire spelling of this request's `kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BimQuery { .. } => BIM_QUERY_KIND,
            Self::ParamList => PARAM_LIST_KIND,
            Self::MechanicalTree => MECHANICAL_TREE_KIND,
            Self::ActionRecorder => ACTION_RECORDER_KIND,
            Self::Scenes => SCENES_KIND,
            Self::Display => DISPLAY_KIND,
        }
    }
}

/// The answer to one [`HostReadRequest`], tagged by the same `kind`.
///
/// The inner value is the **unchanged** wire response of the underlying command;
/// a carrier unwraps it before answering, so this envelope never reaches a
/// frontend. It exists so a future kind can share the seam without a second
/// trait.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HostReadAnswer {
    /// `kind: "bimQuery"` — the page plus its facet summary.
    BimQuery(BimQueryResponse),
    /// `kind: "paramList"` — the document-level parameter list.
    ParamList(ParamListResponse),
    /// `kind: "mechanicalTree"` — the bounded FeatureManager tree.
    MechanicalTree(MechanicalTreeResponse),
    /// `kind: "actionRecorder"` — the recorder state and visible macros.
    ActionRecorder(ActionRecorderSnapshot),
    /// `kind: "scenes"` — the scene list.
    Scenes(ScenesResponse),
    /// `kind: "display"` — the Shadows/Fog record.
    Display(DisplayResponse),
}

/// Serialize a request as `{"kind": ..., <fields>}`.
///
/// Hand-written (rather than derived) so the tag spelling is [`BIM_QUERY_KIND`]
/// in exactly one place.
impl Serialize for HostReadRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::BimQuery {
                filter,
                offset,
                limit,
            } => {
                let mut state = serializer.serialize_struct("HostReadRequest", 4)?;
                state.serialize_field("kind", BIM_QUERY_KIND)?;
                state.serialize_field("filter", filter)?;
                state.serialize_field("offset", offset)?;
                state.serialize_field("limit", limit)?;
                state.end()
            }
            Self::ParamList => {
                let mut state = serializer.serialize_struct("HostReadRequest", 1)?;
                state.serialize_field("kind", PARAM_LIST_KIND)?;
                state.end()
            }
            Self::MechanicalTree => {
                let mut state = serializer.serialize_struct("HostReadRequest", 1)?;
                state.serialize_field("kind", MECHANICAL_TREE_KIND)?;
                state.end()
            }
            Self::ActionRecorder => {
                let mut state = serializer.serialize_struct("HostReadRequest", 1)?;
                state.serialize_field("kind", ACTION_RECORDER_KIND)?;
                state.end()
            }
            Self::Scenes => {
                let mut state = serializer.serialize_struct("HostReadRequest", 1)?;
                state.serialize_field("kind", SCENES_KIND)?;
                state.end()
            }
            Self::Display => {
                let mut state = serializer.serialize_struct("HostReadRequest", 1)?;
                state.serialize_field("kind", DISPLAY_KIND)?;
                state.end()
            }
        }
    }
}

/// Decode a request, naming an unknown `kind` with [`HOST_READ_UNKNOWN_KIND`].
///
/// The derived internally-tagged decoder would refuse an unknown variant with
/// serde's generic wording; this one returns the stable marker so a host (or an
/// evidence run) can classify it.
impl<'de> Deserialize<'de> for HostReadRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            kind: String,
            #[serde(default)]
            filter: Option<BimFilterWire>,
            #[serde(default)]
            offset: Option<usize>,
            #[serde(default)]
            limit: Option<usize>,
        }

        let raw = Raw::deserialize(deserializer)?;
        match raw.kind.as_str() {
            BIM_QUERY_KIND => {
                let filter = raw
                    .filter
                    .ok_or_else(|| serde::de::Error::missing_field("filter"))?;
                Ok(Self::BimQuery {
                    filter,
                    offset: raw.offset.unwrap_or(0),
                    limit: raw.limit.unwrap_or(BIM_DEFAULT_LIMIT),
                })
            }
            PARAM_LIST_KIND => Ok(Self::ParamList),
            MECHANICAL_TREE_KIND => Ok(Self::MechanicalTree),
            ACTION_RECORDER_KIND => Ok(Self::ActionRecorder),
            SCENES_KIND => Ok(Self::Scenes),
            DISPLAY_KIND => Ok(Self::Display),
            other => Err(serde::de::Error::custom(format!(
                "{HOST_READ_UNKNOWN_KIND}: `kind` {other:?} is not one of [{BIM_QUERY_KIND:?}, \
                 {PARAM_LIST_KIND:?}, {MECHANICAL_TREE_KIND:?}, {ACTION_RECORDER_KIND:?}, \
                 {SCENES_KIND:?}, {DISPLAY_KIND:?}]"
            ))),
        }
    }
}

/// The platform side of every request-carrying read.
///
/// A desktop carrier installs one of these (normally
/// `cad_desktop_shell::state::HostReadFn`) that marshals the request onto the
/// `!Send` session thread; a wasm carrier answers through
/// [`crate::session::CadSession::host_read`] directly. A carrier that installs
/// none keeps the per-kind named refusal.
///
/// # Why this trait lives in `cad-ui-core` (not `cad-desktop-shell`)
///
/// The registry row that names the seam lives here and both transports expand
/// it, so the trait must be visible to the wire layer. `cad-desktop-shell` only
/// adds the closure adapter. That is a layering constraint, not a preference.
///
/// Implementations must not hold their host's shell lock while waiting on the
/// session thread (the established seam contract), and must not re-enter
/// `Shared`.
pub trait HostReadHost: Send + Sync {
    /// Run one request and return its tagged answer.
    fn read(&self, request: &HostReadRequest) -> Result<HostReadAnswer, String>;
}

/// The optional [`HostReadHost`] handle a desktop `QueueReads` carries.
///
/// A newtype with its own `Debug` (the trait object has none) so the reads
/// struct keeps its `Debug`/`Clone`/`Copy`/`Default` derives.
#[derive(Clone, Copy, Default)]
pub struct HostReadSeam<'a>(pub Option<&'a Arc<dyn HostReadHost>>);

impl std::fmt::Debug for HostReadSeam<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("HostReadSeam")
            .field(&self.0.is_some())
            .finish()
    }
}

impl<'a> HostReadSeam<'a> {
    /// The installed host, when the carrier installed one.
    pub fn host(self) -> Option<&'a Arc<dyn HostReadHost>> {
        self.0
    }

    /// Run one request; no seam answers the kind's own refusal.
    pub fn read(self, request: &HostReadRequest) -> Result<HostReadAnswer, String> {
        match self.host() {
            Some(host) => host.read(request),
            None => Err(no_seam_refusal(request)),
        }
    }
}

/// The honest refusal of a carrier with no seam for this request's kind.
///
/// One arm per kind: a new kind must decide its own word here rather than
/// borrowing BIM's, so "no lane" is never reported under the wrong name.
pub fn no_seam_refusal(request: &HostReadRequest) -> String {
    match request {
        HostReadRequest::BimQuery { .. } => BIM_UNSUPPORTED.to_string(),
        HostReadRequest::ParamList => PARAM_LIST_UNSUPPORTED.to_string(),
        HostReadRequest::MechanicalTree => MECHANICAL_TREE_UNSUPPORTED.to_string(),
        HostReadRequest::ActionRecorder => ACTION_RECORDER_UNSUPPORTED.to_string(),
        HostReadRequest::Scenes => SCENES_UNSUPPORTED.to_string(),
        HostReadRequest::Display => DISPLAY_UNSUPPORTED.to_string(),
    }
}

/// The read seam every request-carrying wire row goes through on **both**
/// transports.
///
/// A wasm carrier implements it over the session it already holds; a desktop
/// carrier forwards to its [`HostReadSeam`]. The default is the honest
/// per-kind refusal, so a target with no lane needs no impl.
pub trait HostReads {
    /// Run one request-carrying read.
    fn host_read(&self, request: &HostReadRequest) -> Result<HostReadAnswer, String> {
        Err(no_seam_refusal(request))
    }
}

#[cfg(test)]
mod tests;
