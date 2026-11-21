//! The BIM query/summary **wire** (ADR-0042 D11): request parsing and the
//! camelCase answer the two `cad_bim_*` commands return.
//!
//! The backend (`cad_query::bim`) owns every rule — the index, the filter
//! semantics, the ordering, the paging and the counters. This module owns only
//! the transport shape: it turns the wire filter into a [`BimFilter`], maps the
//! backend rows into JSON, and refuses what it cannot express instead of
//! guessing.
//!
//! # Why the mapping is explicit
//!
//! `cad_query`'s types deliberately carry no serde derive (they are backend
//! values, not wire values). Every field below is therefore written out once, by
//! hand, so a backend field rename cannot silently change the frontend contract:
//! the compiler catches it here.
//!
//! # No silent narrowing
//!
//! * An unknown `value.kind` is [`BIM_UNSUPPORTED_VALUE`], never coerced (a
//!   string that looks like a number stays a string).
//! * A non-finite `real` is [`BIM_REAL_NOT_FINITE`], never rounded or clamped.
//! * Ids travel as `u64` (the frontend's `number`); the database mints ids from a
//!   monotonic counter, far inside that range.
//! * A carrier that cannot serve the query answers [`BIM_UNSUPPORTED`] by name —
//!   never an empty page that would read as "this document has no BIM data".
//!
//! # Wire shape (frozen)
//!
//! ```jsonc
//! // args of both commands
//! { "filter": { "categories": [], "levels": [], "types": [], "materials": [],
//!               "property": { "pset": "Pset_WallCommon", "property": "IsExternal",
//!                             "op": "eq",
//!                             "value": { "kind": "bool", "value": true } },
//!               "evidence": "declared" },
//!   "offset": 0, "limit": 100 }
//!
//! // cad_bim_query
//! { "databaseId": 1, "revision": 42, "total": 12, "offset": 0, "limit": 100,
//!   "rows": [], "stale": false }
//!
//! // cad_bim_summary
//! { "databaseId": 1, "revision": 42, "total": 12,
//!   "summary": { "total": 12, "byCategory": [], "byLevel": [], "byType": [],
//!                "byMaterial": [] } }
//! ```

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use cad_app::cad_query::bim::{
    BimFacetCount, BimFilter, BimPropertyCount, BimPropertyFilter, BimPropertyOp, BimReal, BimRow,
    BimScalar, BimSummary,
};
use cad_app::cad_query::QueryPage;
use cad_db::{BimEvidence, BimKey, BimSource};
use cad_domain::DatabaseId;

// ---------------------------------------------------------------------- seam ---

/// The platform side of the two `cad_bim_*` commands.
///
/// A BIM query reads the **open document**, which on the desktop carriers lives
/// in the `!Send` owner-thread session (GTK thread-local on Linux). The shared
/// command layer therefore reaches it through this seam exactly like
/// [`ShellYmodelHost`] does for the save preview: the host marshals the call onto
/// its session thread and returns the wire answer.
///
/// A carrier that installs no seam has no BIM query lane and answers
/// [`BIM_UNSUPPORTED`] by name. That is a different fact from "this document has
/// no BIM data" (an empty page), and the two must never be conflated.
///
/// Implementations must not lock their host's shell state while waiting on the
/// session thread; the command clones the seam out of `Shared` and releases the
/// shell lock first (the established seam contract).
///
/// The desktop shell's ymodel seam has the same shape
/// (`cad_desktop_shell::state::ShellYmodelHost`).
pub trait BimHost: Send + Sync {
    /// Run one filtered, paged query.
    fn bim_query(&self, request: &BimQueryRequest) -> Result<BimQueryResponse, String>;

    /// Run one filtered summary.
    fn bim_summary(&self, request: &BimQueryRequest) -> Result<BimSummaryResponse, String>;
}

/// The read seam the two `cad_bim_*` wire commands go through on **both**
/// transports.
///
/// It is deliberately separate from the session's `ControlReads`: those are
/// owner-tick projections (no arguments, mirrored ahead of time), while these are
/// **request-carrying** reads. A wasm carrier implements it over the session it
/// already holds; a desktop carrier implements it over [`BimSeam`] (the host's
/// [`BimHost`]). The default is the honest refusal, so a target with no lane
/// needs no impl.
pub trait BimReads {
    /// One filtered, paged query.
    fn bim_query(&self, _request: &BimQueryRequest) -> Result<BimQueryResponse, String> {
        Err(BIM_UNSUPPORTED.to_string())
    }

    /// Counts over one filtered set.
    fn bim_summary(&self, _request: &BimQueryRequest) -> Result<BimSummaryResponse, String> {
        Err(BIM_UNSUPPORTED.to_string())
    }
}

/// The optional [`BimHost`] handle a desktop `QueueReads` carries.
///
/// A newtype with its own `Debug` (the trait object has none) so the reads struct
/// keeps its `Debug`/`Clone`/`Copy`/`Default` derives without listing the seam.
#[derive(Clone, Copy, Default)]
pub struct BimSeam<'a>(pub Option<&'a std::sync::Arc<dyn BimHost>>);

impl std::fmt::Debug for BimSeam<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("BimSeam")
            .field(&self.0.is_some())
            .finish()
    }
}

impl<'a> BimSeam<'a> {
    /// The installed host, when the carrier installed one.
    pub fn host(self) -> Option<&'a std::sync::Arc<dyn BimHost>> {
        self.0
    }
}

/// Stable refusal of a carrier that has no BIM query lane at all.
///
/// The same word on every host: the React panel keys on it to hide the BIM
/// table instead of showing an empty one (which would claim the document has no
/// BIM data).
pub const BIM_UNSUPPORTED: &str = "unsupported-runtime: bimQuery";

/// Stable refusal of a filter value whose `kind` this transport cannot express.
pub const BIM_UNSUPPORTED_VALUE: &str = "bim.filter.unsupported_value";

/// Stable refusal of a filter real that is not finite.
///
/// The backend only ever stores finite reals, so this can only come from a
/// caller; it is refused by name instead of being rounded into some neighbour.
pub const BIM_REAL_NOT_FINITE: &str = "bim.filter.real_not_finite";

/// Stable refusal of a query bound to another document.
///
/// The backend's [`cad_domain::CadError::StaleResult`], named for the wire: the
/// caller must re-run the query against the document that is open now.
pub const BIM_STALE: &str = "bim.stale: the query document is not the open document";

/// The page size a request that omits `limit` gets (the frontend always sends
/// one; this is the documented default for a hand-written request).
pub const BIM_DEFAULT_LIMIT: usize = 100;

// ------------------------------------------------------------------- request ---

/// One wire BIM filter: the facets the backend ANDs, plus one property predicate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimFilterWire {
    /// Category tokens (`IfcWall`, …).
    #[serde(default)]
    pub categories: Vec<String>,
    /// Level definition ids or display names.
    #[serde(default)]
    pub levels: Vec<String>,
    /// Type definition ids or display names.
    #[serde(default)]
    pub types: Vec<String>,
    /// Material definition ids or display names.
    #[serde(default)]
    pub materials: Vec<String>,
    /// One property predicate.
    #[serde(default)]
    pub property: Option<BimPropertyFilterWire>,
    /// Restrict to one evidence class; `None` means "no evidence filter".
    #[serde(default)]
    pub evidence: Option<BimEvidenceWire>,
}

/// One wire property predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimPropertyFilterWire {
    /// Property-set name (`Pset_WallCommon`, …).
    pub pset: String,
    /// Property name inside that set.
    pub property: String,
    /// The comparison.
    pub op: BimOpWire,
    /// The value compared against.
    pub value: BimValueWire,
}

/// The wire spelling of a property comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BimOpWire {
    /// Equal (`Real`/`Integer` cross-compare numerically).
    Eq,
    /// Not equal, for elements that declare the property.
    Ne,
    /// Substring match for text values.
    Contains,
}

impl BimOpWire {
    /// The backend comparison.
    pub fn to_op(self) -> BimPropertyOp {
        match self {
            BimOpWire::Eq => BimPropertyOp::Eq,
            BimOpWire::Ne => BimPropertyOp::Ne,
            BimOpWire::Contains => BimPropertyOp::Contains,
        }
    }
}

/// How the BIM data was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BimEvidenceWire {
    /// The source stated it.
    Declared,
    /// A heuristic produced it.
    Heuristic,
}

impl BimEvidenceWire {
    /// The stored evidence class.
    pub fn to_evidence(self) -> BimEvidence {
        match self {
            BimEvidenceWire::Declared => BimEvidence::Declared,
            BimEvidenceWire::Heuristic => BimEvidence::Heuristic,
        }
    }

    /// The wire spelling of a stored evidence class.
    pub fn from_evidence(evidence: BimEvidence) -> BimEvidenceWire {
        match evidence {
            BimEvidence::Declared => BimEvidenceWire::Declared,
            BimEvidence::Heuristic => BimEvidenceWire::Heuristic,
        }
    }
}

/// One wire filter value, tagged by `kind`.
///
/// The four supported kinds are the backend's own closed set. [`Self::Unsupported`]
/// exists only so an unknown `kind` reaches [`BimQueryRequest::filter`] intact and
/// is refused with [`BIM_UNSUPPORTED_VALUE`] at conversion time — the answer says
/// *which* kind was sent, instead of a generic serde "unknown variant".
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BimValueWire {
    /// A boolean.
    Bool {
        /// The value.
        value: bool,
    },
    /// A signed integer.
    Integer {
        /// The value.
        value: i64,
    },
    /// A finite real number.
    Real {
        /// The value; never rounded.
        value: f64,
    },
    /// A string.
    Text {
        /// The value.
        value: String,
    },
    /// A `kind` this transport cannot express (already refused by [`Self::to_scalar`]).
    Unsupported {
        /// The caller's own `kind` token (not named `kind`: serde refuses a field
        /// that collides with the internal tag).
        token: String,
    },
}

impl<'de> Deserialize<'de> for BimValueWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let object = value.as_object().ok_or_else(|| {
            serde::de::Error::custom("a BIM filter value must be an object with a `kind`")
        })?;
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| serde::de::Error::custom("a BIM filter value needs a string `kind`"))?;
        let raw = object.get("value");
        let missing =
            || serde::de::Error::custom(format!("bim value `{kind}` has no usable `value`"));
        Ok(match kind {
            "bool" => BimValueWire::Bool {
                value: raw.and_then(Value::as_bool).ok_or_else(missing)?,
            },
            "integer" => BimValueWire::Integer {
                value: raw.and_then(Value::as_i64).ok_or_else(missing)?,
            },
            "real" => BimValueWire::Real {
                value: raw.and_then(Value::as_f64).ok_or_else(missing)?,
            },
            "text" => BimValueWire::Text {
                value: raw.and_then(Value::as_str).ok_or_else(missing)?.to_string(),
            },
            other => BimValueWire::Unsupported {
                token: other.to_string(),
            },
        })
    }
}

impl BimValueWire {
    /// The backend scalar, or the explicit refusal.
    pub fn to_scalar(&self) -> Result<BimScalar, String> {
        Ok(match self {
            BimValueWire::Bool { value } => BimScalar::Bool(*value),
            BimValueWire::Integer { value } => BimScalar::Integer(*value),
            BimValueWire::Real { value } => {
                if !value.is_finite() {
                    return Err(format!("{BIM_REAL_NOT_FINITE}: {value}"));
                }
                BimScalar::Real(BimReal::new(*value))
            }
            BimValueWire::Text { value } => BimScalar::Text(value.clone()),
            BimValueWire::Unsupported { token } => {
                return Err(format!(
                    "{BIM_UNSUPPORTED_VALUE}: `kind` {token:?} is not one of \
                     bool|integer|real|text"
                ))
            }
        })
    }
}

impl BimFilterWire {
    /// The backend filter, or the explicit refusal of a value this wire cannot
    /// express.
    pub fn to_filter(&self) -> Result<BimFilter, String> {
        let property = match &self.property {
            Some(property) => Some(BimPropertyFilter::new(
                property.pset.clone(),
                property.property.clone(),
                property.op.to_op(),
                property.value.to_scalar()?,
            )),
            None => None,
        };
        Ok(BimFilter {
            categories: self.categories.clone(),
            levels: self.levels.clone(),
            types: self.types.clone(),
            materials: self.materials.clone(),
            property,
            evidence: self.evidence.map(BimEvidenceWire::to_evidence),
        })
    }
}

/// One BIM request: the filter plus the page window.
///
/// Both commands take this same object. `offset`/`limit` page the **rows** of
/// `cad_bim_query`; a summary is one logical row, so its command always asks for
/// exactly that row (see [`crate::session::CadSession::bim_summary`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimQueryRequest {
    /// What to select.
    pub filter: BimFilterWire,
    /// Rows to skip (never negative: it is a `usize`).
    #[serde(default)]
    pub offset: usize,
    /// Maximum rows; `0` requests only the total (the `QueryPage` contract),
    /// `BIM_DEFAULT_LIMIT` when omitted.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    BIM_DEFAULT_LIMIT
}

impl BimQueryRequest {
    /// A request from the three decoded wire arguments.
    pub fn new(filter: BimFilterWire, offset: usize, limit: usize) -> BimQueryRequest {
        BimQueryRequest {
            filter,
            offset,
            limit,
        }
    }
}

// ------------------------------------------------------------------ responses ---

/// One logical component's identity on the wire.
///
/// Two states: a source identifier (`{"kind":"source",…}`) or a bare entity
/// (`{"kind":"entity","entityId":7}`). The same key also appears in
/// [`BimRowWire::entity_ids`]'s ordering context, so both must be expressible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BimObjectWire {
    /// The `(connector, document, id)` equivalence class.
    Source {
        /// Connector id (`"ifc"`, …).
        connector: String,
        /// The source document's own identifier.
        document_id: String,
        /// The element's identifier inside that document.
        id: String,
    },
    /// A component with no usable source identifier: the entity itself.
    Entity {
        /// The geometry row that is the component.
        entity_id: u64,
    },
}

impl BimObjectWire {
    /// Project a backend component key.
    pub fn from_key(key: &BimKey) -> BimObjectWire {
        match key {
            BimKey::Source {
                connector,
                document_id,
                id,
            } => BimObjectWire::Source {
                connector: connector.clone(),
                document_id: document_id.clone(),
                id: id.clone(),
            },
            BimKey::Entity(entity) => BimObjectWire::Entity {
                entity_id: wire_id(entity.0),
            },
        }
    }
}

/// One element's provenance on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimSourceWire {
    /// Connector id (`"ifc"`, `"rvt"`, `"dgn"`).
    pub connector: String,
    /// The source document's own identifier, when it had one.
    pub document_id: Option<String>,
    /// The element's identifier inside that document, when it had one.
    pub id: Option<String>,
    /// The source-native id, for diagnostics.
    pub native_id: Option<String>,
}

impl BimSourceWire {
    /// Project a stored source.
    pub fn from_source(source: &BimSource) -> BimSourceWire {
        BimSourceWire {
            connector: source.connector.clone(),
            document_id: source.document_id.clone(),
            id: source.id.clone(),
            native_id: source.native_id.clone(),
        }
    }
}

/// One property set in a row's summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimPropertyCountWire {
    /// Property-set name.
    pub name: String,
    /// Properties the set holds.
    pub count: usize,
}

impl BimPropertyCountWire {
    /// Project one backend count.
    pub fn from_count(count: &BimPropertyCount) -> BimPropertyCountWire {
        BimPropertyCountWire {
            name: count.name.clone(),
            count: count.count,
        }
    }
}

/// One query row: one logical component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimRowWire {
    /// The component identity.
    pub object: BimObjectWire,
    /// Every geometry row of the component, ascending.
    pub entity_ids: Vec<u64>,
    /// The element category token, as declared.
    pub category: Option<String>,
    /// The element display name, as declared.
    pub name: Option<String>,
    /// The referenced level's display name.
    pub level_name: Option<String>,
    /// The referenced type's display name.
    pub type_name: Option<String>,
    /// The referenced material's display name.
    pub material_name: Option<String>,
    /// The representative element's provenance.
    pub source: BimSourceWire,
    /// The representative element's evidence class.
    pub evidence: BimEvidenceWire,
    /// The representative element's property sets.
    pub property_summary: Vec<BimPropertyCountWire>,
}

impl BimRowWire {
    /// Project a backend row. Only the wire spelling changes; no field is
    /// dropped, reordered or invented.
    pub fn from_row(row: &BimRow) -> BimRowWire {
        BimRowWire {
            object: BimObjectWire::from_key(&row.object),
            entity_ids: row.entity_ids.iter().map(|id| wire_id(id.0)).collect(),
            category: row.category.clone(),
            name: row.name.clone(),
            level_name: row.level_name.clone(),
            type_name: row.type_name.clone(),
            material_name: row.material_name.clone(),
            source: BimSourceWire::from_source(&row.source),
            evidence: BimEvidenceWire::from_evidence(row.evidence),
            property_summary: row
                .property_summary
                .iter()
                .map(BimPropertyCountWire::from_count)
                .collect(),
        }
    }
}

/// The `cad_bim_query` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimQueryResponse {
    /// The database the page was answered from; `0` when no document is open.
    pub database_id: u64,
    /// The database revision the page is bound to; `0` when no document is open.
    pub revision: u64,
    /// Matched components (not rows of the current page).
    pub total: usize,
    /// The window the caller asked for.
    pub offset: usize,
    /// The window the caller asked for (`0` = total only, per `QueryPage`).
    pub limit: usize,
    /// The page's rows.
    pub rows: Vec<BimRowWire>,
    /// Always `false` on a successful answer: a query bound to another document
    /// is [`BIM_STALE`], an error, never a successful page flagged stale.
    pub stale: bool,
}

/// One bucket of a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimFacetCountWire {
    /// Grouping key (folded category token, or definition id).
    pub key: String,
    /// Display spelling.
    pub name: String,
    /// Components in the bucket.
    pub count: usize,
}

impl BimFacetCountWire {
    /// Project one backend bucket.
    pub fn from_count(count: &BimFacetCount) -> BimFacetCountWire {
        BimFacetCountWire {
            key: count.key.clone(),
            name: count.name.clone(),
            count: count.count,
        }
    }
}

/// Counts over the filtered components.
///
/// Counting goes through the component key, so one source element split into
/// several geometry rows counts once. Facets only count components that declare
/// them, so they may sum to less than [`Self::total`] (no "unknown" bucket is
/// fabricated).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimSummaryWire {
    /// Matched components.
    pub total: usize,
    /// Components per category token.
    pub by_category: Vec<BimFacetCountWire>,
    /// Components per level definition.
    pub by_level: Vec<BimFacetCountWire>,
    /// Components per type definition.
    pub by_type: Vec<BimFacetCountWire>,
    /// Components per material definition.
    pub by_material: Vec<BimFacetCountWire>,
}

impl BimSummaryWire {
    /// Project a backend summary.
    pub fn from_summary(summary: &BimSummary) -> BimSummaryWire {
        BimSummaryWire {
            total: summary.total,
            by_category: summary
                .by_category
                .iter()
                .map(BimFacetCountWire::from_count)
                .collect(),
            by_level: summary
                .by_level
                .iter()
                .map(BimFacetCountWire::from_count)
                .collect(),
            by_type: summary
                .by_type
                .iter()
                .map(BimFacetCountWire::from_count)
                .collect(),
            by_material: summary
                .by_material
                .iter()
                .map(BimFacetCountWire::from_count)
                .collect(),
        }
    }
}

/// The `cad_bim_summary` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BimSummaryResponse {
    /// The database the summary was answered from; `0` when no document is open.
    pub database_id: u64,
    /// The database revision the summary is bound to; `0` with no document.
    pub revision: u64,
    /// Matched components (the same number as `summary.total`).
    pub total: usize,
    /// The counts.
    pub summary: BimSummaryWire,
}

// ----------------------------------------------------------------- conversion ---

/// A database id on the wire.
///
/// Ids are minted from a monotonic counter; the `u64` wire type is the
/// frontend's `number` and is far wider than any id this build mints. The `0`
/// answer means "no document is open".
fn wire_id(id: u128) -> u64 {
    id as u64
}

fn wire_database(database: Option<DatabaseId>) -> u64 {
    database.map(|id| wire_id(id.0)).unwrap_or(0)
}

/// Build the `cad_bim_query` answer from one backend page.
pub fn query_response(page: QueryPage<BimRow>, database: Option<DatabaseId>) -> BimQueryResponse {
    BimQueryResponse {
        database_id: wire_database(database),
        revision: page.request.revision.0,
        total: page.total,
        offset: page.request.offset,
        limit: page.request.limit,
        rows: page.rows.iter().map(BimRowWire::from_row).collect(),
        // A stale document is the backend's `StaleResult` error, not a page.
        stale: false,
    }
}

/// Build the `cad_bim_summary` answer from the backend's single summary row.
///
/// A page with no row (only possible when the caller asked for `limit = 0`)
/// answers an empty summary and a `total` of `0`: nothing was counted because
/// nothing was returned, which is the `QueryPage` contract rather than a
/// fabricated zero-component document.
pub fn summary_response(
    page: QueryPage<BimSummary>,
    database: Option<DatabaseId>,
) -> BimSummaryResponse {
    let summary = page
        .rows
        .first()
        .map(BimSummaryWire::from_summary)
        .unwrap_or_default();
    BimSummaryResponse {
        database_id: wire_database(database),
        revision: page.request.revision.0,
        total: summary.total,
        summary,
    }
}

#[cfg(test)]
mod tests;
