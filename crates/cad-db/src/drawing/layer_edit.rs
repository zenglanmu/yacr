//! The sanctioned layer-table write paths: create, rename, delete and the
//! attribute write.
//!
//! The layer table is part of the authoritative drawing database, so the base
//! drawing is read-only after import (spec §4.3) and these four methods are the
//! **only** sanctioned way to change it — mirroring
//! [`DrawingDatabase::set_block_visibility_state`](super::DrawingDatabase::set_block_visibility_state):
//! validate **everything** first, then mutate, then raise the revision exactly
//! once and publish a [`ChangeSet`]. A refusal at any point leaves the revision,
//! the layer table and every entity exactly as they were.
//!
//! # The `visible == !(off || frozen)` invariant
//!
//! [`Layer::visible`] is the single answered question every consumer reads
//! (scene filtering, the layer panel's `databaseVisible`), while
//! [`LayerAttributes::off`] / [`LayerAttributes::frozen`] record **why** a row is
//! hidden. [`DrawingDatabase::set_layer_attributes`] is the only post-import
//! place that writes both, and it writes them **together** in one mutation, so
//! the invariant can never be observed half-applied: a write that turns `off`
//! or `frozen` on makes the row invisible, and clearing the last of the two
//! makes it visible again.
//!
//! # Why the change set carries no object changes
//!
//! [`ObjectChange`](crate::ObjectChange) addresses entities (`ObjectId` is an
//! entity id); there is no layer object-change kind. A layer edit therefore
//! publishes a `ChangeSet` with an empty `changes` list and a **raised
//! revision**: the revision is the documented staleness tag every derived
//! consumer already binds to (layer panel projection, scene identity, hover
//! cache), so a new/renamed/deleted layer invalidates them without inventing a
//! second dependency graph. Inventing a fake `Update` for an entity on the layer
//! is deliberately **not** done — that would claim geometry/style changed when it
//! did not.
//!
//! # These writes are NOT undoable
//!
//! `cad-history` records entity patches only
//! ([`cad_history::DrawingPatch`] is `{ EntityId, before/after DbEntity }`), so a
//! layer create/rename/delete cannot be represented in an undo record. These
//! methods are deliberately **not** called through a
//! [`DrawingTransaction`](super::DrawingTransaction) and record no undo step;
//! undo/redo leaves the layer table untouched. A layer patch kind is a separate
//! lane; until it exists, do not advertise layers as undoable.
//!
//! # Naming rules
//!
//! Names are stored **trimmed**. A name is refused when it is empty or
//! whitespace-only, when it contains one of the AutoCAD-illegal characters in
//! [`ILLEGAL_LAYER_NAME_CHARS`], or when it duplicates an existing layer name
//! **case-insensitively** (AutoCAD symbol-table names are case-insensitive, so
//! `Walls` and `WALLS` may not coexist). Nothing is clamped, truncated or
//! silently renamed: a bad name is an explicit refusal.
//!
//! # Id allocation
//!
//! Layer ids are allocated monotonically from
//! [`DrawingDatabase::next_layer_id`] (initialised by the builder to
//! `max(existing) + 1`), exactly like entity ids: allocation is **not** rewound
//! by a delete, so a deleted layer id is never handed out again and a stale
//! reference can never alias a new layer.

use cad_domain::*;

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::tables::{Layer, LayerAttributes};

/// Characters AutoCAD forbids in a symbol-table name (layer, linetype, style,
/// block, …): `< > / \ " : ; ? * | , = ` plus the backtick.
///
/// The set comes from the AutoCAD name grammar documented for named objects
/// (`AcDbSymbolTableRecord` names). It is enforced here because a name that the
/// DWG writer/reader of the target CAD would reject must not be created in the
/// first place; the refusal is explicit and changes nothing.
pub const ILLEGAL_LAYER_NAME_CHARS: &[char] = &[
    '<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', ',', '=', '`',
];

/// The AutoCAD default layer name. It can be renamed and re-flagged in AutoCAD,
/// but it can never be deleted; this build matches it exactly (case-sensitive).
pub const DEFAULT_LAYER_NAME: &str = "0";

impl DrawingDatabase {
    /// The next id [`Self::create_layer`] will allocate.
    ///
    /// Allocated the same way entity ids are: from a per-database monotonic
    /// counter, never from "largest existing id + 1" at call time (that would
    /// reuse the id of a just-deleted layer).
    pub fn next_layer_id(&self) -> LayerId {
        LayerId(self.next_layer_id)
    }

    /// Create a new layer table entry.
    ///
    /// Returns the allocated [`LayerId`] plus the [`ChangeSet`] that publishes
    /// the revision bump. The new row is visible (the DWG default for a new
    /// layer) — visibility editing is a separate capability, not part of this
    /// structural operation.
    ///
    /// Refusals (nothing written, revision unchanged): an empty/whitespace-only
    /// name, an AutoCAD-illegal character in the name, and a name that duplicates
    /// an existing layer case-insensitively. Layer edits are **not undoable** —
    /// see the module docs.
    pub fn create_layer(
        &mut self,
        name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<(LayerId, ChangeSet)> {
        // Phase 1: validate. Nothing below mutates until this passes.
        let name = validated_layer_name(name)?;
        self.reject_duplicate_name(&name, None)?;

        // Phase 2: mutate.
        let id = self.next_layer_id();
        self.next_layer_id = self.next_layer_id.saturating_add(1);
        self.layers.insert(
            id,
            Layer {
                id,
                name,
                visible: true,
            },
        );
        // A created layer gets the documented default attributes (on, thawed,
        // unlocked, plottable, ACI 7 white, `Continuous`, ByLayer weight,
        // opaque), which keeps the `visible == !(off || frozen)` invariant: the
        // row is visible and neither flag is set.
        self.layer_attributes.insert(id, LayerAttributes::default());
        let change_set = self.publish_layer_change(transaction, reason);
        Ok((id, change_set))
    }

    /// Rename an existing layer.
    ///
    /// Same validation as [`Self::create_layer`] for the new name (trimmed,
    /// non-empty, legal characters, unique case-insensitively **excluding the
    /// renamed row itself**, so a pure case change `walls` -> `WALLS` is
    /// allowed).
    ///
    /// Renaming a layer to the name it already has is an authoritative no-op: no
    /// revision bump and a `before == after` change set, exactly like
    /// [`DrawingDatabase::set_block_visibility_state`] for an already-active
    /// state. Nothing is fabricated as "changed".
    ///
    /// Refusals (nothing written, revision unchanged): an unknown layer id, an
    /// empty/whitespace-only name, an illegal character, and a duplicate of
    /// another layer's name. Not undoable — see the module docs.
    pub fn rename_layer(
        &mut self,
        id: LayerId,
        name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        // Phase 1: validate.
        let current = self
            .layers
            .get(&id)
            .ok_or_else(|| unknown_layer(id))?
            .name
            .clone();
        let name = validated_layer_name(name)?;
        self.reject_duplicate_name(&name, Some(id))?;
        if current == name {
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }

        // Phase 2: mutate.
        if let Some(layer) = self.layers.get_mut(&id) {
            layer.name = name;
        }
        Ok(self.publish_layer_change(transaction, reason))
    }

    /// Delete a layer table entry.
    ///
    /// Deletion is refused when it would leave a dangling reference or remove the
    /// drawing's default layer:
    ///
    /// * an unknown id;
    /// * the layer named [`DEFAULT_LAYER_NAME`] (`0`, matched exactly) — AutoCAD's
    ///   default layer is a required row;
    /// * a layer still referenced by **any** entity in the drawing (model space,
    ///   paper space or a block member — they are one `entities` table here). The
    ///   refusal names the referencing count, and no entity's `layer` is ever
    ///   re-pointed: deleting an in-use layer is refused, never silently
    ///   "retargeted".
    ///
    /// Not undoable — see the module docs.
    pub fn delete_layer(
        &mut self,
        id: LayerId,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate.
        let layer = self.layers.get(&id).ok_or_else(|| unknown_layer(id))?;
        if layer.name == DEFAULT_LAYER_NAME {
            return Err(CadError::InvalidInput(format!(
                "layer.default_undeletable: layer '{}' is the drawing's default layer and cannot \
                 be deleted",
                DEFAULT_LAYER_NAME
            )));
        }
        let referencing = self.entities.values().filter(|e| e.layer == id).count();
        if referencing > 0 {
            return Err(CadError::InvalidInput(format!(
                "layer.in_use: layer '{}' ({}) is still referenced by {referencing} entit{}; \
                 move or delete them first",
                layer.name,
                id.0,
                if referencing == 1 { "y" } else { "ies" }
            )));
        }

        // Phase 2: mutate.
        self.layers.remove(&id);
        // The attributes row is keyed by layer id and must not outlive its
        // layer: leaving it behind would keep an unreachable entry (and one that
        // a future id-reuse would silently inherit).
        self.layer_attributes.remove(&id);
        Ok(self.publish_layer_change(transaction, reason))
    }

    /// Replace a layer's attributes, keeping the visibility invariant.
    ///
    /// This is the **post-import** attribute write: the builder's
    /// [`DrawingDatabaseBuilder::set_layer_attributes`](crate::DrawingDatabaseBuilder::set_layer_attributes)
    /// stores what the importer resolved, and this method is the only sanctioned
    /// way to change it afterwards. It is deliberately narrow — it writes one
    /// row's attribute set and the `visible` flag derived from it, and nothing
    /// else.
    ///
    /// # What it enforces
    ///
    /// * **the invariant, atomically**: `visible` becomes
    ///   [`LayerAttributes::is_visible`] of the new set (`!(off || frozen)`) in the
    ///   *same* mutation as the attribute row, so no observer can see one
    ///   updated and the other stale;
    /// * **one revision bump per real change**: an attribute set identical to the
    ///   stored one (and a `visible` flag that already agrees) is an
    ///   authoritative no-op — `before == after` in the change set, no revision
    ///   bump, nothing fabricated as changed (same rule as
    ///   [`Self::rename_layer`] to the current name);
    /// * a refusal — an unknown layer id — changes nothing.
    ///
    /// The caller is responsible for the *values*: `cad-db` does not know a
    /// layer's legal key set, its linetype table's names or the wire grammar. It
    /// only keeps the stored state consistent.
    ///
    /// Not undoable — see the module docs.
    pub fn set_layer_attributes(
        &mut self,
        id: LayerId,
        attributes: LayerAttributes,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        // Phase 1: validate. The row must exist; nothing else is this method's
        // call (see "The caller is responsible for the values" above).
        let visible = {
            let layer = self.layers.get(&id).ok_or_else(|| unknown_layer(id))?;
            let visible = attributes.is_visible();
            // The document declares the *attribute* state, so the write is only a
            // no-op when the stored attributes and the derived flag already agree.
            if self.layer_attributes(id) == attributes && layer.visible == visible {
                return Ok(ChangeSet {
                    database: self.id,
                    before,
                    after: before,
                    transaction,
                    reason: reason.to_string(),
                    changes: Vec::new(),
                });
            }
            visible
        };

        // Phase 2: mutate. Both writes are part of this one phase, before the
        // single `publish_layer_change`, so the invariant holds for the whole
        // published state.
        self.layer_attributes.insert(id, attributes);
        if let Some(layer) = self.layers.get_mut(&id) {
            layer.visible = visible;
        }
        Ok(self.publish_layer_change(transaction, reason))
    }

    /// Raise the revision exactly once and build the revision-tagged change set.
    ///
    /// The only mutation step shared by the layer writes: after this the table
    /// change is authoritative and every revision-bound projection (layer rows,
    /// scene identity) is stale by construction.
    fn publish_layer_change(&mut self, transaction: TransactionId, reason: &str) -> ChangeSet {
        let before = self.revision;
        let after = Revision(before.0 + 1);
        self.revision = after;
        // A layer edit is not a geometry change, but every mutation clears the
        // bounds cache so an invalidation can never be missed (same defensive
        // rule as `allocate_entity_id`).
        self.invalidate_bounds_cache();
        ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            // No entity changed: see the module docs ("Why the change set carries
            // no object changes").
            changes: Vec::new(),
        }
    }

    /// Refuse `name` when another layer already uses it case-insensitively.
    ///
    /// `except` is the row being renamed, so a rename may keep its own name
    /// (including a pure case change). The comparison folds with
    /// [`str::to_lowercase`], the same rule the layer panel's search and the
    /// session's name resolution use, so "resolves as a duplicate here" and
    /// "ambiguous there" cannot disagree.
    fn reject_duplicate_name(&self, name: &str, except: Option<LayerId>) -> CadResult<()> {
        let folded = name.to_lowercase();
        if let Some(existing) = self
            .layers
            .values()
            .find(|layer| Some(layer.id) != except && layer.name.to_lowercase() == folded)
        {
            return Err(CadError::InvalidInput(format!(
                "layer.duplicate_name: this drawing already has a layer named '{}' \
                 (names are unique case-insensitively)",
                existing.name
            )));
        }
        Ok(())
    }
}

/// Trim and validate a layer name, returning the value that is stored.
fn validated_layer_name(name: &str) -> CadResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CadError::InvalidInput(
            "layer.name_empty: a layer name must not be empty or whitespace-only".into(),
        ));
    }
    if let Some(illegal) = trimmed
        .chars()
        .find(|c| ILLEGAL_LAYER_NAME_CHARS.contains(c))
    {
        return Err(CadError::InvalidInput(format!(
            "layer.name_illegal_char: a layer name must not contain '{illegal}'"
        )));
    }
    Ok(trimmed.to_string())
}

/// The stable refusal for an id that names no layer in this drawing.
fn unknown_layer(id: LayerId) -> CadError {
    CadError::InvalidInput(format!(
        "layer.unknown_id: this drawing has no layer with id {}",
        id.0
    ))
}
