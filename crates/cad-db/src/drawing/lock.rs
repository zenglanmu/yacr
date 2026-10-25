//! Layer-lock enforcement for entity writes.
//!
//! [`LayerAttributes::locked`] is stored (import, and the
//! `SETLAYERPROPERTY` / `cad_set_layer_property` attribute write) but before
//! this module nothing read it, so a locked layer accepted every edit. This
//! module gives it exactly one behaviour: **an entity on a locked layer is
//! read-only**. Creating, modifying (including moving it onto a locked layer)
//! or deleting it is refused with the stable code `layer.locked`, and the
//! refusal names both the entity and the layer.
//!
//! # One rule, two database chokepoints
//!
//! The rule itself is
//! [`DrawingDatabase::ensure_change_allowed_on_locked_layer`]. It is consulted
//! in `cad-db` at exactly two places, and **no command anywhere carries its own
//! lock check**:
//!
//! * `DrawingTransaction::stage` — all five transaction stagers
//!   (`insert_entity`, `update_entity`, `update_entity_display`, `delete_entity`,
//!   `transform_entity`) funnel through it, so a refused write stages
//!   **nothing**: the transaction stays empty, the revision and the history
//!   depth are untouched, and the caller sees the error at the call that asked
//!   for the write.
//! * [`DrawingDatabase::apply_drawing_changes`] — the single validated write
//!   path. Every commit and every history replay goes through it, and it checks
//!   the whole batch in its validation phase, **before** mutating anything. So a
//!   write can never bypass the rule by not using the transaction API, and a
//!   half-applied transaction is impossible by construction (the batch is
//!   all-or-nothing).
//!
//! # Only *effective* changes are refused
//!
//! The rule compares the change with the stored row and allows a write that
//! leaves it identical. That is what keeps the display path working (see
//! below); it also means an idempotent "set this property to the value it
//! already has" is not a lock violation, because it changes nothing.
//!
//! # Display writes stay allowed (deliberate)
//!
//! `DrawingTransaction::update_entity_display` stages the stored entity row
//! **unchanged** and replays the per-entity display attributes beside it — it
//! is a display-representation rebuild, not a drawing edit, and AutoCAD
//! regenerates a locked layer rather than refusing to draw it. The unchanged row
//! passes the "effective change" test above, so display writes keep working on a
//! locked layer (colour/linetype/lineweight/opacity still update and the
//! ChangeSet still carries `STYLE`). The same applies to
//! [`DrawingDatabase::restore_entity_display_state`], which only re-inserts the
//! render attributes/visibility membership a delete pruned.
//!
//! A command that also changes the entity row itself (geometry, layer, …) is
//! refused by its own `update_entity` / `transform_entity` / `delete_entity`
//! call, because that call is an effective change.
//!
//! # Layer-table edits are not entity edits
//!
//! `create_layer` / `rename_layer` / `delete_layer` /
//! [`DrawingDatabase::set_layer_attributes`] are layer-level writes and stay
//! untouched (they do not go through a drawing transaction at all). In
//! particular `set_layer_attributes(.. locked: false ..)` is the **escape
//! hatch**: unlock, then the same edit succeeds. No lock state is cached
//! anywhere, so there is nothing to invalidate.
//!
//! # Frozen / off layers are *not* covered here
//!
//! A frozen (or off) layer is already invisible
//! (`Layer::visible == !(off || frozen)`), so its entities do not enter the
//! scene and are dropped by the visibility-aware pick funnel: they cannot be
//! picked for a new edit. This lane deliberately adds **no second freeze rule**
//! on the write path — a selection captured before the freeze still edits (the
//! pre-existing behaviour, now stated instead of implied). Locking is the
//! *visible but read-only* state and is the only thing enforced here.
//!
//! # History replay: the current lock state wins
//!
//! Undo/redo replays a recorded `DrawingPatch` through
//! [`DrawingDatabase::apply_drawing_changes`], i.e. through this rule. A step
//! that would now create/modify/delete an entity whose layer is locked is
//! therefore **refused** with `layer.locked`; `cad-app` puts the step back on
//! its stack (`cancel_pending_drawing`), so the record is not silently dropped
//! and the document is not mutated. The chosen rule is "a patch is replayed,
//! never re-authorized": a replay is checked against the lock state *at replay
//! time*, not at record time, because silently mutating a layer the user has
//! since locked would be a write the user cannot see coming. (AutoCAD's own
//! undo behaviour is not claimed here; this is this build's documented rule.)
//!
//! # Explicit gap: a locked layer can still be made *current*
//!
//! AutoCAD also refuses to make a locked layer the current layer. The active
//! layer here is `SessionState` (session-only, no database write), so it is
//! outside this choke point and **not** refused yet. Creation on a locked
//! *active* layer is already refused, because the insert itself is checked.

use cad_domain::*;

use super::DrawingDatabase;
use crate::entity::DbEntity;

impl DrawingDatabase {
    /// Whether layer `layer` is locked.
    ///
    /// Missing attribute rows resolve to [`LayerAttributes::default`]
    /// (unlocked), exactly like [`Self::layer_attributes`]; a hand-built or
    /// pre-attribute database is therefore fully writable.
    pub fn layer_locked(&self, layer: LayerId) -> bool {
        self.layer_attributes(layer).locked
    }

    /// Refuse a staged/validated entity write that would create, modify or
    /// delete an entity on a locked layer.
    ///
    /// See the module docs for the rule, the two call sites and why display-only
    /// writes pass. `change == None` means delete. A row identical to the stored
    /// one (the display-attribute path stages exactly that) is not an effective
    /// change and is allowed.
    pub(crate) fn ensure_change_allowed_on_locked_layer(
        &self,
        id: EntityId,
        change: Option<&DbEntity>,
    ) -> CadResult<()> {
        let existing = self.entities.get(&id);
        match change {
            Some(entity) => {
                if existing.is_some_and(|previous| previous == entity) {
                    // No effective change: the display-attribute path stages the
                    // stored row verbatim, and an idempotent write changes
                    // nothing. Neither is a lock violation.
                    return Ok(());
                }
                match existing {
                    Some(previous) => {
                        // The row is on the locked layer today.
                        self.refuse_locked_layer(
                            previous.layer,
                            format!("cannot modify entity {} on", id.0),
                        )?;
                        // …or the write would move it onto a locked layer.
                        if entity.layer != previous.layer {
                            self.refuse_locked_layer(
                                entity.layer,
                                format!("cannot move entity {} onto", id.0),
                            )?;
                        }
                        Ok(())
                    }
                    None => self.refuse_locked_layer(
                        entity.layer,
                        format!("cannot insert entity {} on", id.0),
                    ),
                }
            }
            None => match existing {
                // A delete of a missing entity is not this rule's refusal; the
                // apply path reports that as the existing invariant error.
                None => Ok(()),
                Some(previous) => self.refuse_locked_layer(
                    previous.layer,
                    format!("cannot delete entity {} on", id.0),
                ),
            },
        }
    }

    /// The refusal itself: `layer.locked` when `layer` is locked, naming the
    /// layer row.
    ///
    /// The layer name is read from the layer table (never fabricated); an
    /// entity whose layer row is missing has no locked attributes either, so the
    /// name fallback below is unreachable for a real refusal and only keeps the
    /// message well-formed if that ever changes.
    fn refuse_locked_layer(&self, layer: LayerId, action: String) -> CadResult<()> {
        if !self.layer_locked(layer) {
            return Ok(());
        }
        let name = self
            .layers
            .get(&layer)
            .map(|row| row.name.as_str())
            .unwrap_or("<unknown>");
        Err(CadError::InvalidInput(format!(
            "layer.locked: {action} locked layer '{name}' ({}); unlock the layer first",
            layer.0
        )))
    }
}
