//! `GRIPSTRETCH`: apply **one** grip drag to **one** entity.
//!
//! This is the command half of the grip model in [`crate::grips`]. The model
//! decides which handles an entity exposes, which handle a pick hits and what
//! geometry a drag of it produces ([`crate::grips::apply_grip`]); this module
//! re-reads the addressed entity from the authoritative drawing, hands its
//! **current** geometry to that drag rule and, only on `Ok`, stages exactly one
//! `update_entity` in one transaction / one undo step.
//!
//! ## The payload is a gesture's data, not a gesture
//!
//! [`crate::CommandPayload::GripStretch`] carries `{ id, to }`: the handle id
//! resolved at pick time plus the world point the pointer resolved to. There is
//! **no** gesture state machine, **no** renderer marker, **no** host arming and
//! **no** pointer routing in this build — nothing produces this payload yet. A
//! later lane owns those. The command exists now so the write path, its refusals
//! and its one-undo-entry guarantee are pinned before a UI can depend on them.
//!
//! ## Nothing on refusal
//!
//! Every refusal happens **before** `begin_drawing_transaction`, so a refused
//! drag writes nothing: no entity update, no revision bump, no `ChangeSet` and —
//! the reason `grip.zero_displacement` is an error instead of an empty `Ok` — no
//! empty undo entry.
//!
//! * `grip.target_missing` — the id's entity does not resolve in this document;
//! * `property.not_addressable` — the addressed entity is **not** a model-space
//!   entity: a block-definition member (reachable only through an `INSERT`
//!   instance path) or a paper-space entity. This **reuses** the property-write
//!   reachability reason rather than inventing a second addressability model. A
//!   [`crate::grips::GripId`] carries no instance path and no sub-element field,
//!   so those two forms cannot even be expressed in the payload; the reachable
//!   case behind them ("this id does not name a directly editable model-space
//!   entity") is exactly what this check refuses;
//! * `grip.non_finite_target` — `to` has a `NaN`/infinite coordinate, refused
//!   before the drawing is read at all (the model would refuse it too, with the
//!   same code);
//! * `grip.stale_handle`, `grip.zero_displacement`, `grip.unsupported_kind` —
//!   surfaced verbatim from [`crate::grips::apply_grip`], which re-validates the
//!   handle against the **current** geometry (a stale id is never trusted);
//! * a missing `GripStretch` payload — the bare `GRIPSTRETCH` selector carries no
//!   handle, so it is refused by name here (the typed command line never
//!   fabricates a drag).
//!
//! ## Not `STRETCH`
//!
//! This never routes through [`CommandId::StretchEntities`] / the crossing-window
//! stretch: that command moves every vertex inside a rectangle (or whole
//! entities) and has its own refusal set. A grip drag moves exactly one handle,
//! and the model's `apply_grip` output is what gets stored.

use super::*;
use crate::grips::{apply_grip, finite_target, REASON_TARGET_MISSING};

impl Application {
    /// Apply one grip drag as one `update_entity` in one transaction.
    ///
    /// Refusals are listed in the module doc; each one leaves the drawing
    /// untouched (no revision bump, no `ChangeSet`, no undo entry) because none of
    /// them opens a transaction. On success the stored geometry is the model's
    /// drag output verbatim — for a `LINE` the untouched endpoint keeps its exact
    /// `f64` bits, because the rule returns the two stored points rather than
    /// recomputing a pair.
    pub(crate) fn grip_stretch(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::GripStretch(grip) = &command.payload else {
            return Err(CadError::InvalidInput(
                "GRIPSTRETCH needs a GripStretch { id, to } payload; the bare selector \
                 carries no handle to drag"
                    .into(),
            ));
        };
        // Refuse a non-finite target before the drawing is even read: there is
        // nothing to validate it against and no transaction may be built.
        finite_target(grip.to)?;

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        // Re-read the target: the id is a caller-held reference, so a deleted or
        // replaced entity is a named refusal, never a staged write.
        let before = drawing.entity(grip.id.entity).cloned().ok_or_else(|| {
            CadError::InvalidInput(format!(
                "{REASON_TARGET_MISSING}: GRIPSTRETCH target {} does not exist",
                grip.id.entity.0
            ))
        })?;
        if before.space != SpaceId::Model {
            return Err(CadError::Unsupported(format!(
                "property.not_addressable: GRIPSTRETCH target {} is a {} entity; grip \
                 editing addresses model-space entities only",
                before.id.0,
                space_kind(&before.space)
            )));
        }

        // Re-validate the handle against the current geometry. An `Err` here
        // skips the transaction entirely, which is why a zero displacement is an
        // error in the model rather than an `Ok` with unchanged geometry.
        let geometry = apply_grip(&before.geometry, &grip.id, grip.to)?;

        let mut after = before.clone();
        after.geometry = geometry;

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("stretch grip", transaction)?;
            staged.update_entity(after)?;
            staged.commit()?
        };
        // Read the committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let after = candidate.entity(before.id).cloned().ok_or_else(|| {
            CadError::Invariant("grip-stretched entity vanished after commit".into())
        })?;
        let object = ObjectId(before.id.0);
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "stretch grip".into(),
                patches: vec![drawing_patch(before.id, Some(before), Some(after))],
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![object],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(object),
                code: "drawing.gripstretch".into(),
                message: "GRIPSTRETCH: moved one grip handle".into(),
            }],
            measurement: None,
        })
    }
}

/// Name the space a refused grip target lives in (ASCII, for the reason text).
fn space_kind(space: &SpaceId) -> &'static str {
    match space {
        SpaceId::Model => "model-space",
        SpaceId::Paper(_) => "paper-space",
        SpaceId::Block(_) => "block-definition",
    }
}
