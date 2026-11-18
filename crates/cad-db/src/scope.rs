//! The explicit dirty scope of one commit (ADR-0037 D1/D8/D11/D12).
//!
//! [`ChangeSet::changes`](crate::ChangeSet) answers "which *entities* changed and
//! how" with a 5-bit [`ChangeMask`](crate::ChangeMask). It cannot answer "which
//! persisted rows must be rewritten": layer, layout, style, linetype, scale and
//! header writers publish an **empty** change list and only raise the revision
//! (see `drawing::layer_edit`), and even an entity entry carries no table or
//! model identity. A saver that only knew `revision != saved_revision` could
//! therefore "succeed" while writing nothing.
//!
//! [`ChangeScope`] is that missing answer: the exact set of
//! element shards, table rows and the document header a commit made stale.
//! Every write path in this crate fills it; the session accumulates it across
//! commits, and the store uses it as the dirty half of D11's entry set
//! (`new_entries = live_shards ∪ dirty_shards ∪ tables_present ∪
//! foreign_entries`, where `dirty_shards = shards ∪ prev_shards`).
//!
//! # In memory only, never persisted
//!
//! A `ChangeScope` is **commit metadata**, not document state. It is never
//! serialized into `.ymodel`: the file records data, and the dirty set is
//! recomputed from the edits since the last save. Nothing on disk depends on it.
//!
//! # Why it lives on `ChangeSet` and not in `DrawingDatabase`
//!
//! [`DrawingDatabase`] is `Clone`, is shared with the scene thread behind an
//! `Arc`, and is edited through `Arc::make_mut` (see
//! `crates/cad-app/src/app_history.rs`). A batch stored inside it would be
//! **forked** by that clone: the editor's copy would accumulate dirt while the
//! scene thread's snapshot kept the old set, and `Arc::make_mut` would reset it
//! on every first write after a snapshot. The `ChangeSet` is the value that
//! travels with the edit (host → session → store), so the scope belongs there.
//!
//! # What each field means
//!
//! * [`ChangeScope::shards`] — `elements/<model>-<shard>.json` entries whose
//!   **new** content must be written (insert/update rows, plus any row whose
//!   non-`DbEntity` payload changed, e.g. a linetype/scale name cascade that
//!   retargets render attributes);
//! * [`ChangeScope::prev_shards`] — entries the **old** row occupied. A moved
//!   entity dirties both sides (the old shard loses a row); a delete has only
//!   this side;
//! * [`ChangeScope::tables`] — named tables whose rows changed;
//! * [`ChangeScope::header`] — `document.json` must be rewritten: a header
//!   field changed, or an id high-water mark moved (a row was created).
//!
//! Shard assignment follows ADR-0037 D11 exactly: `seq / 2048` inside one Model,
//! the Model derived from the in-memory [`SpaceId`] ([`ModelKey::from_space`]).
//! Block **definition** rows, styles and categories live in the document-level
//! [`ModelKey::Definitions`] model at `shard_of(id)`; their members live in
//! [`ModelKey::Block`].

use std::collections::BTreeSet;

use cad_domain::ymodel::{shard_of, ModelKey, Shard};
use cad_domain::{BlockId, EntityId, LayerId, LayoutId, LinetypeId, SpaceId};

/// A named table whose rows are persisted as a table rather than as elements.
///
/// The set is deliberately closed: a new table must be added here (and to the
/// store's table dispatch) instead of being silently unpersisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TableKind {
    /// Layer rows (including the attributes stored beside each row).
    Layers,
    /// Layout rows (paper-space definitions).
    Layouts,
    /// Text styles.
    Styles,
    /// Dimension styles.
    DimStyles,
    /// Multileader styles.
    MLeaderStyles,
    /// Linetype rows (the name cascade that touches layer/entity attributes is
    /// recorded as [`TableKind::Layers`] plus element shards on the same write).
    Linetypes,
    /// Named annotation scales.
    Scales,
    /// Page-setup records keyed by layout.
    PlotSettings,
    /// External-reference metadata.
    ExternalReferences,
}

/// The set of persisted rows one commit made stale.
///
/// This is commit metadata only — it is never written into a document (see the
/// module docs). All fields are public because the store reads them directly;
/// the builder-style methods are the write paths' idiomatic way to fill them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeScope {
    /// Element shards whose **new** content must be written.
    pub shards: BTreeSet<(ModelKey, Shard)>,
    /// Element shards the **old** row occupied (moves and deletes).
    pub prev_shards: BTreeSet<(ModelKey, Shard)>,
    /// Tables whose rows changed.
    pub tables: BTreeSet<TableKind>,
    /// `document.json` must be rewritten.
    pub header: bool,
}

impl ChangeScope {
    /// True when this commit changed nothing persistable.
    ///
    /// A no-op write path returns an empty scope together with the empty
    /// `changes` list it already returns: the two facts have to agree, or a
    /// saver would rewrite a file for an edit that stored nothing.
    pub fn is_empty(&self) -> bool {
        self.shards.is_empty()
            && self.prev_shards.is_empty()
            && self.tables.is_empty()
            && !self.header
    }

    /// Union another commit's scope into this one.
    ///
    /// Used where one published `ChangeSet` spans several validated database
    /// writes (the command layer folds them into one revision span), so the
    /// dirty set must grow with every half.
    pub fn merge(&mut self, other: &ChangeScope) {
        self.shards.extend(other.shards.iter().copied());
        self.prev_shards.extend(other.prev_shards.iter().copied());
        self.tables.extend(other.tables.iter().copied());
        self.header |= other.header;
    }

    /// Record the new row of an element in `space`.
    pub fn element(&mut self, space: &SpaceId, seq: u128) {
        self.element_in(ModelKey::from_space(space.clone()), seq);
    }

    /// Record the shard the **old** row of an element occupied.
    pub fn prev_element(&mut self, space: &SpaceId, seq: u128) {
        self.prev_element_in(ModelKey::from_space(space.clone()), seq);
    }

    /// Record the new row of an element in an explicit [`ModelKey`].
    pub fn element_in(&mut self, model: ModelKey, seq: u128) {
        self.shards.insert((model, shard_of(seq)));
    }

    /// Record the shard the old row occupied in an explicit [`ModelKey`].
    ///
    /// A delete has only this side: the row disappears from that shard, so the
    /// entry must still be rewritten even though no new row names it.
    pub fn prev_element_in(&mut self, model: ModelKey, seq: u128) {
        self.prev_shards.insert((model, shard_of(seq)));
    }

    /// Record a **block definition** row (it lives in [`ModelKey::Definitions`]).
    pub fn definition(&mut self, seq: u128) {
        self.element_in(ModelKey::Definitions, seq);
    }

    /// Record the [`ModelKey::Definitions`] shard an old block definition row
    /// occupied.
    pub fn prev_definition(&mut self, seq: u128) {
        self.prev_element_in(ModelKey::Definitions, seq);
    }

    /// Record that one table's rows changed.
    pub fn table(&mut self, kind: TableKind) {
        self.tables.insert(kind);
    }

    /// Record that `document.json` must be rewritten.
    pub fn header(&mut self) {
        self.header = true;
    }
}

impl crate::drawing::DrawingDatabase {
    /// The id high-water marks persisted today (ADR-0037 D5), in a fixed order.
    ///
    /// A commit that moved one of these has made `document.json` stale even when
    /// it changed no header field: the counters are stored there, and rebuilding
    /// them from `max(id) + 1` on load would hand a deleted id to a new object.
    ///
    /// ADR-0037 D5 requires **nine**: `next_linetype_id` does not exist yet, so
    /// linetype creation still derives `max(row id) + 1` and can reuse a deleted
    /// id. That field and its mint path are added with the header slice, and this
    /// array grows with it.
    pub(crate) fn identity_watermarks(&self) -> [u128; 8] {
        [
            self.next_entity_id,
            self.next_block_id,
            self.next_layer_id,
            self.next_layout_id,
            self.next_style_id,
            self.next_dim_style_id,
            self.next_mleader_style_id,
            self.next_scale_id,
        ]
    }

    /// The dirty scope of one combined drawing batch, read **before** it mutates.
    ///
    /// [`Self::apply_drawing_changes_combined`](crate::DrawingDatabase::apply_drawing_changes_combined)
    /// is the choke point for entity, block, layer, layout, style and linetype
    /// rows, so this is where all of them are mapped onto persisted entries:
    ///
    /// * an entity row dirties the shard of its **new** space and the shard of
    ///   the **old** row (a move straddles two, a delete has only the old side);
    /// * a block **definition** row lives in [`ModelKey::Definitions`] at its own
    ///   sequence, while its members live in [`ModelKey::Block`];
    /// * a non-empty table batch dirties its [`TableKind`], and a page-setup row
    ///   that appears, changes or disappears dirties [`TableKind::PlotSettings`]
    ///   even when its layout row only moved;
    /// * a row that did not exist yet moves that table's id high-water mark, so
    ///   `document.json` must be rewritten (`header`);
    /// * a linetype rename cascades the stored name into every layer row's
    ///   attributes in the same mutation, so those layer rows are dirty too.
    ///
    /// Deliberately not filled here: display states and the entity ids a linetype
    /// cascade retargeted. Their rows exist only **after** the batch lands, so
    /// the apply path adds them once the mutation is done.
    pub(crate) fn combined_change_scope(
        &self,
        block_changes: &[(BlockId, Option<crate::drawing::BlockRow>)],
        changes: &[(EntityId, Option<crate::entity::DbEntity>)],
        layer_changes: &[(LayerId, Option<crate::drawing::LayerRow>)],
        layout_changes: &[(LayoutId, Option<crate::drawing::LayoutRow>)],
        style_changes: crate::drawing::StyleTableChanges<'_>,
        linetype_changes: &[(LinetypeId, Option<crate::tables::LineType>)],
    ) -> ChangeScope {
        let mut scope = ChangeScope::default();
        for (id, change) in changes {
            match change {
                Some(entity) => {
                    scope.element(&entity.space, id.0);
                    match self.entities.get(id) {
                        Some(previous) => scope.prev_element(&previous.space, id.0),
                        // A row entered the document: its id must be reflected by
                        // the persisted high-water mark, which the allocator (or
                        // the explicitly supplied id) already advanced.
                        None => scope.header(),
                    }
                }
                None => {
                    if let Some(previous) = self.entities.get(id) {
                        scope.prev_element(&previous.space, id.0);
                    }
                }
            }
        }
        for (id, change) in block_changes {
            let existed = self.blocks.contains_key(id);
            if change.is_some() {
                scope.definition(id.0);
                if !existed {
                    scope.header();
                }
            } else if existed {
                scope.prev_definition(id.0);
            }
        }
        if !layer_changes.is_empty() {
            scope.table(TableKind::Layers);
            if layer_changes
                .iter()
                .any(|(id, change)| change.is_some() && !self.layers.contains_key(id))
            {
                scope.header();
            }
        }
        if !layout_changes.is_empty() {
            scope.table(TableKind::Layouts);
            for (id, change) in layout_changes {
                let had_plot = self.plot_settings.contains_key(id);
                let writes_plot = change.as_ref().is_some_and(|row| row.plot.is_some());
                if had_plot || writes_plot {
                    scope.table(TableKind::PlotSettings);
                }
                if change.is_some() && !self.layouts.contains_key(id) {
                    scope.header();
                }
            }
        }
        if !style_changes.is_empty() {
            // One helper for the combined path and the public style entry, so the
            // two cannot disagree about which tables a staged row dirties.
            scope.merge(&self.style_batch_scope(style_changes));
        }
        if !linetype_changes.is_empty() {
            scope.table(TableKind::Linetypes);
            let renamed: Vec<&str> = linetype_changes
                .iter()
                .filter_map(|(id, change)| {
                    let previous = self.linetypes.get(id)?.name.trim();
                    let after = change.as_ref()?.name.trim();
                    (previous != after).then_some(previous)
                })
                .collect();
            if !renamed.is_empty()
                && self.layer_attributes.values().any(|attributes| {
                    renamed
                        .iter()
                        .any(|old| attributes.linetype.eq_ignore_ascii_case(old))
                })
            {
                scope.table(TableKind::Layers);
            }
        }
        scope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn scope_is_send_and_sync() {
        // The session accumulates scopes across threads (edit path vs. store)
        // and must be able to hand one to a saver.
        _assert_send_sync::<ChangeScope>();
        _assert_send_sync::<TableKind>();
    }

    #[test]
    fn default_is_empty_and_a_table_alone_is_not() {
        let mut scope = ChangeScope::default();
        assert!(scope.is_empty());
        scope.table(TableKind::Layers);
        assert!(!scope.is_empty());
    }

    #[test]
    fn elements_use_the_shard_rule_and_model_of_their_space() {
        let mut scope = ChangeScope::default();
        scope.element(&SpaceId::Model, 2049);
        scope.prev_element(&SpaceId::Paper(cad_domain::LayoutId(1)), 0);
        scope.definition(1);
        let expected = [
            (ModelKey::ModelSpace, Shard(1)),
            (ModelKey::Definitions, Shard(0)),
        ];
        assert_eq!(
            scope.shards.iter().copied().collect::<Vec<_>>(),
            expected.to_vec()
        );
        assert_eq!(
            scope.prev_shards.iter().copied().collect::<Vec<_>>(),
            vec![(ModelKey::Paper(cad_domain::LayoutId(1)), Shard(0))]
        );
    }

    #[test]
    fn merge_unions_every_part() {
        let mut left = ChangeScope::default();
        left.table(TableKind::Layers);
        let mut right = ChangeScope::default();
        right.element(&SpaceId::Model, 3);
        right.header();
        left.merge(&right);
        assert!(left.tables.contains(&TableKind::Layers));
        assert!(left.shards.contains(&(ModelKey::ModelSpace, Shard(0))));
        assert!(left.header);
        // Merging is idempotent.
        left.merge(&right);
        assert_eq!(left.shards.len(), 1);
    }
}
