//! The mutable annotation database and its staged transactions.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use crate::annotation::{validate_annotation, Annotation};
use crate::change::{ChangeMask, ChangeSet, ObjectChange};

/// The mutable annotation database; all writes go through transactions.
#[derive(Debug, Clone)]
pub struct AnnotationDatabase {
    id: DatabaseId,
    revision: Revision,
    annotations: BTreeMap<AnnotationId, Annotation>,
    saved_revision: Revision,
}

impl AnnotationDatabase {
    pub fn new(id: DatabaseId) -> Self {
        Self {
            id,
            revision: Revision(0),
            annotations: BTreeMap::new(),
            saved_revision: Revision(0),
        }
    }

    pub fn id(&self) -> DatabaseId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    pub fn annotations(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.values()
    }

    pub fn get(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations.get(&id)
    }

    /// Called only after durable export succeeds, with the exported revision.
    pub fn mark_exported(&mut self, revision: Revision) -> CadResult<()> {
        if revision.0 > self.revision.0 {
            return Err(CadError::Invariant(
                "cannot mark a revision that has not happened".to_string(),
            ));
        }
        self.saved_revision = revision;
        Ok(())
    }

    pub fn begin(
        &mut self,
        reason: &str,
        id: TransactionId,
    ) -> CadResult<AnnotationTransaction<'_>> {
        AnnotationTransaction::new(self, reason, id)
    }

    /// Single validated write path used by both transactions and history.
    ///
    /// `changes` maps an annotation id to its new value (`None` = delete).
    /// Applies atomically: on any validation failure nothing is modified.
    pub fn apply_annotation_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: Vec<(AnnotationId, Option<Annotation>)>,
    ) -> CadResult<ChangeSet> {
        // Validate first; do not touch the map until every change is legal.
        // Validation covers id/key agreement, geometry finiteness/validity and
        // style bounds so a poisoned annotation can never enter the store
        // (audit B12).
        let mut seen = BTreeSet::new();
        for (id, change) in &changes {
            if !seen.insert(*id) {
                return Err(CadError::Invariant(format!(
                    "transaction {transaction:?} modifies annotation {id:?} twice"
                )));
            }
            match change {
                Some(annotation) => {
                    if annotation.id != *id {
                        return Err(CadError::Invariant(format!(
                            "change key {id:?} does not match annotation id {:?}",
                            annotation.id
                        )));
                    }
                    validate_annotation(annotation)?;
                }
                None => {
                    if !self.annotations.contains_key(id) {
                        return Err(CadError::Invariant(format!(
                            "annotation {id:?} does not exist"
                        )));
                    }
                }
            }
        }

        let before = self.revision;
        if changes.is_empty() {
            // Nothing staged: do not advance the revision or claim a change.
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }
        let mut ordered = Vec::with_capacity(changes.len());
        for (id, change) in changes {
            match change {
                Some(annotation) => {
                    let mask = match self.annotations.get(&id) {
                        Some(previous) => ChangeMask::for_annotation_update(previous, &annotation),
                        None => ChangeMask::GEOMETRY.union(ChangeMask::STYLE),
                    };
                    if self.annotations.insert(id, annotation).is_some() {
                        ordered.push(ObjectChange::Update(ObjectId(id.0), mask));
                    } else {
                        ordered.push(ObjectChange::Insert(ObjectId(id.0)));
                    }
                }
                None => {
                    self.annotations.remove(&id);
                    ordered.push(ObjectChange::Delete(ObjectId(id.0)));
                }
            }
        }
        let after = Revision(before.0 + 1);
        self.revision = after;
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: ordered,
        })
    }
}

/// A staged, all-or-nothing annotation transaction.
pub struct AnnotationTransaction<'a> {
    database: &'a mut AnnotationDatabase,
    reason: String,
    transaction: TransactionId,
    pub(crate) staged: BTreeMap<AnnotationId, Option<Annotation>>,
}

impl<'a> AnnotationTransaction<'a> {
    fn new(
        database: &'a mut AnnotationDatabase,
        reason: &str,
        transaction: TransactionId,
    ) -> CadResult<Self> {
        if reason.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "transaction reason is required".to_string(),
            ));
        }
        Ok(AnnotationTransaction {
            database,
            reason: reason.to_string(),
            transaction,
            staged: BTreeMap::new(),
        })
    }

    /// Number of staged changes (used by tests and the UI status).
    pub fn staged_len(&self) -> usize {
        self.staged.len()
    }

    pub fn insert_annotation(&mut self, annotation: Annotation) -> CadResult<AnnotationId> {
        let id = annotation.id;
        if self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} already exists"
            )));
        }
        self.staged.insert(id, Some(annotation));
        Ok(id)
    }

    pub fn update_annotation(&mut self, annotation: Annotation) -> CadResult<()> {
        let id = annotation.id;
        if !self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} does not exist"
            )));
        }
        self.staged.insert(id, Some(annotation));
        Ok(())
    }

    pub fn delete_annotation(&mut self, id: AnnotationId) -> CadResult<()> {
        if !self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} does not exist"
            )));
        }
        self.staged.insert(id, None);
        Ok(())
    }

    pub fn commit(mut self) -> CadResult<ChangeSet> {
        let staged = std::mem::take(&mut self.staged);
        let changes: Vec<(AnnotationId, Option<Annotation>)> = staged.into_iter().collect();
        self.database
            .apply_annotation_changes(&self.reason, self.transaction, changes)
    }

    /// Uncommitted staged changes are discarded on drop.
    pub fn rollback(self) {
        // Dropping `staged` is the rollback; the database was never touched.
    }
}
