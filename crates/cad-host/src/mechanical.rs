//! The mechanical domain's FeatureManager tree read (ADR-0053 mechanical stage).
//!
//! The mechanical profile's left tray is a SolidWorks-style FeatureManager
//! design tree. The tree is a **projection of the document**, never a second
//! model: the authority stays in `cad-db`
//!
//! * `tables/features.json` — the ordered feature definitions attached to
//!   element rows (ADR-0043 P2). This build has the **schema half** only:
//!   [`cad_db::DrawingDatabase::evaluate_feature`] refuses every operation with
//!   `rel.not_implemented`, so every feature row projects as `schema_only`
//!   rather than pretending it produced geometry;
//! * the element rows whose geometry is a [`cad_domain::Body3d`] — the part's
//!   bodies. A row reports its topology token and whether the exact B-rep is
//!   present, which is exactly what the display cache does or does not show.
//!
//! # Read-only and bounded
//!
//! Nothing here writes, evaluates a feature, tessellates a body or copies a
//! B-rep. Rows are ordered the way the reference tree orders them — features in
//! evaluation order first, then bodies by entity id — and the answer is capped at
//! [`MECHANICAL_TREE_MAX_ROWS`]. `total` always counts the whole tree and
//! `truncated` states that the page is short, so a bounded read can never read as
//! "this document has fewer rows".
//!
//! No document is an explicit empty answer (`revision: 0`, `rows: []`,
//! `total: 0`), never an error: the caller asked a real question and "nothing is
//! open" is its real answer.

use cad_db::FeatureOperation;
use cad_domain::{BodyKind, SemanticGeometry};

use crate::HostCore;

/// Most rows one [`MechanicalTree`] answer carries.
///
/// A tree is a navigation surface, not a data export; a document with more rows
/// gets the first page plus `truncated: true` and its real `total`.
pub const MECHANICAL_TREE_MAX_ROWS: usize = 1024;

/// One row of the FeatureManager tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MechanicalTreeRow {
    /// `"feature"` or `"body"`.
    pub kind: &'static str,
    /// The element row the node projects (a persistent entity id).
    pub id: u64,
    /// The operation token for a feature, the topology token for a body.
    pub token: String,
    /// `schema_only` (feature), `ready` or `unavailable` (body).
    pub status: &'static str,
    /// Evaluation order; `None` for a body.
    pub order: Option<u32>,
    /// Input count (features), `0` for a body.
    pub inputs: usize,
    /// Bound parameter-slot count on the projecting element.
    pub parameters: usize,
}

/// The document's mechanical tree, bounded to the first
/// [`MECHANICAL_TREE_MAX_ROWS`] rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MechanicalTree {
    /// The revision the rows were read from (`0` with no document).
    pub revision: u64,
    /// The first page of rows, in tree order.
    pub rows: Vec<MechanicalTreeRow>,
    /// The whole tree's row count (not the page length).
    pub total: usize,
    /// Whether `rows` is shorter than `total`.
    pub truncated: bool,
}

/// The persisted topology token of one body.
fn body_token(kind: &BodyKind) -> &'static str {
    match kind {
        BodyKind::Wire => "wire",
        BodyKind::Sheet => "sheet",
        BodyKind::Solid => "solid",
        BodyKind::Unknown => "unknown",
    }
}

/// The feature table's token is already persisted; re-use it so the wire cannot
/// drift from `tables/features.json`.
fn feature_token(operation: FeatureOperation) -> String {
    operation.as_token().to_string()
}

impl HostCore {
    /// The open document's FeatureManager tree, in reference order.
    ///
    /// Runtime content only: the projection reads the current database and
    /// never mutates it, so calling it between edits is always safe.
    pub fn mechanical_tree(&self) -> MechanicalTree {
        let controller = self.controller.borrow();
        let Some(database) = controller.drawing() else {
            return MechanicalTree::default();
        };
        let features = database.features();
        let body_count = database
            .entities()
            .filter(|entity| matches!(entity.geometry, SemanticGeometry::Body3d(_)))
            .count();
        let total = features.len() + body_count;
        let mut rows = Vec::with_capacity(total.min(MECHANICAL_TREE_MAX_ROWS));
        // Features in evaluation order: the reference design tree is
        // chronological, and `ordered()` is the table's own total order.
        for feature in features.ordered() {
            if rows.len() >= MECHANICAL_TREE_MAX_ROWS {
                break;
            }
            rows.push(MechanicalTreeRow {
                kind: "feature",
                // The same `u64` id spelling the selection wire already uses;
                // the database mints ids far inside that range.
                id: feature.id.0 as u64,
                token: feature_token(feature.operation),
                status: "schema_only",
                order: Some(feature.order),
                inputs: feature.inputs.len(),
                parameters: feature.parameters.len(),
            });
        }
        // Then the bodies, by entity id. Feature rows above already own their
        // element ids; a body row is a *separate* node (the same element can be
        // both the feature's target and a body), which matches the reference's
        // "feature + body folder" shape without inventing a parent link.
        for entity in database.entities() {
            if rows.len() >= MECHANICAL_TREE_MAX_ROWS {
                break;
            }
            let SemanticGeometry::Body3d(body) = &entity.geometry else {
                continue;
            };
            let exact = body.is_exact();
            rows.push(MechanicalTreeRow {
                kind: "body",
                id: entity.id.0 as u64,
                token: body_token(&body.kind).to_string(),
                status: if exact { "ready" } else { "unavailable" },
                order: None,
                inputs: 0,
                parameters: database
                    .param_bindings(entity.id)
                    .map(|bindings| bindings.len())
                    .unwrap_or(0),
            });
        }
        MechanicalTree {
            revision: database.revision().0,
            total,
            truncated: total > rows.len(),
            rows,
        }
    }
}
