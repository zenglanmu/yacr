//! The document-level parameter list wire (ADR-0043 P1).
//!
//! P1 exposes the list over the one request-carrying host-read seam
//! ([`crate::host_read`]); there is deliberately **no React panel** and no
//! dedicated wire command. The rows are a read-only projection of the base
//! parameter table plus the active configuration's status — never an edit path.
//!
//! # Suppressed rows
//!
//! The effective table omits a suppressed parameter, but the list still includes
//! it so a caller can see that the configuration removes it. Its `value` is the
//! **base** evaluation (what the row would be without the overlay), or `0.0` when
//! even the base table cannot evaluate. That is the honest reading of "the base
//! row exists but the active configuration suppresses it"; a fabricated effective
//! value would claim the parameter is currently in force.
//!
//! # Wire shape (frozen)
//!
//! ```jsonc
//! // HostReadRequest
//! { "kind": "paramList" }
//!
//! // HostReadAnswer
//! { "kind": "paramList",
//!   "active": "Small",           // null when the base table is in force
//!   "rows": [
//!     { "seq": 1, "name": "r", "kind": "length", "unit": "mm",
//!       "value": 5.0, "status": "overridden" },
//!     { "seq": 2, "name": "q", "kind": "number", "unit": null,
//!       "value": 10.0, "status": "inherited" },
//!     { "seq": 3, "name": "s", "kind": "length", "unit": "mm",
//!       "value": 7.0, "status": "suppressed" }
//!   ],
//!   "total": 3 }
//! ```

use serde::{Deserialize, Serialize};

/// The wire spelling of the parameter-list request kind.
pub const PARAM_LIST_KIND: &str = "paramList";

/// Stable refusal of a carrier that has no parameter-list read lane.
///
/// Its own word, never BIM's: "this carrier has no lane" and "this document has
/// no parameters" are different facts.
pub const PARAM_LIST_UNSUPPORTED: &str = "unsupported-runtime: paramList";

/// One parameter row of the document-level list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamListRow {
    /// The base table sequence (identity across renames).
    pub seq: u64,
    /// The base row's name.
    pub name: String,
    /// `length` | `number` | `angle`.
    pub kind: String,
    /// The canonical or declared unit token (`mm`, `rad`), or `null`.
    pub unit: Option<String>,
    /// The effective value; for a suppressed row, the base value (or `0.0`).
    pub value: f64,
    /// `inherited` | `overridden` | `suppressed`.
    pub status: String,
}

/// The whole document parameter list under the active configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamListResponse {
    /// The configuration currently applied, or `null` for the base table.
    pub active: Option<String>,
    /// Base rows in ascending sequence order.
    pub rows: Vec<ParamListRow>,
    /// `rows.len()`; kept explicit for the BIM page's `total` convention.
    pub total: usize,
}

impl ParamListResponse {
    /// Project the host's computed list onto the wire shape.
    pub fn from_host(list: cad_host::ParamList) -> Self {
        Self {
            active: list.active,
            total: list.total,
            rows: list
                .rows
                .into_iter()
                .map(|row| ParamListRow {
                    seq: row.seq,
                    name: row.name,
                    kind: row.kind,
                    unit: row.unit,
                    value: row.value,
                    status: row.status,
                })
                .collect(),
        }
    }
}
