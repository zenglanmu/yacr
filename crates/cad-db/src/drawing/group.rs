//! Persisted entity groups (`tables/groups.json`, AutoCAD `AcDbGroup`).
//!
//! A group is a named, ordered set of **directly addressable** entities. Unlike
//! the former session-only store, the table is document data: it is written to
//! `.ymodel`, copied by DXF/DWG import/export, versioned by the format minor and
//! reachable through one write path that publishes a [`ChangeSet`].
//!
//! # Identity
//!
//! The row identity is a table-local [`GroupSeq`], never a [`cad_domain`]
//! `PersistentId`: groups have no header counter and no `IdKind` (the same
//! storage decision as parameters, ADR-0043 D4/D12). The name is user-visible
//! and unique **case-insensitively**; a rename keeps the sequence, so a history
//! patch addresses the same group on both sides.
//!
//! # Members
//!
//! Members keep capture order and are unique by [`cad_domain::EntityId`]. The
//! command layer only accepts direct refs (no block-instance path, no
//! sub-element), because a persisted group stores entity handles and has no
//! instance-path model. Every member must exist in the database both at
//! [`DrawingDatabase::apply_group_changes`] time and at builder `finish()`.
//!
//! # One write path
//!
//! [`DrawingDatabase::apply_group_changes`] validates the whole batch first,
//! then mutates and raises the revision **once**, returning one [`ChangeSet`]
//! whose scope names [`TableKind::Groups`]. A refusal leaves the table, the
//! revision and the history untouched.

use std::collections::BTreeSet;

use cad_domain::{CadError, CadResult, EntityId, Revision, TransactionId};

use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::scope::{ChangeScope, TableKind};

/// Table-local identity of one persisted group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupSeq(pub u64);

/// One persisted group row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    /// Table-local identity; stable across renames.
    pub seq: GroupSeq,
    /// User-visible name, trimmed. Unique case-insensitively in the table.
    pub name: String,
    /// Free-form description (DXF code 300). Empty is omitted from the archive.
    pub description: String,
    /// Whether the group is selectable (DXF code 71). The importer stores the
    /// source flag and the exporter writes it back; no command mutates it.
    pub selectable: bool,
    /// Members in capture order, unique.
    pub members: Vec<EntityId>,
}

impl GroupRow {
    /// An empty-member group with the DXF defaults (`selectable = true`).
    pub fn new(seq: GroupSeq, name: impl Into<String>, members: Vec<EntityId>) -> Self {
        Self {
            seq,
            name: name.into(),
            description: String::new(),
            selectable: true,
            members,
        }
    }

    /// Whether `name` resolves to this row (case-insensitive, trimmed).
    pub fn has_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name.trim())
    }
}

/// A before/after record of one group row.
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete). The sequence is carried separately so a rename can never
/// be mistaken for a different identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPatch {
    pub seq: GroupSeq,
    pub before: Option<GroupRow>,
    pub after: Option<GroupRow>,
}

impl DrawingDatabase {
    /// Every group row, oldest sequence first.
    pub fn groups(&self) -> impl Iterator<Item = &GroupRow> + '_ {
        self.groups.values()
    }

    /// One group by its table-local sequence.
    pub fn group(&self, seq: GroupSeq) -> Option<&GroupRow> {
        self.groups.get(&seq)
    }

    /// Resolve a name: exact match first, then a unique case-insensitive one.
    pub fn group_by_name(&self, name: &str) -> Option<&GroupRow> {
        let name = name.trim();
        if let Some(found) = self.groups.values().find(|row| row.name == name) {
            return Some(found);
        }
        let mut matches = self
            .groups
            .values()
            .filter(|row| row.name.eq_ignore_ascii_case(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Whether a name is already taken (case-insensitively).
    pub fn group_name_taken(&self, name: &str) -> bool {
        let name = name.trim();
        self.groups
            .values()
            .any(|row| row.name.eq_ignore_ascii_case(name))
    }

    /// The sequence a newly created group should use.
    ///
    /// Mirrors the parameter table: `max(existing) + 1`, never a persisted
    /// counter. A deleted maximum may therefore be reused; the sequence is a
    /// table-local address, not a durable object id.
    pub fn next_group_seq(&self) -> GroupSeq {
        GroupSeq(
            self.groups
                .keys()
                .next_back()
                .map(|seq| seq.0)
                .unwrap_or(0)
                .saturating_add(1),
        )
    }

    /// Apply a validated batch of group-row patches as one revision.
    ///
    /// `patches` is a set of before/after rows; a create has `before: None`, a
    /// delete `after: None`, and an edit both. An empty batch is a no-op: same
    /// revision, no changes, an empty scope. A content change raises the
    /// revision by exactly one and publishes one [`ChangeSet`] whose scope marks
    /// [`TableKind::Groups`].
    ///
    /// # Validation (before any mutation)
    ///
    /// * every patch names its row's `seq` consistently on both sides;
    /// * no sequence appears twice in the batch;
    /// * every final name is non-empty after trimming and unique
    ///   case-insensitively (a rename may only swap case with itself);
    /// * every member exists in the database and appears once.
    ///
    /// A refused batch leaves the table, the revision and the allocators exactly
    /// as they were.
    pub fn apply_group_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        patches: &[GroupPatch],
    ) -> CadResult<ChangeSet> {
        let before = self.revision;
        let noop = ChangeSet {
            database: self.id,
            before,
            after: before,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope: ChangeScope::default(),
        };
        if patches.is_empty() {
            return Ok(noop);
        }

        // Phase 1: validate the batch and build the final table.
        let mut next = self.groups.clone();
        let mut changed = false;
        let mut seen_seqs: BTreeSet<GroupSeq> = BTreeSet::new();
        for patch in patches {
            if !seen_seqs.insert(patch.seq) {
                return Err(CadError::Invariant(format!(
                    "a group batch names sequence {} more than once",
                    patch.seq.0
                )));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "group.invalid_patch: a group patch needs a before or after row".into(),
                ));
            }
            for row in patch.before.iter().chain(patch.after.iter()) {
                if row.seq != patch.seq {
                    return Err(CadError::InvalidInput(format!(
                        "group.invalid_patch: a group patch for sequence {} carries a row with \
                         sequence {}",
                        patch.seq.0, row.seq.0
                    )));
                }
            }
            // The stored row must match the before side (if any), so a stale
            // patch cannot silently replace a group another edit created.
            match (&patch.before, next.get(&patch.seq)) {
                (Some(claimed), Some(stored)) if claimed != stored => {
                    return Err(CadError::InvalidInput(format!(
                        "group.stale_patch: group sequence {} changed since the edit was planned",
                        patch.seq.0
                    )));
                }
                (Some(_), None) => {
                    return Err(CadError::InvalidInput(format!(
                        "group.stale_patch: group sequence {} no longer exists",
                        patch.seq.0
                    )));
                }
                (None, Some(_)) => {
                    return Err(CadError::InvalidInput(format!(
                        "group.stale_patch: group sequence {} already exists",
                        patch.seq.0
                    )));
                }
                _ => {}
            }
            if patch.before == patch.after {
                continue;
            }
            changed = true;
            match &patch.after {
                Some(row) => {
                    next.insert(row.seq, row.clone());
                }
                None => {
                    next.remove(&patch.seq);
                }
            }
        }
        if !changed {
            return Ok(noop);
        }

        let mut names: BTreeSet<String> = BTreeSet::new();
        for row in next.values() {
            let name = row.name.trim();
            if name.is_empty() {
                return Err(CadError::InvalidInput(
                    "group.invalid_name: a group name cannot be empty".into(),
                ));
            }
            let folded = name.to_ascii_lowercase();
            if !names.insert(folded) {
                return Err(CadError::InvalidInput(format!(
                    "group.duplicate: a group named '{name}' already exists (names are unique \
                     case-insensitively)"
                )));
            }
            let mut members: BTreeSet<EntityId> = BTreeSet::new();
            for member in &row.members {
                if !self.entities.contains_key(member) {
                    return Err(CadError::InvalidInput(format!(
                        "group.stale_ref: entity {} does not exist in this drawing",
                        member.0
                    )));
                }
                if !members.insert(*member) {
                    return Err(CadError::InvalidInput(format!(
                        "group.duplicate_member: entity {} appears twice in group '{}'",
                        member.0, name
                    )));
                }
            }
        }

        // Phase 2: mutate and publish one revision.
        self.groups = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.tables.insert(TableKind::Groups);
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}

// ---------------------------------------------------------------------------
// Builder / load path.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Replace the document's group table.
    ///
    /// The table's immediate invariants (non-empty names, unique names/sequences
    /// and unique members) are checked here so a malformed archive is refused at
    /// its own row; member existence is checked once at
    /// [`crate::DrawingDatabaseBuilder::finish`], where every entity row exists.
    pub fn set_groups(&mut self, rows: Vec<GroupRow>) -> CadResult<()> {
        let mut table = std::collections::BTreeMap::new();
        let mut names: BTreeSet<String> = BTreeSet::new();
        for row in rows {
            let name = row.name.trim();
            if name.is_empty() {
                return Err(CadError::InvalidInput(
                    "group.invalid_name: a group name cannot be empty".into(),
                ));
            }
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(CadError::InvalidInput(format!(
                    "group.duplicate: a group named '{name}' already exists"
                )));
            }
            let mut members = BTreeSet::new();
            for member in &row.members {
                if !members.insert(*member) {
                    return Err(CadError::InvalidInput(format!(
                        "group.duplicate_member: entity {} appears twice in group '{name}'",
                        member.0
                    )));
                }
            }
            if table.insert(row.seq, row).is_some() {
                return Err(CadError::InvalidInput(
                    "group.duplicate_sequence: two group rows share one sequence".into(),
                ));
            }
        }
        self.database.groups = table;
        Ok(())
    }
}

/// Check a whole document's group table (the builder `finish()` gate).
///
/// Every member must name an entity row that exists in the finished database.
/// A dangling handle is a malformed document, never a silently shortened group.
pub(crate) fn validate_groups(database: &DrawingDatabase) -> CadResult<()> {
    for row in database.groups.values() {
        for member in &row.members {
            if !database.entities.contains_key(member) {
                return Err(CadError::InvalidInput(format!(
                    "group.stale_ref: group '{}' references missing entity {}",
                    row.name, member.0
                )));
            }
        }
    }
    Ok(())
}
