//! Change masks and the ordered change sets published by a commit.

use cad_domain::*;

use crate::entity::DbEntity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeMask(pub u8);

impl ChangeMask {
    pub const GEOMETRY: Self = Self(1);
    pub const STYLE: Self = Self(2);
    pub const TRANSFORM: Self = Self(4);
    pub const REFERENCES: Self = Self(8);
    pub const METADATA: Self = Self(16);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether this change requires display representations to be rebuilt.
    pub fn invalidates_representation(self) -> bool {
        self.contains(ChangeMask::GEOMETRY)
            || self.contains(ChangeMask::STYLE)
            || self.contains(ChangeMask::TRANSFORM)
            || self.contains(ChangeMask::REFERENCES)
    }

    /// The precise mask describing how one drawing entity changed.
    ///
    /// - geometry differs → [`Self::GEOMETRY`];
    /// - layer, space or draw order differ → [`Self::STYLE`] (the entity's
    ///   placement/display style, not its shape);
    /// - object bookkeeping (type key, source handle, object revision) differs →
    ///   [`Self::METADATA`];
    /// - a byte-for-byte identical entity still reports [`Self::METADATA`] so an
    ///   update never produces a zero mask that would look like "nothing".
    ///
    /// A caller must **not** use this to describe a MOVE: a rigid transform
    /// changes the geometry but the mask additionally needs [`Self::TRANSFORM`]
    /// so the renderer can take a transform fast path. Only
    /// [`crate::DrawingTransaction::transform_entity`] knows that intent, and it
    /// augments the mask; `for_entity_update` alone cannot infer it.
    pub fn for_entity_update(before: &DbEntity, after: &DbEntity) -> Self {
        let mut mask = Self(0);
        if before.geometry != after.geometry {
            mask = mask.union(Self::GEOMETRY);
        }
        if before.layer != after.layer
            || before.space != after.space
            || before.draw_order != after.draw_order
        {
            mask = mask.union(Self::STYLE);
        }
        if before.object.type_key != after.object.type_key
            || before.object.source_handle != after.object.source_handle
            || before.object.revision != after.object.revision
        {
            mask = mask.union(Self::METADATA);
        }
        if mask.0 == 0 {
            mask = Self::METADATA;
        }
        mask
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectChange {
    Insert(ObjectId),
    Update(ObjectId, ChangeMask),
    Delete(ObjectId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSet {
    pub database: DatabaseId,
    pub before: Revision,
    pub after: Revision,
    pub transaction: TransactionId,
    pub reason: String,
    pub changes: Vec<ObjectChange>,
}

impl ChangeSet {
    /// True when this change set is exactly the successor of `revision`.
    pub fn follows(&self, database: DatabaseId, revision: Revision) -> bool {
        self.database == database
            && self.before == revision
            && self.after.0.checked_sub(1) == Some(revision.0)
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}
