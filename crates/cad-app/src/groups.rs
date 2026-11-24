//! The bounded session association store behind `TEXTMASK` / `TEXTUNMASK`.
//!
//! This is **not** the entity-group authority. User-facing groups (`GROUP`,
//! `UNGROUP`, `GROUPEDIT`) live in the document
//! ([`cad_db::DrawingDatabase::groups`], `tables/groups.json`) and are saved,
//! imported and exported. This store only pairs a TEXTMASK text entity with the
//! wipeout it created, so a later `TEXTUNMASK` (or a re-run of `TEXTMASK`) can
//! find the mask without inventing a second group table. Those internal pairs
//! are session state and are deliberately not exposed as `cad_status.groups`.
//!
//! ## Bounds and names
//!
//! [`ENTITY_GROUP_CAP`] = **64** associations. Unlike the named-view store, a
//! full store is an explicit refusal rather than a silent drop: a TEXTMASK run
//! that cannot retain its pair refuses before writing the mask.
//!
//! Names are stored trimmed and resolved **exactly first, then by a unique ASCII
//! case-insensitive match**; `TEXTMASK` allocates `*TEXTMASK-<n>` names that do
//! not collide with each other.
//!
//! ## Membership
//!
//! Members keep insertion order and are unique by [`SelectionRef`] equality.
//! No drawing entity is deleted by this store; deleting a mask prunes its pair.

use cad_domain::SelectionRef;

/// How many TEXTMASK associations a session retains.
pub const ENTITY_GROUP_CAP: usize = 64;

/// One named session group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityGroup {
    /// Stored name, trimmed (the command-line grammar refuses an empty name).
    pub name: String,
    /// The captured members, in capture order and unique.
    pub members: Vec<SelectionRef>,
}

/// A plain value store: no database, no history, no I/O and no persistence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityGroups {
    groups: Vec<EntityGroup>,
}

impl EntityGroups {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum number of groups retained.
    pub fn cap(&self) -> usize {
        ENTITY_GROUP_CAP
    }

    /// How many groups are currently retained.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether no group is retained.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The retained groups in creation order, oldest first.
    pub fn groups(&self) -> impl Iterator<Item = &EntityGroup> + '_ {
        self.groups.iter()
    }

    /// Resolve a name: exact match first, then a unique case-insensitive one.
    pub fn resolve(&self, name: &str) -> Option<&EntityGroup> {
        let name = name.trim();
        if let Some(found) = self.groups.iter().find(|group| group.name == name) {
            return Some(found);
        }
        let mut matches = self
            .groups
            .iter()
            .filter(|group| group.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Whether a name is already taken (case-insensitively).
    pub fn contains(&self, name: &str) -> bool {
        self.groups
            .iter()
            .any(|group| group.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Insert a new group. The caller has already checked uniqueness and the cap.
    pub fn create(&mut self, name: &str, members: Vec<SelectionRef>) {
        self.groups.push(EntityGroup {
            name: name.trim().to_string(),
            members,
        });
    }

    /// Remove one group by name, returning whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let name = name.trim();
        let Some(index) = self
            .groups
            .iter()
            .position(|group| group.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.groups.remove(index);
        true
    }

    /// Add every member not already present; returns how many were added.
    pub fn add_members(&mut self, name: &str, members: &[SelectionRef]) -> Option<usize> {
        let group = self
            .groups
            .iter_mut()
            .find(|group| group.name.eq_ignore_ascii_case(name.trim()))?;
        let mut added = 0;
        for member in members {
            if !group.members.contains(member) {
                group.members.push(member.clone());
                added += 1;
            }
        }
        Some(added)
    }

    /// Remove every listed member; returns how many were removed.
    pub fn remove_members(&mut self, name: &str, members: &[SelectionRef]) -> Option<usize> {
        let group = self
            .groups
            .iter_mut()
            .find(|group| group.name.eq_ignore_ascii_case(name.trim()))?;
        let before = group.members.len();
        group
            .members
            .retain(|member| !members.iter().any(|candidate| candidate == member));
        Some(before - group.members.len())
    }

    /// Rename one group, returning `false` when the old name does not resolve.
    ///
    /// The caller has already checked that the new name is free. The stored
    /// casing is the requested one.
    pub fn rename(&mut self, old: &str, new: &str) -> bool {
        let Some(index) = self
            .groups
            .iter()
            .position(|group| group.name.eq_ignore_ascii_case(old.trim()))
        else {
            return false;
        };
        self.groups[index].name = new.trim().to_string();
        true
    }
}
