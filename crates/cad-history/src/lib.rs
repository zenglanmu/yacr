//! Undo/redo over the annotation database (spec v2.0 §4.7, §16.3).
//!
//! Undo stores explicit before/after patches; it never re-executes a command,
//! so a non-deterministic recomputation can never produce a different result.
//! Undo and redo publish a [`ChangeSet`] like any other commit.

use cad_db::{Annotation, AnnotationDatabase, ChangeSet};
use cad_domain::*;
use std::collections::VecDeque;

/// A reversible change to one annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationPatch {
    pub id: AnnotationId,
    /// `None` means the annotation did not exist before (a creation).
    pub before: Option<Annotation>,
    /// `None` means the annotation does not exist after (a deletion).
    pub after: Option<Annotation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UndoRecord {
    pub transaction: TransactionId,
    pub label: String,
    pub patches: Vec<AnnotationPatch>,
    /// Records with an equal merge key may be coalesced (e.g. a drag).
    pub merge_key: Option<String>,
}

impl UndoRecord {
    fn approx_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>() + self.label.len();
        for p in &self.patches {
            bytes += std::mem::size_of::<AnnotationPatch>();
            if let Some(a) = &p.before {
                bytes += a.text.len();
            }
            if let Some(a) = &p.after {
                bytes += a.text.len();
            }
        }
        bytes
    }
}

/// Bounded undo/redo stacks.
pub struct History {
    pub memory_budget_bytes: usize,
    undo: VecDeque<UndoRecord>,
    redo: Vec<UndoRecord>,
    used_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new(16 * 1024 * 1024)
    }
}

impl History {
    pub fn new(memory_budget_bytes: usize) -> Self {
        History {
            memory_budget_bytes,
            undo: VecDeque::new(),
            redo: Vec::new(),
            used_bytes: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Record a committed change. Clears the redo stack.
    pub fn record(&mut self, record: UndoRecord) -> CadResult<()> {
        if record.patches.is_empty() {
            return Err(CadError::InvalidInput(
                "an undo record needs at least one patch".to_string(),
            ));
        }
        // Coalesce with the previous record when the merge key matches.
        if let Some(key) = &record.merge_key {
            if let Some(last) = self.undo.back_mut() {
                if last.merge_key.as_deref() == Some(key.as_str()) {
                    for patch in record.patches {
                        if let Some(existing) = last.patches.iter_mut().find(|p| p.id == patch.id) {
                            existing.after = patch.after;
                        } else {
                            last.patches.push(patch);
                        }
                    }
                    self.used_bytes = self.undo.iter().map(|r| r.approx_bytes()).sum();
                    self.redo.clear();
                    return Ok(());
                }
            }
        }
        self.used_bytes += record.approx_bytes();
        self.undo.push_back(record);
        self.redo.clear();
        self.enforce_budget();
        Ok(())
    }

    fn enforce_budget(&mut self) {
        while self.used_bytes > self.memory_budget_bytes && self.undo.len() > 1 {
            if let Some(removed) = self.undo.pop_front() {
                self.used_bytes = self.used_bytes.saturating_sub(removed.approx_bytes());
            }
        }
    }

    /// Undo the most recent record.
    pub fn undo(&mut self, database: &mut AnnotationDatabase) -> CadResult<ChangeSet> {
        let record = self
            .undo
            .pop_back()
            .ok_or_else(|| CadError::InvalidInput("nothing to undo".to_string()))?;
        self.used_bytes = self.used_bytes.saturating_sub(record.approx_bytes());
        let changes: Vec<(AnnotationId, Option<Annotation>)> = record
            .patches
            .iter()
            .map(|p| (p.id, p.before.clone()))
            .collect();
        let change_set = database.apply_annotation_changes(
            &format!("undo: {}", record.label),
            record.transaction,
            changes,
        )?;
        self.redo.push(record);
        Ok(change_set)
    }

    /// Redo the most recently undone record.
    pub fn redo(&mut self, database: &mut AnnotationDatabase) -> CadResult<ChangeSet> {
        let record = self
            .redo
            .pop()
            .ok_or_else(|| CadError::InvalidInput("nothing to redo".to_string()))?;
        let changes: Vec<(AnnotationId, Option<Annotation>)> = record
            .patches
            .iter()
            .map(|p| (p.id, p.after.clone()))
            .collect();
        let change_set = database.apply_annotation_changes(
            &format!("redo: {}", record.label),
            record.transaction,
            changes,
        )?;
        self.used_bytes += record.approx_bytes();
        self.undo.push_back(record);
        self.enforce_budget();
        Ok(change_set)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.used_bytes = 0;
    }
}

/// Optional crash-recovery journal (spec §4.7, §16.3).
pub trait RecoveryJournal {
    fn append(&mut self, record: &UndoRecord) -> CadResult<()>;
    /// Re-apply the journal to a database, returning the change sets produced.
    fn recover(&self, database: &mut AnnotationDatabase) -> CadResult<Vec<ChangeSet>>;
}

/// In-memory journal used by tests and the CLI.
#[derive(Default)]
pub struct MemoryJournal {
    records: Vec<UndoRecord>,
}

impl MemoryJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl RecoveryJournal for MemoryJournal {
    fn append(&mut self, record: &UndoRecord) -> CadResult<()> {
        self.records.push(record.clone());
        Ok(())
    }

    fn recover(&self, database: &mut AnnotationDatabase) -> CadResult<Vec<ChangeSet>> {
        let mut out = Vec::new();
        for record in &self.records {
            let changes: Vec<(AnnotationId, Option<Annotation>)> = record
                .patches
                .iter()
                .map(|p| (p.id, p.after.clone()))
                .collect();
            out.push(database.apply_annotation_changes(
                &format!("recover: {}", record.label),
                record.transaction,
                changes,
            )?);
        }
        Ok(out)
    }
}

/// Build a patch from the difference between two annotation states.
pub fn patch(
    id: AnnotationId,
    before: Option<Annotation>,
    after: Option<Annotation>,
) -> AnnotationPatch {
    AnnotationPatch { id, before, after }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{AnnotationGeometry, AnnotationStyle};

    fn ann(id: u128, text: &str) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            text: text.to_string(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    fn database_with() -> AnnotationDatabase {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        db.apply_annotation_changes(
            "seed",
            TransactionId(0),
            vec![(AnnotationId(1), Some(ann(1, "a")))],
        )
        .unwrap();
        db
    }

    #[test]
    fn create_then_undo_removes_and_redo_restores() {
        let mut db = database_with();
        let mut history = History::default();
        // The creation of annotation 5 was already committed elsewhere:
        db.apply_annotation_changes(
            "create",
            TransactionId(1),
            vec![(AnnotationId(5), Some(ann(5, "b")))],
        )
        .unwrap();
        history
            .record(UndoRecord {
                transaction: TransactionId(1),
                label: "create b".into(),
                patches: vec![patch(AnnotationId(5), None, Some(ann(5, "b")))],
                merge_key: None,
            })
            .unwrap();

        history.undo(&mut db).unwrap();
        assert!(db.get(AnnotationId(5)).is_none());
        history.redo(&mut db).unwrap();
        assert!(db.get(AnnotationId(5)).is_some());
    }

    #[test]
    fn update_restores_previous_value() {
        let mut db = database_with();
        let mut history = History::default();
        db.apply_annotation_changes(
            "update",
            TransactionId(2),
            vec![(AnnotationId(1), Some(ann(1, "edited")))],
        )
        .unwrap();
        history
            .record(UndoRecord {
                transaction: TransactionId(2),
                label: "edit a".into(),
                patches: vec![patch(
                    AnnotationId(1),
                    Some(ann(1, "a")),
                    Some(ann(1, "edited")),
                )],
                merge_key: None,
            })
            .unwrap();
        history.undo(&mut db).unwrap();
        assert_eq!(db.get(AnnotationId(1)).unwrap().text, "a");
    }

    #[test]
    fn merge_key_coalesces_records() {
        let mut history = History::default();
        let r = |t: &str| UndoRecord {
            transaction: TransactionId(1),
            label: "drag".into(),
            patches: vec![patch(AnnotationId(1), Some(ann(1, "a")), Some(ann(1, t)))],
            merge_key: Some("drag-1".into()),
        };
        history.record(r("mid")).unwrap();
        history.record(r("end")).unwrap();
        assert_eq!(history.undo_depth(), 1);
    }

    #[test]
    fn empty_record_is_rejected() {
        let mut history = History::default();
        let r = UndoRecord {
            transaction: TransactionId(1),
            label: "noop".into(),
            patches: vec![],
            merge_key: None,
        };
        assert!(history.record(r).is_err());
    }

    #[test]
    fn budget_evicts_oldest() {
        let mut history = History::new(1);
        for i in 0..5 {
            history
                .record(UndoRecord {
                    transaction: TransactionId(1),
                    label: format!("step {i}"),
                    patches: vec![patch(AnnotationId(i), None, Some(ann(i, "x")))],
                    merge_key: None,
                })
                .unwrap();
        }
        assert_eq!(history.undo_depth(), 1);
    }

    #[test]
    fn journal_recovers_state() {
        let mut journal = MemoryJournal::new();
        journal
            .append(&UndoRecord {
                transaction: TransactionId(9),
                label: "create".into(),
                patches: vec![patch(AnnotationId(7), None, Some(ann(7, "recovered")))],
                merge_key: None,
            })
            .unwrap();
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        let sets = journal.recover(&mut db).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(db.get(AnnotationId(7)).unwrap().text, "recovered");
    }
}
