//! Self-authored in-context relations (`tables/relations.json`, ADR-0043 P3).
//!
//! Unlike `tables/bim-relations.json` (source-fidelity IFC vocabulary), this
//! table records **this document's own** typed references between elements: a
//! face/edge/vertex of one element hosted by another, with an explicit state.
//!
//! ```json
//! [{"source":{"entityId":1},"target":{"entityId":2},
//!   "reference":{"kind":"face","key":"f1"},"state":"in_context","label":null}]
//! ```
//!
//! The vocabulary (families, propagation policy, state machine) is
//! [`cad_domain::relation`]; this module is its persistent table and the one
//! state-write path. There is **no geometric evaluation**: P3 is the typed
//! record, and [`cad_domain::relation::relation_propagation`] answers
//! `rel.no_rule` for reference kinds without a registered rule.
//!
//! # State writes
//!
//! [`InContextRelations::update_state`] is the pure transition check;
//! [`DrawingDatabase::update_relation_state`] wraps it in exactly one revision
//! and a [`crate::ChangeScope`] that dirties [`crate::TableKind::Relations`]
//! only. A refused transition (`rel.state_transition`) changes nothing.

use std::collections::BTreeMap;

use cad_domain::relation::{InContextState, Propagation, RelationReference, REL_STATE_TRANSITION};
use cad_domain::{CadError, CadResult, EntityId, Revision, TransactionId};

use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet, DrawingDatabase};

/// Most rows `tables/relations.json` may hold.
pub const REL_MAX_RELATIONS: usize = 100_000;

/// Most bytes the encoded relation table may occupy.
pub const REL_MAX_BYTES: usize = 32 * 1024 * 1024;

/// Longest reference key, in bytes.
pub const REL_MAX_REFERENCE_KEY_BYTES: usize = 4 * 1024;

/// A stable ordinal for the reference kind, used only as a map-key component.
fn kind_ord(reference: &RelationReference) -> u8 {
    match reference {
        RelationReference::Face { .. } => 0,
        RelationReference::Edge { .. } => 1,
        RelationReference::Vertex { .. } => 2,
    }
}

/// The map key: `(source, target, reference)` with the reference spelled as
/// `(kind ordinal, key)` so it is ordered without requiring `Ord` on the domain
/// enum.
type RelationKey = (EntityId, EntityId, u8, String);

fn key_of(source: EntityId, target: EntityId, reference: &RelationReference) -> RelationKey {
    (
        source,
        target,
        kind_ord(reference),
        reference.key().to_string(),
    )
}

/// One typed in-context edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InContextRelation {
    pub source: EntityId,
    pub target: EntityId,
    pub reference: RelationReference,
    pub state: InContextState,
    pub label: Option<String>,
}

/// The P3 relation table, unique by `(source, target, reference)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InContextRelations {
    rows: BTreeMap<RelationKey, InContextRelation>,
}

impl InContextRelations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one edge. Self edges, over-long keys and duplicates are refused.
    pub fn insert(&mut self, row: InContextRelation) -> CadResult<()> {
        if row.source == row.target {
            return Err(CadError::InvalidInput(format!(
                "rel.self_edge: entity {} cannot reference itself",
                row.source.0
            )));
        }
        if row.reference.key().trim().is_empty() {
            return Err(CadError::InvalidInput(
                "rel.invalid: a reference key must not be empty".into(),
            ));
        }
        if row.reference.key().len() > REL_MAX_REFERENCE_KEY_BYTES {
            return Err(CadError::InvalidInput(format!(
                "rel.invalid: a reference key is {} bytes, over the \
                 {REL_MAX_REFERENCE_KEY_BYTES}-byte budget",
                row.reference.key().len()
            )));
        }
        if self.rows.len() >= REL_MAX_RELATIONS {
            return Err(CadError::InvalidInput(format!(
                "rel.budget_exceeded: a document may hold at most {REL_MAX_RELATIONS} relations"
            )));
        }
        // A fresh row starts conceptually in `InContext`; the requested state
        // must be reachable from there (identical state counts as a no-op
        // transition).
        if !InContextState::InContext.can_transition(row.state) {
            return Err(CadError::InvalidInput(format!(
                "{REL_STATE_TRANSITION}: a new relation cannot start in {:?}",
                row.state.token()
            )));
        }
        let key = key_of(row.source, row.target, &row.reference);
        if self.rows.contains_key(&key) {
            return Err(CadError::InvalidInput(format!(
                "rel.duplicate: {} {} -> {} is already recorded",
                row.reference.token(),
                row.source.0,
                row.target.0
            )));
        }
        self.rows.insert(key, row);
        Ok(())
    }

    pub fn get(
        &self,
        source: EntityId,
        target: EntityId,
        reference: &RelationReference,
    ) -> Option<&InContextRelation> {
        self.rows.get(&key_of(source, target, reference))
    }

    /// Move one edge to `next`, refusing an illegal transition.
    ///
    /// Pure table mutation: the database wrapper adds the revision/scope.
    pub fn update_state(
        &mut self,
        source: EntityId,
        target: EntityId,
        reference: &RelationReference,
        next: InContextState,
    ) -> CadResult<()> {
        let key = key_of(source, target, reference);
        let row = self.rows.get_mut(&key).ok_or_else(|| {
            CadError::InvalidInput(format!(
                "rel.unknown: no {} relation {} -> {}",
                reference.token(),
                source.0,
                target.0
            ))
        })?;
        if !row.state.can_transition(next) {
            return Err(CadError::InvalidInput(format!(
                "{REL_STATE_TRANSITION}: {} cannot move from {:?} to {:?}",
                reference.token(),
                row.state.token(),
                next.token()
            )));
        }
        row.state = next;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &InContextRelation> {
        self.rows.values()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl DrawingDatabase {
    /// The document's self-authored in-context relations.
    pub fn relations(&self) -> &InContextRelations {
        &self.relations
    }

    /// The propagation policy of one relation kind (D1/D2).
    ///
    /// Delegates to [`cad_domain::relation::relation_propagation`] so P3's
    /// policy is reachable from the database layer (and testable) instead of
    /// being dead vocabulary. P2 registers no rule: `voids`/`fills` answer
    /// [`Propagation::NoRule`], ownership kinds never propagate.
    pub fn relation_propagation(
        kind_token: &str,
        host_type_key: &str,
        hosted_type_key: &str,
    ) -> Propagation {
        cad_domain::relation::relation_propagation(kind_token, host_type_key, hosted_type_key)
    }

    /// Move one relation to `next` as one revision.
    ///
    /// The relation table row changed, so the scope marks
    /// [`TableKind::Relations`]; no element geometry, table other than this one,
    /// or header field is touched. An identical state is a no-op (same revision,
    /// empty scope). An illegal transition refuses with `rel.state_transition`
    /// and leaves the table untouched.
    pub fn update_relation_state(
        &mut self,
        source: EntityId,
        target: EntityId,
        reference: &RelationReference,
        next: InContextState,
        reason: &str,
        transaction: TransactionId,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        let current = self
            .relations
            .get(source, target, reference)
            .map(|row| row.state);
        if current == Some(next) {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
                scope: ChangeScope::default(),
            });
        }
        self.relations
            .update_state(source, target, reference, next)?;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Relations);
        let after = Revision(before.0 + 1);
        self.revision = after;
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}

// ---------------------------------------------------------------------------
// Builder + finish gate.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Replace the document's in-context relation table (ADR-0043 P3).
    ///
    /// Uniqueness, self edges and key budgets are enforced by
    /// [`InContextRelations::insert`]; endpoint existence is checked at
    /// [`crate::DrawingDatabaseBuilder::finish`].
    pub fn set_relations(&mut self, relations: InContextRelations) -> CadResult<()> {
        self.database.relations = relations;
        Ok(())
    }
}

/// Validate the relation table against the finished document.
pub(crate) fn validate_in_context_relations(database: &DrawingDatabase) -> CadResult<()> {
    if database.relations.len() > REL_MAX_RELATIONS {
        return Err(CadError::InvalidInput(format!(
            "rel.budget_exceeded: {} rows over the {REL_MAX_RELATIONS}-row budget",
            database.relations.len()
        )));
    }
    for row in database.relations.iter() {
        for (side, id) in [("source", row.source), ("target", row.target)] {
            if !database.entities.contains_key(&id) {
                return Err(CadError::InvalidInput(format!(
                    "rel.endpoint_missing: {side} entity {} does not exist",
                    id.0
                )));
            }
        }
        if row.source == row.target {
            return Err(CadError::InvalidInput(format!(
                "rel.self_edge: entity {} cannot reference itself",
                row.source.0
            )));
        }
        if row.reference.key().trim().is_empty() {
            return Err(CadError::InvalidInput(
                "rel.invalid: a reference key must not be empty".into(),
            ));
        }
    }
    Ok(())
}
