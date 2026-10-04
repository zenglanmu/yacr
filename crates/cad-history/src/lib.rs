//! Undo/redo over the annotation database (spec v2.0 §4.7, §16.3).
//!
//! Undo stores explicit before/after patches; it never re-executes a command,
//! so a non-deterministic recomputation can never produce a different result.
//! Undo and redo publish a [`ChangeSet`] like any other commit.

use cad_db::{Annotation, AnnotationDatabase, ChangeSet, DbEntity};
use cad_domain::*;
use std::collections::{HashSet, VecDeque};

/// A reversible change to one annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationPatch {
    pub id: AnnotationId,
    /// `None` means the annotation did not exist before (a creation).
    pub before: Option<Annotation>,
    /// `None` means the annotation does not exist after (a deletion).
    pub after: Option<Annotation>,
}

/// A reversible change to one drawing entity (spec F-EDIT).
///
/// The drawing twin of [`AnnotationPatch`]: it keeps the full [`DbEntity`]
/// before and after so an undo or redo restores the geometry *and* the
/// identity/layer/space/draw_order exactly, without re-executing the original
/// command. `None` on either side means "did not exist" (a creation or a
/// deletion). A drawing undo record may apply several of these in one commit
/// (for example a MOVE over a multi-selection).
///
/// This type lives in `cad-history` (which depends on `cad-db`) so the drawing
/// database can consume a patch list without `cad-db` depending on history; the
/// annotation path keeps its `AnnotationDatabase`-specific undo/redo methods
/// because a single [`History`] may only be bound to one database. Hosts apply
/// a drawing record with [`AnnotationDatabase::apply_drawing_changes`] (or the
/// equivalent drawing API) directly.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingPatch {
    pub id: EntityId,
    /// `None` means the entity did not exist before (a creation).
    pub before: Option<DbEntity>,
    /// `None` means the entity does not exist after (a deletion).
    pub after: Option<DbEntity>,
}

/// Build a drawing patch from the difference between two entity states.
pub fn drawing_patch(
    id: EntityId,
    before: Option<DbEntity>,
    after: Option<DbEntity>,
) -> DrawingPatch {
    DrawingPatch { id, before, after }
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
    fn validate(&self) -> CadResult<()> {
        if self.merge_key.as_deref() == Some(DRAWING_MARKER) {
            return Err(CadError::InvalidInput(
                "the drawing history marker is reserved".into(),
            ));
        }
        if self.patches.is_empty() {
            return Err(CadError::InvalidInput(
                "an undo record needs at least one patch".into(),
            ));
        }
        let mut ids = HashSet::new();
        for patch in &self.patches {
            if !ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "an undo record cannot contain duplicate annotation ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "an annotation patch needs a before or after state".into(),
                ));
            }
            for annotation in patch.before.iter().chain(patch.after.iter()) {
                if annotation.id != patch.id {
                    return Err(CadError::InvalidInput(
                        "annotation patch key does not match snapshot id".into(),
                    ));
                }
            }
        }
        Ok(())
    }

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

/// Undo/redo stacks with an approximate budget on retained undo entries.
///
/// The newest undo entry is retained even if it exceeds the budget. The byte
/// estimate is not a total heap measurement: nested geometry allocations,
/// spare collection capacity, redo entries and a pending drawing step are not
/// fully accounted for.
///
/// Annotation and drawing records share one ordered history: `undo`/`redo`
/// carry the user-visible ordering (annotation payloads plus placeholder
/// markers for drawing steps), while `drawing_undo`/`drawing_redo` and the
/// parallel `redo_markers` hold the drawing payloads. A single document thus has
/// one undo stream across both kinds of edit (spec F-EDIT).
pub struct History {
    pub memory_budget_bytes: usize,
    undo: VecDeque<UndoRecord>,
    redo: Vec<UndoRecord>,
    drawing_undo: VecDeque<DrawingUndoRecord>,
    drawing_redo: Vec<DrawingUndoRecord>,
    /// One marker per entry in `redo`, in the same order.
    redo_markers: Vec<String>,
    pending: Option<PendingDrawing>,
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
            drawing_undo: VecDeque::new(),
            drawing_redo: Vec::new(),
            redo_markers: Vec::new(),
            pending: None,
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

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Estimated bytes of retained undo entries, including drawing markers.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Record a committed change. Clears the redo stack.
    ///
    /// Coalescing keeps the earliest before state and the latest after state.
    /// Patches that return to their original state are removed; if all patches
    /// cancel out, the merged entry is removed without touching earlier entries.
    /// Even a fully canceled merge clears redo, since new edits were committed.
    /// Malformed records are rejected before changing either stack.
    pub fn record(&mut self, record: UndoRecord) -> CadResult<()> {
        self.ensure_no_pending_drawing()?;
        record.validate()?;
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
                    last.patches.retain(|patch| patch.before != patch.after);
                    if last.patches.is_empty() {
                        self.undo.pop_back();
                    }
                    self.used_bytes = self.undo.iter().map(|r| r.approx_bytes()).sum::<usize>()
                        + self
                            .drawing_undo
                            .iter()
                            .map(|r| r.approx_bytes())
                            .sum::<usize>();
                    self.redo.clear();
                    self.redo_markers.clear();
                    self.drawing_redo.clear();
                    self.enforce_budget();
                    return Ok(());
                }
            }
        }
        self.used_bytes += record.approx_bytes();
        self.undo.push_back(record);
        self.redo.clear();
        self.redo_markers.clear();
        self.drawing_redo.clear();
        self.enforce_budget();
        Ok(())
    }

    fn enforce_budget(&mut self) {
        while self.used_bytes > self.memory_budget_bytes && self.undo.len() > 1 {
            let Some(removed) = self.undo.pop_front() else {
                break;
            };
            self.used_bytes = self.used_bytes.saturating_sub(removed.approx_bytes());
            // Keep the parallel drawing payload stack in step: a dropped
            // drawing marker must drop the matching drawing record and its
            // accounted bytes, or the two stacks desynchronise.
            if removed.merge_key.as_deref() == Some(DRAWING_MARKER) {
                if let Some(dropped) = self.drawing_undo.pop_front() {
                    self.used_bytes = self.used_bytes.saturating_sub(dropped.approx_bytes());
                }
            }
        }
    }

    /// Undo the most recent record.
    ///
    /// Fails when the most recent step is a drawing edit: drawing records must
    /// be applied to the *drawing* database by the caller. This keeps a single
    /// ordering without ever routing an entity patch into the annotation store.
    pub fn undo(&mut self, database: &mut AnnotationDatabase) -> CadResult<ChangeSet> {
        self.ensure_no_pending_drawing()?;
        let top_is_drawing = matches!(
            self.undo.back().and_then(|r| r.merge_key.as_deref()),
            Some(DRAWING_MARKER)
        );
        if top_is_drawing {
            return Err(CadError::InvalidInput(
                "the most recent undo step is a drawing edit; apply it to the drawing database"
                    .into(),
            ));
        }
        let record = self
            .undo
            .back()
            .ok_or_else(|| CadError::InvalidInput("nothing to undo".to_string()))?;
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
        // Only move the record after the atomic database write succeeds.
        let record = self.undo.pop_back().expect("record was checked above");
        self.used_bytes = self.used_bytes.saturating_sub(record.approx_bytes());
        self.redo_markers.push(String::new());
        self.redo.push(record);
        Ok(change_set)
    }

    /// Redo the most recently undone record.
    pub fn redo(&mut self, database: &mut AnnotationDatabase) -> CadResult<ChangeSet> {
        self.ensure_no_pending_drawing()?;
        if self.next_redo_is_drawing() {
            return Err(CadError::InvalidInput(
                "the next redo step is a drawing edit; apply it to the drawing database".into(),
            ));
        }
        let record = self
            .redo
            .last()
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
        let record = self.redo.pop().expect("record was checked above");
        self.redo_markers.pop();
        self.used_bytes += record.approx_bytes();
        self.undo.push_back(record);
        self.enforce_budget();
        Ok(change_set)
    }

    /// Discard all history, including any pending drawing step.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.drawing_undo.clear();
        self.drawing_redo.clear();
        self.redo_markers.clear();
        self.pending = None;
        self.used_bytes = 0;
    }

    /// Record a committed drawing-entity change. Clears the redo stack.
    ///
    /// Drawing and annotation records share the same stacks and the same
    /// per-document [`History`], so a single undo stream interleaves both in the
    /// exact order the user performed them. An empty patch list is rejected, so
    /// a command can never claim success while recording nothing.
    pub fn record_drawing(&mut self, record: DrawingUndoRecord) -> CadResult<()> {
        self.ensure_no_pending_drawing()?;
        record.validate()?;
        if let Some(key) = &record.merge_key {
            if let Some(last) = self.undo.back_mut() {
                if last.merge_key.as_deref() == Some(key.as_str()) {
                    return Err(CadError::Unsupported(
                        "drawing records cannot be coalesced with annotation records".into(),
                    ));
                }
            }
        }
        let transaction = record.transaction;
        let label = record.label.clone();
        let marker = drawing_marker(transaction, label);
        // Compute the byte cost of the drawing payload before moving the
        // patches into the drawing stack.
        self.used_bytes += record.approx_bytes() + marker.approx_bytes();
        self.drawing_undo.push_back(record);
        self.drawing_redo.clear();
        self.undo.push_back(marker);
        self.redo.clear();
        self.redo_markers.clear();
        self.enforce_budget();
        Ok(())
    }

    /// Whether the most recent record is a drawing record.
    pub fn pop_drawing_for_undo(&self) -> bool {
        matches!(
            self.undo.back().and_then(|r| r.merge_key.as_deref()),
            Some(DRAWING_MARKER)
        )
    }

    /// Undo the most recent drawing record, returning its patches.
    ///
    /// The caller applies the `before` entities through the drawing database's
    /// own validated write path; the record is only moved to the redo stack
    /// after the caller reports success via [`History::finish_drawing`].
    /// This method pops the record and holds it until finished, so a failed
    /// apply cannot silently lose or duplicate the step.
    /// Other history writes are rejected until this step is finished or canceled.
    pub fn begin_drawing_undo(&mut self) -> CadResult<DrawingUndoRecord> {
        self.ensure_no_pending_drawing()?;
        let top_is_drawing = matches!(
            self.undo.back().and_then(|r| r.merge_key.as_deref()),
            Some(DRAWING_MARKER)
        );
        if !top_is_drawing {
            return Err(CadError::InvalidInput(
                "the most recent undo step is not a drawing edit".into(),
            ));
        }
        let record = self
            .drawing_undo
            .pop_back()
            .ok_or_else(|| CadError::Invariant("drawing undo stack desynchronised".into()))?;
        let stamp = self.undo.pop_back().expect("checked above");
        self.used_bytes = self
            .used_bytes
            .saturating_sub(record.approx_bytes() + stamp.approx_bytes());
        self.pending = Some(PendingDrawing {
            record,
            stamp,
            direction: UndoDirection::Undo,
        });
        Ok(self.pending.as_ref().expect("just set").record.clone())
    }

    /// Whether the most recent redo step is a drawing record.
    pub fn next_redo_is_drawing(&self) -> bool {
        matches!(self.drawing_redo_last_marker(), Some(DRAWING_MARKER))
    }

    fn drawing_redo_last_marker(&self) -> Option<&str> {
        self.redo_markers.last().map(|s| s.as_str())
    }

    /// Redo the most recent drawing delete/update, returning its patches.
    pub fn begin_drawing_redo(&mut self) -> CadResult<DrawingUndoRecord> {
        self.ensure_no_pending_drawing()?;
        if !self.next_redo_is_drawing() {
            return Err(CadError::InvalidInput(
                "the next redo step is not a drawing edit".into(),
            ));
        }
        if self.redo.is_empty() || self.drawing_redo.is_empty() {
            return Err(CadError::Invariant(
                "drawing redo stack desynchronised".into(),
            ));
        }
        let record = self
            .drawing_redo
            .pop()
            .ok_or_else(|| CadError::Invariant("drawing redo stack desynchronised".into()))?;
        self.redo_markers.pop();
        let stamp = self.redo.pop().expect("checked above");
        self.pending = Some(PendingDrawing {
            record,
            stamp,
            direction: UndoDirection::Redo,
        });
        Ok(self.pending.as_ref().expect("just set").record.clone())
    }

    /// Commit the pending drawing undo/redo after the database accepted it.
    pub fn finish_drawing(&mut self) -> CadResult<()> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| CadError::Invariant("no pending drawing undo".into()))?;
        match pending.direction {
            UndoDirection::Undo => {
                self.redo_markers.push(DRAWING_MARKER.to_string());
                self.drawing_redo.push(pending.record);
                // The annotation redo stack mirrors the ordering with a marker.
                self.redo.push(pending.stamp);
            }
            UndoDirection::Redo => {
                self.used_bytes += pending.record.approx_bytes() + pending.stamp.approx_bytes();
                self.drawing_undo.push_back(pending.record);
                self.undo.push_back(pending.stamp);
                self.enforce_budget();
            }
        }
        Ok(())
    }

    /// Put a pending drawing undo/redo back where it came from.
    ///
    /// Called when the database rejected the patch after
    /// [`History::begin_drawing_undo`]/[`History::begin_drawing_redo`] popped it:
    /// the step must not be lost. The byte accounting mirrors `finish_drawing`.
    pub fn cancel_pending_drawing(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        match pending.direction {
            UndoDirection::Undo => {
                self.used_bytes += pending.record.approx_bytes() + pending.stamp.approx_bytes();
                self.undo.push_back(pending.stamp);
                self.drawing_undo.push_back(pending.record);
            }
            UndoDirection::Redo => {
                self.redo_markers.push(DRAWING_MARKER.to_string());
                self.redo.push(pending.stamp);
                self.drawing_redo.push(pending.record);
            }
        }
    }

    fn ensure_no_pending_drawing(&self) -> CadResult<()> {
        if self.pending.is_some() {
            return Err(CadError::InvalidInput(
                "finish or cancel the pending drawing history step first".into(),
            ));
        }
        Ok(())
    }
}

/// The marker `merge_key` that identifies a drawing record on the shared stack.
pub const DRAWING_MARKER: &str = "__drawing__";

/// The placeholder annotation record that keeps the shared undo/redo ordering
/// honest. It never reaches `History::undo`/`redo` because those check the
/// marker and route drawing records through the drawing API.
fn drawing_marker(transaction: TransactionId, label: String) -> UndoRecord {
    UndoRecord {
        transaction,
        label,
        patches: vec![AnnotationPatch {
            id: AnnotationId(transaction.0),
            before: None,
            after: None,
        }],
        merge_key: Some(DRAWING_MARKER.to_string()),
    }
}

enum UndoDirection {
    Undo,
    Redo,
}

struct PendingDrawing {
    record: DrawingUndoRecord,
    stamp: UndoRecord,
    direction: UndoDirection,
}

/// A reversible batch of drawing-entity changes, the drawing twin of
/// [`UndoRecord`].
///
/// The patches carry the full before/after [`DbEntity`]; `transaction` is the
/// id to reuse when the record is undone/redone so the resulting [`ChangeSet`]
/// is traceable to the same logical edit. An empty `patches` is a caller bug
/// and [`History`] rejects it, so a command can never record a silent no-op.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingUndoRecord {
    pub transaction: TransactionId,
    pub label: String,
    pub patches: Vec<DrawingPatch>,
    /// Records with an equal merge key may be coalesced (e.g. a drag). The
    /// drawing commands in `cad-app` deliberately pass `None` so each user
    /// action is exactly one undo step.
    pub merge_key: Option<String>,
}

impl DrawingUndoRecord {
    fn validate(&self) -> CadResult<()> {
        if self.patches.is_empty() {
            return Err(CadError::InvalidInput(
                "a drawing undo record needs at least one patch".into(),
            ));
        }
        let mut ids = HashSet::new();
        for patch in &self.patches {
            if !ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate entity ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a drawing patch needs a before or after state".into(),
                ));
            }
            for entity in patch.before.iter().chain(patch.after.iter()) {
                if entity.id != patch.id || entity.object.id.0 != patch.id.0 {
                    return Err(CadError::InvalidInput(
                        "drawing patch key does not match snapshot identity".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn approx_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>() + self.label.len();
        for p in &self.patches {
            bytes += std::mem::size_of::<DrawingPatch>();
            if let Some(e) = &p.before {
                bytes += std::mem::size_of::<cad_db::DbEntity>() + e.object.type_key.len();
            }
            if let Some(e) = &p.after {
                bytes += std::mem::size_of::<cad_db::DbEntity>() + e.object.type_key.len();
            }
        }
        bytes
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
mod tests;
