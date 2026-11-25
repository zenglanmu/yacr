//! The mechanical domain's FeatureManager tree **wire** (ADR-0053 mechanical
//! stage): the camelCase answer of the one `cad_mechanical_tree` read.
//!
//! The backend (`cad_host::HostCore::mechanical_tree`) owns the projection: the
//! feature table in evaluation order, then the body rows, both bounded. This
//! module is only the transport shape; it never re-orders, filters or evaluates.
//!
//! # Named for what it is
//!
//! The read is a **tree projection**, not a general mechanical API: it answers
//! the design tree's rows and nothing else. Feature creation/evaluation,
//! assembly mates and mass properties are separate commands (`PARAM`,
//! `MASSPROP`, the solid commands) and do not travel through this envelope.
//!
//! # No silent claims
//!
//! Every row carries its own status token:
//!
//! * a body is `ready` when its exact B-rep is present and `unavailable` when it
//!   is not — the same fact the renderer's display cache reports;
//! * a feature is `schema_only`: this build stores `tables/features.json` but
//!   [`cad_db::DrawingDatabase::evaluate_feature`] refuses every operation with
//!   `rel.not_implemented`. The tree therefore shows recorded intent, never a
//!   claim that the feature produced geometry.
//!
//! A carrier with no tree lane answers [`MECHANICAL_TREE_UNSUPPORTED`] by name,
//! never an empty tree that would read as "this document has no features".
//!
//! # Wire shape (frozen)
//!
//! ```jsonc
//! // cad_mechanical_tree — no arguments
//! { "revision": 42,
//!   "rows": [
//!     { "kind": "feature", "id": 7, "token": "extrude", "status": "schema_only",
//!       "order": 0, "inputs": 1, "parameters": 2 },
//!     { "kind": "body", "id": 9, "token": "solid", "status": "ready",
//!       "order": null, "inputs": 0, "parameters": 0 }
//!   ],
//!   "total": 2, "truncated": false }
//! ```

use serde::{Deserialize, Serialize};

/// The wire spelling of the tree request kind.
pub const MECHANICAL_TREE_KIND: &str = "mechanicalTree";

/// Stable refusal of a carrier that has no mechanical-tree read lane.
///
/// Its own word, never the parameter list's: "this carrier has no lane" and
/// "this document has no features" are different facts.
pub const MECHANICAL_TREE_UNSUPPORTED: &str = "unsupported-runtime: mechanicalTree";

/// One row of the FeatureManager tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MechanicalTreeRow {
    /// `feature` | `body`.
    pub kind: String,
    /// The element row the node projects (a persistent entity id).
    pub id: u64,
    /// `extrude`/`revolve`/… for a feature; `solid`/`sheet`/`wire`/`unknown`
    /// for a body.
    pub token: String,
    /// `schema_only` (feature), `ready` or `unavailable` (body).
    pub status: String,
    /// Evaluation order for a feature; `null` for a body.
    pub order: Option<u32>,
    /// Input count for a feature; `0` for a body.
    pub inputs: usize,
    /// Bound parameter-slot count on the projecting element.
    pub parameters: usize,
}

/// The whole bounded tree answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MechanicalTreeResponse {
    /// The document revision the rows were read from (`0` with no document).
    pub revision: u64,
    /// The first page of rows, in tree order.
    pub rows: Vec<MechanicalTreeRow>,
    /// The whole tree's row count (not the page length).
    pub total: usize,
    /// Whether `rows` is shorter than `total`.
    pub truncated: bool,
}

impl MechanicalTreeResponse {
    /// Project the host's computed tree onto the wire shape.
    pub fn from_host(tree: cad_host::MechanicalTree) -> Self {
        Self {
            revision: tree.revision,
            total: tree.total,
            truncated: tree.truncated,
            rows: tree
                .rows
                .into_iter()
                .map(|row| MechanicalTreeRow {
                    kind: row.kind.to_string(),
                    id: row.id,
                    token: row.token,
                    status: row.status.to_string(),
                    order: row.order,
                    inputs: row.inputs,
                    parameters: row.parameters,
                })
                .collect(),
        }
    }
}
