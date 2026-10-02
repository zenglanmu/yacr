//! Change masks and the ordered change sets published by a commit.

use cad_domain::*;

use crate::annotation::Annotation;

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

    /// The precise mask describing how one annotation changed.
    ///
    /// A pure metadata/identity edit (anchor, text payload, timestamps,
    /// precision) must not force a full geometry rebuild; only the geometry,
    /// style, anchor transform and reference fields do (audit B12).
    pub fn for_annotation_update(before: &Annotation, after: &Annotation) -> Self {
        let mut mask = Self(0);
        if before.geometry != after.geometry {
            mask = mask.union(Self::GEOMETRY);
        }
        if before.style != after.style || before.space != after.space || before.text != after.text {
            mask = mask.union(Self::STYLE);
        }
        match (&before.anchor, &after.anchor) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                if a.fallback != b.fallback || a.instance != b.instance {
                    mask = mask.union(Self::TRANSFORM);
                }
                if a.source_handle != b.source_handle
                    || a.sub_element != b.sub_element
                    || a.status != b.status
                {
                    mask = mask.union(Self::REFERENCES);
                }
            }
            // An anchor appearing or disappearing changes both what the
            // annotation references and where it resolves.
            _ => mask = mask.union(Self::TRANSFORM).union(Self::REFERENCES),
        }
        if before.precision != after.precision
            || before.created_unix_ms != after.created_unix_ms
            || before.modified_unix_ms != after.modified_unix_ms
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
