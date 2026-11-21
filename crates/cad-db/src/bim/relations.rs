//! BIM relations: the edges between BIM components (ADR-0042 D3 tail / D12).
//!
//! Phase 1 stores per-element BIM data ([`super::BimElement`]) and the shared
//! definition rows ([`super::BimDefinitions`]). Phase 3 adds the many-to-many
//! edges an IFC (or DGN Item Type) file declares — `IfcRelAggregates`,
//! `IfcRelNests`, spatial containment, groups, connectivity — as **one bounded
//! table** rather than relationship elements:
//!
//! ```text
//! tables/bim-relations.json
//! [{"kind": "aggregates",
//!   "parent": {"kind": "source", "connector": "ifc", "document_id": "doc", "id": "P1"},
//!   "child":  {"kind": "source", "connector": "ifc", "document_id": "doc", "id": "C1"},
//!   "source": null,
//!   "label": null}]
//! ```
//!
//! # Why a table and not elements
//!
//! A relationship element would need a persisted identity kind
//! (`IdKind::Relationship`, 12) plus a counter and a high-water mark in
//! `document.json` (D12). This table uses **no** `PersistentId` at all: a row is
//! keyed by its own `(kind, parent, child)` triple, whose endpoints reuse
//! [`BimKey`] — the same `(connector, document_id, id)` / element equivalence
//! class the element `bim.source` block already encodes (D5). So the format
//! grows without touching `header.rs`, `IdCounters` or `IdKind`; D12's 11/12/13
//! stay reserved.
//!
//! # No dangling rule, but a strict shape
//!
//! Unlike [`super::validate`], a relation does **not** have to resolve to
//! stored elements: an IFC product may have semantics but no geometry row, and
//! its relation still belongs to the document. What *is* enforced is the shape
//! (endpoint strings non-empty and bounded, a known kind token) and the budget,
//! on **both** the write and the read side, so a hostile archive cannot make the
//! reader build an unbounded edge set.
//!
//! # Budgets and duplicate rows
//!
//! [`BIM_MAX_RELATIONS`] rows and [`BIM_MAX_RELATION_BYTES`] encoded bytes. An
//! over-budget table is refused (`ymodel.bim.budget_exceeded`), never truncated.
//! A second row with the same `(kind, parent, child)` is refused at
//! [`BimRelations::insert`] (`ymodel.bim.invalid`): the writer cannot merge two
//! edges one consumer would have to choose between, and the import path already
//! deduplicates its relation scan.

use std::collections::BTreeMap;

use cad_domain::CadResult;

use super::{check_text, to_cad_error, BimError, BimKey};

/// Most rows `tables/bim-relations.json` may hold (ADR-0042 D3 tail).
pub const BIM_MAX_RELATIONS: usize = 100_000;

/// Most bytes the encoded relation table may occupy (ADR-0042 D3 tail).
pub const BIM_MAX_RELATION_BYTES: usize = 32 * 1024 * 1024;

/// The kind of edge a relation row is (ADR-0042 D3 tail).
///
/// The five named kinds are the IFC relationship families this build reads;
/// every other source-specific edge is `other:<token>`. The token is an open
/// string (`other:rvt-assembly`) on purpose: a new connector must not need a
/// format change to store its own edge names, while the named kinds stay stable
/// so a query can recognise them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimRelationKind {
    /// `IfcRelAggregates`: whole → part.
    Aggregates,
    /// `IfcRelNests`: container → nested object.
    Nests,
    /// Spatial containment (`IfcRelContainedInSpatialStructure`).
    Contains,
    /// A grouping relation (`IfcRelAssignsToGroup`, systems).
    Group,
    /// A connectivity relation (`IfcRelConnects*`).
    Connects,
    /// `IfcRelVoidsElement`: host element → opening element (ADR-0043 P2).
    ///
    /// A D1 **reference** edge: P2 registers no `(host, hosted)` rule, so it is
    /// [`cad_domain::relation::Propagation::NoRule`] and the IFC exporter reports
    /// it as not written instead of inventing a wall-reflow heuristic.
    Voids,
    /// `IfcRelFillsElement`: opening element → filling element (ADR-0043 P2).
    ///
    /// Same reference family and `rel.no_rule` policy as [`Self::Voids`].
    Fills,
    /// Any other source edge, spelled `other:<token>`.
    Other(String),
}

impl BimRelationKind {
    /// The prefix of the open variant's spelled token.
    pub const OTHER_PREFIX: &'static str = "other:";

    /// The persisted spelling. Part of the format.
    pub fn as_token(&self) -> String {
        match self {
            BimRelationKind::Aggregates => "aggregates".to_string(),
            BimRelationKind::Nests => "nests".to_string(),
            BimRelationKind::Contains => "contains".to_string(),
            BimRelationKind::Group => "group".to_string(),
            BimRelationKind::Connects => "connects".to_string(),
            BimRelationKind::Voids => "voids".to_string(),
            BimRelationKind::Fills => "fills".to_string(),
            BimRelationKind::Other(token) => format!("{}{token}", Self::OTHER_PREFIX),
        }
    }

    /// Parse the persisted spelling.
    ///
    /// An unknown token, or `other:` with an empty token, is a malformed
    /// document rather than a new kind this build silently accepts.
    pub fn parse(value: &str) -> Option<BimRelationKind> {
        match value {
            "aggregates" => Some(BimRelationKind::Aggregates),
            "nests" => Some(BimRelationKind::Nests),
            "contains" => Some(BimRelationKind::Contains),
            "group" => Some(BimRelationKind::Group),
            "connects" => Some(BimRelationKind::Connects),
            "voids" => Some(BimRelationKind::Voids),
            "fills" => Some(BimRelationKind::Fills),
            other => other
                .strip_prefix(Self::OTHER_PREFIX)
                .filter(|token| !token.trim().is_empty())
                .map(|token| BimRelationKind::Other(token.to_string())),
        }
    }
}

/// One edge between two BIM components (ADR-0042 D3 tail).
///
/// `parent`/`child`/`source` reuse [`BimKey`] rather than inventing a second
/// endpoint type: a relation points at the same logical component an element's
/// `bim.source` block groups, so the two cannot drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimRelation {
    /// What kind of edge this is.
    pub kind: BimRelationKind,
    /// The whole / container / group side.
    pub parent: BimKey,
    /// The part / nested / member side.
    pub child: BimKey,
    /// The relationship object's own source identity, when the source declared
    /// one (an IFC relationship's `GlobalId`).
    pub source: Option<BimKey>,
    /// A human-readable label, when the source declared one.
    pub label: Option<String>,
}

/// `tables/bim-relations.json`: the edges, keyed by `(kind, parent, child)`.
///
/// A map, not a vector, so the writer is deterministic and a duplicate edge is
/// refused at [`Self::insert`]. Empty for every document without relations, and
/// the writer then emits no table at all (which keeps such a document at the
/// minor its other features imply).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BimRelations {
    rows: BTreeMap<(BimRelationKind, BimKey, BimKey), BimRelation>,
}

impl BimRelations {
    /// An empty table.
    pub fn new() -> BimRelations {
        BimRelations::default()
    }

    /// Insert one edge.
    ///
    /// A duplicate `(kind, parent, child)` is refused. The `source` and `label`
    /// fields are not part of the key: two rows with the same endpoints and kind
    /// but different labels would still be one edge with an ambiguous label, and
    /// the writer cannot merge them.
    pub fn insert(&mut self, row: BimRelation) -> Result<(), BimError> {
        validate_relation(&row)?;
        let key = (row.kind.clone(), row.parent.clone(), row.child.clone());
        if self.rows.contains_key(&key) {
            return Err(BimError::invalid(format!(
                "duplicate relation {} {} -> {}",
                row.kind.as_token(),
                describe_key(&row.parent),
                describe_key(&row.child)
            )));
        }
        self.rows.insert(key, row);
        Ok(())
    }

    /// The edge with this exact `(kind, parent, child)`, if it is stored.
    pub fn get(
        &self,
        kind: &BimRelationKind,
        parent: &BimKey,
        child: &BimKey,
    ) -> Option<&BimRelation> {
        self.rows
            .get(&(kind.clone(), parent.clone(), child.clone()))
    }

    /// Every edge, in `(kind, parent, child)` order.
    pub fn iter(&self) -> impl Iterator<Item = &BimRelation> {
        self.rows.values()
    }

    /// How many edges the table holds.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table is empty (and therefore not written).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Adopt an already-unique row set (the decoder's path).
    pub(crate) fn from_rows(
        rows: BTreeMap<(BimRelationKind, BimKey, BimKey), BimRelation>,
    ) -> BimRelations {
        BimRelations { rows }
    }
}

impl<'a> IntoIterator for &'a BimRelations {
    type Item = &'a BimRelation;
    type IntoIter =
        std::collections::btree_map::Values<'a, (BimRelationKind, BimKey, BimKey), BimRelation>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.values()
    }
}

/// Check one endpoint's shape and per-string budget.
fn check_key(what: &str, key: &BimKey) -> Result<(), BimError> {
    match key {
        BimKey::Source {
            connector,
            document_id,
            id,
        } => {
            check_text(&format!("{what}.connector"), connector, false)?;
            check_text(&format!("{what}.document_id"), document_id, false)?;
            check_text(&format!("{what}.id"), id, false)
        }
        BimKey::Entity(_) => Ok(()),
    }
}

/// A short spelling for a refusal detail; not part of the format.
fn describe_key(key: &BimKey) -> String {
    match key {
        BimKey::Source {
            connector,
            document_id,
            id,
        } => format!("{connector}:{document_id}:{id}"),
        BimKey::Entity(entity) => format!("entity:{}", entity.0),
    }
}

/// Check one relation row's own shape and budgets (no cross-references).
pub fn validate_relation(row: &BimRelation) -> Result<(), BimError> {
    let token = row.kind.as_token();
    check_text("relation.kind", &token, false)?;
    if let BimRelationKind::Other(other) = &row.kind {
        if other.trim().is_empty() {
            return Err(BimError::invalid("relation.kind: empty other token"));
        }
    }
    check_key("relation.parent", &row.parent)?;
    check_key("relation.child", &row.child)?;
    if let Some(source) = &row.source {
        check_key("relation.source", source)?;
    }
    if let Some(label) = &row.label {
        check_text("relation.label", label, false)?;
    }
    Ok(())
}

/// Check a whole relation table: row count, row shapes and encoded size.
///
/// The builder's `finish()` gate calls this. It does **not** require the
/// endpoints to name stored elements (see the module docs); it only refuses a
/// malformed or over-budget set, and never truncates.
pub fn validate_relations(relations: &BimRelations) -> Result<(), BimError> {
    if relations.len() > BIM_MAX_RELATIONS {
        return Err(BimError::budget(format!(
            "{} relation rows exceeds the {BIM_MAX_RELATIONS} budget",
            relations.len()
        )));
    }
    for row in relations.iter() {
        validate_relation(row)?;
    }
    let encoded = crate::snapshot::relations::relations_encoded_len(relations)
        .map_err(|error| BimError::invalid(error.detail))?;
    if encoded > BIM_MAX_RELATION_BYTES {
        return Err(BimError::budget(format!(
            "the relation table encodes to {encoded} bytes, over the \
             {BIM_MAX_RELATION_BYTES}-byte budget"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Builder path.
//
// Lives here, not in `builder.rs` (991 lines, ADR-0042 risk 6): the builder
// exposes its `database` field crate-wide and this module owns the relation side
// of the import contract, exactly like the element/definition setters in
// `bim.rs`.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Replace the BIM relation table (ADR-0042 D3 tail).
    ///
    /// Every row is shape-checked here; the count/encoded-size budgets are
    /// checked once at [`Self::finish`], where the whole table exists. Nothing
    /// is truncated: a duplicate row or an over-budget table is refused.
    pub fn set_bim_relations(&mut self, relations: BimRelations) -> CadResult<()> {
        for row in &relations {
            validate_relation(row).map_err(to_cad_error)?;
        }
        self.database.bim_relations = relations;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
