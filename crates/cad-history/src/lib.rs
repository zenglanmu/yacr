//! Undo/redo over the drawing database (spec v2.0 §4.7, §16.3).
//!
//! Undo stores explicit before/after patches; it never re-executes a command,
//! so a non-deterministic recomputation can never produce a different result.
//! Undo and redo publish a [`ChangeSet`] like any other commit.

use cad_db::{DbEntity, EntityDisplayState};
use cad_domain::*;
use std::collections::{HashSet, VecDeque};

/// A reversible change to one drawing entity (spec F-EDIT).
///
/// It keeps the full [`DbEntity`] before and after so an undo or redo restores
/// the geometry *and* the identity/layer/space/draw_order exactly, without
/// re-executing the original command. `None` on either side means "did not
/// exist" (a creation or a deletion). A record may apply several of these in one
/// commit (for example a MOVE over a multi-selection).
///
/// A delete also prunes per-entity display state (render attributes and
/// dynamic-visibility membership) that the [`DbEntity`] does not carry, so the
/// patch keeps an optional [`EntityDisplayState`] for each side; the undo/redo
/// applier restores it when that side is re-inserted.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingPatch {
    pub id: EntityId,
    /// `None` means the entity did not exist before (a creation).
    pub before: Option<DbEntity>,
    /// `None` means the entity does not exist after (a deletion).
    pub after: Option<DbEntity>,
    /// Display state to restore when the `before` side is (re-)inserted.
    pub before_state: Option<EntityDisplayState>,
    /// Display state to restore when the `after` side is (re-)inserted.
    pub after_state: Option<EntityDisplayState>,
}

/// Build a drawing patch from the difference between two entity states.
///
/// Display state is not captured; use [`drawing_patch_deleted`] for a deletion
/// that removed render attributes / dynamic-visibility membership.
pub fn drawing_patch(
    id: EntityId,
    before: Option<DbEntity>,
    after: Option<DbEntity>,
) -> DrawingPatch {
    DrawingPatch {
        id,
        before,
        after,
        before_state: None,
        after_state: None,
    }
}

/// A deletion patch that also captures the pre-delete display state, so an undo
/// restores the entity's render attributes and dynamic-visibility membership.
pub fn drawing_patch_deleted(
    id: EntityId,
    before: DbEntity,
    before_state: EntityDisplayState,
) -> DrawingPatch {
    DrawingPatch {
        id,
        before: Some(before),
        after: None,
        before_state: Some(before_state),
        after_state: None,
    }
}

/// A reversible batch of drawing-entity changes.
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

/// Undo/redo stacks with an approximate budget on retained undo entries.
///
/// The newest undo entry is retained even if it exceeds the budget. The byte
/// estimate is not a total heap measurement: nested geometry allocations,
/// spare collection capacity, redo entries and a pending drawing step are not
/// fully accounted for.
pub struct History {
    pub memory_budget_bytes: usize,
    undo: VecDeque<DrawingUndoRecord>,
    redo: Vec<DrawingUndoRecord>,
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

    /// Estimated bytes of retained undo entries.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Record a committed drawing-entity change. Clears the redo stack.
    ///
    /// Coalescing keeps the earliest before state and the latest after state.
    /// Patches that return to their original state are removed; if all patches
    /// cancel out, the merged entry is removed without touching earlier entries.
    /// Even a fully canceled merge clears redo, since new edits were committed.
    /// An empty patch list is rejected, so a command can never claim success
    /// while recording nothing.
    pub fn record_drawing(&mut self, record: DrawingUndoRecord) -> CadResult<()> {
        self.ensure_no_pending_drawing()?;
        record.validate()?;
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
                    self.recompute_used_bytes();
                    self.redo.clear();
                    self.enforce_budget();
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

    fn recompute_used_bytes(&mut self) {
        self.used_bytes = self.undo.iter().map(|r| r.approx_bytes()).sum();
    }

    fn enforce_budget(&mut self) {
        while self.used_bytes > self.memory_budget_bytes && self.undo.len() > 1 {
            let Some(removed) = self.undo.pop_front() else {
                break;
            };
            self.used_bytes = self.used_bytes.saturating_sub(removed.approx_bytes());
        }
    }

    /// Discard all history, including any pending drawing step.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.used_bytes = 0;
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
        let record = self
            .undo
            .pop_back()
            .ok_or_else(|| CadError::InvalidInput("nothing to undo".into()))?;
        self.used_bytes = self.used_bytes.saturating_sub(record.approx_bytes());
        self.pending = Some(PendingDrawing {
            record: record.clone(),
            direction: UndoDirection::Undo,
        });
        Ok(record)
    }

    /// Redo the most recently undone drawing record, returning its patches.
    pub fn begin_drawing_redo(&mut self) -> CadResult<DrawingUndoRecord> {
        self.ensure_no_pending_drawing()?;
        let record = self
            .redo
            .pop()
            .ok_or_else(|| CadError::InvalidInput("nothing to redo".into()))?;
        self.pending = Some(PendingDrawing {
            record: record.clone(),
            direction: UndoDirection::Redo,
        });
        Ok(record)
    }

    /// Commit the pending drawing undo/redo after the database accepted it.
    pub fn finish_drawing(&mut self) -> CadResult<()> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| CadError::Invariant("no pending drawing undo".into()))?;
        match pending.direction {
            UndoDirection::Undo => self.redo.push(pending.record),
            UndoDirection::Redo => {
                self.used_bytes += pending.record.approx_bytes();
                self.undo.push_back(pending.record);
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
                self.used_bytes += pending.record.approx_bytes();
                self.undo.push_back(pending.record);
            }
            UndoDirection::Redo => {
                self.redo.push(pending.record);
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

enum UndoDirection {
    Undo,
    Redo,
}

struct PendingDrawing {
    record: DrawingUndoRecord,
    direction: UndoDirection,
}

#[cfg(test)]
mod tests;
