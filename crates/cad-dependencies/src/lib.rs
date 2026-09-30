//! Reverse dependency index and invalidation propagation (spec v2.0 §16.2).
//!
//! Changes are first resolved to the affected set, then only the derived state
//! that actually depends on them is invalidated. A change that only touches
//! metadata does not invalidate representations; a change to geometry does.

use cad_db::{ChangeMask, ChangeSet};
use cad_domain::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// What must be rebuilt after a change set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Invalidation {
    pub representations: Vec<ObjectId>,
    pub bounds: Vec<ObjectId>,
    pub spatial: Vec<ObjectId>,
    pub query: Vec<ObjectId>,
    /// True when the subscriber must rebuild a full snapshot because the
    /// revision stream was not continuous (spec §4.6).
    pub rebuild_snapshot: bool,
}

impl Invalidation {
    pub fn is_empty(&self) -> bool {
        !self.rebuild_snapshot
            && self.representations.is_empty()
            && self.bounds.is_empty()
            && self.spatial.is_empty()
            && self.query.is_empty()
    }
}

struct Edge {
    consumer: ObjectId,
    mask: ChangeMask,
}

/// Tracks which objects depend on which, so a change propagates to a bounded
/// set of consumers instead of recomputing the whole drawing.
pub struct DependencyIndex {
    pub max_depth: usize,
    pub max_affected: usize,
    /// dependency -> consumers/dependents
    dependents: BTreeMap<ObjectId, Vec<Edge>>,
    /// consumer -> dependencies it registered
    dependencies: BTreeMap<ObjectId, BTreeSet<ObjectId>>,
    last_revision: Option<Revision>,
    database: Option<DatabaseId>,
}

impl Default for DependencyIndex {
    fn default() -> Self {
        Self::new(64, 100_000)
    }
}

impl DependencyIndex {
    pub fn new(max_depth: usize, max_affected: usize) -> Self {
        DependencyIndex {
            max_depth,
            max_affected,
            dependents: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            last_revision: None,
            database: None,
        }
    }

    /// Register that `consumer` depends on `dependency` with a change mask.
    pub fn register_masked(&mut self, consumer: ObjectId, dependency: ObjectId, mask: ChangeMask) -> CadResult<()> {
        if consumer == dependency {
            return Err(CadError::Invariant("an object cannot depend on itself".to_string()));
        }
        self.dependencies.entry(consumer).or_default().insert(dependency);
        let edges = self.dependents.entry(dependency).or_default();
        if let Some(existing) = edges.iter_mut().find(|e| e.consumer == consumer) {
            existing.mask = existing.mask.union(mask);
        } else {
            edges.push(Edge { consumer, mask });
        }
        Ok(())
    }

    /// Default registration: any representation-affecting change.
    pub fn register(&mut self, consumer: ObjectId, dependency: ObjectId) -> CadResult<()> {
        let mask = ChangeMask::GEOMETRY
            .union(ChangeMask::STYLE)
            .union(ChangeMask::TRANSFORM)
            .union(ChangeMask::REFERENCES);
        self.register_masked(consumer, dependency, mask)
    }

    pub fn clear(&mut self) {
        self.dependents.clear();
        self.dependencies.clear();
    }

    /// Propagate a change set to the affected consumers.
    pub fn invalidate(&mut self, changes: &ChangeSet) -> CadResult<Invalidation> {
        let mut out = Invalidation::default();

        // Detect a revision gap: if we already saw a later revision, or the
        // change set does not follow the last one we processed, we must rebuild.
        if let Some(last) = self.last_revision {
            let continuous = changes.follows(changes.database, last)
                || changes.before == last
                || changes.after == Revision(last.0 + 1);
            if !continuous && changes.before != changes.after {
                out.rebuild_snapshot = true;
            }
        }
        if let Some(db) = self.database {
            if db != changes.database {
                out.rebuild_snapshot = true;
            }
        }
        self.database = Some(changes.database);
        self.last_revision = Some(changes.after);

        if out.rebuild_snapshot {
            return Ok(out);
        }

        // Roots: the changed objects themselves plus their direct dependents.
        let mut queue: VecDeque<(ObjectId, usize)> = VecDeque::new();
        let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
        let mut affected: BTreeSet<ObjectId> = BTreeSet::new();

        for change in &changes.changes {
            let (id, mask) = match change {
                ObjectChange::Insert(id) => (*id, ChangeMask::GEOMETRY.union(ChangeMask::REFERENCES)),
                ObjectChange::Delete(id) => (*id, ChangeMask::GEOMETRY.union(ChangeMask::REFERENCES)),
                ObjectChange::Update(id, mask) => (*id, *mask),
            };
            // Only representation-affecting changes propagate downward.
            if !mask.invalidates_representation() && !matches!(change, ObjectChange::Delete(_)) {
                continue;
            }
            queue.push_back((id, 0));
            seen.insert(id);
            affected.insert(id);
        }

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= self.max_depth {
                out.rebuild_snapshot = true;
                break;
            }
            if let Some(edges) = self.dependents.get(&node) {
                for edge in edges {
                    if affected.len() >= self.max_affected {
                        out.rebuild_snapshot = true;
                        break;
                    }
                    if !edge.mask.invalidates_representation() {
                        continue;
                    }
                    affected.insert(edge.consumer);
                    if seen.insert(edge.consumer) {
                        queue.push_back((edge.consumer, depth + 1));
                    }
                }
            }
        }

        // Classify: consumers that are themselves depended upon are structural;
        // here we simply report the whole affected set as needing new
        // representations, and mirror it into the other buckets so callers can
        // page the work. A finer classification is a documented follow-up.
        out.representations = affected.iter().copied().collect();
        out.bounds = out.representations.clone();
        out.spatial = out.representations.clone();
        out.query = out.representations.clone();
        Ok(out)
    }

    /// Rebuild after a missed event: everything must be re-derived.
    pub fn force_rebuild(&mut self, revision: Revision) -> Invalidation {
        self.last_revision = Some(revision);
        Invalidation { rebuild_snapshot: true, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{ObjectChange, ChangeSet};

    fn change_set(before: u64, after: u64, changes: Vec<ObjectChange>) -> ChangeSet {
        ChangeSet {
            database: DatabaseId(1),
            before: Revision(before),
            after: Revision(after),
            transaction: TransactionId(1),
            reason: "test".into(),
            changes,
        }
    }

    #[test]
    fn geometry_change_invalidates_dependents() {
        let mut index = DependencyIndex::new(8, 100);
        index.register(ObjectId(100), ObjectId(1)).unwrap();
        let inv = index
            .invalidate(&change_set(0, 1, vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]))
            .unwrap();
        assert!(!inv.rebuild_snapshot);
        assert!(inv.representations.contains(&ObjectId(100)));
    }

    #[test]
    fn metadata_only_change_does_not_invalidate_representations() {
        let mut index = DependencyIndex::new(8, 100);
        index.register(ObjectId(100), ObjectId(1)).unwrap();
        let inv = index
            .invalidate(&change_set(0, 1, vec![ObjectChange::Update(ObjectId(1), ChangeMask::METADATA)]))
            .unwrap();
        assert!(inv.is_empty(), "metadata change must not invalidate representations");
    }

    #[test]
    fn transitive_dependencies_propagate() {
        let mut index = DependencyIndex::new(8, 100);
        index.register(ObjectId(2), ObjectId(1)).unwrap();
        index.register(ObjectId(3), ObjectId(2)).unwrap();
        let inv = index
            .invalidate(&change_set(0, 1, vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]))
            .unwrap();
        assert!(inv.representations.contains(&ObjectId(2)));
        assert!(inv.representations.contains(&ObjectId(3)));
    }

    #[test]
    fn revision_gap_forces_snapshot_rebuild() {
        let mut index = DependencyIndex::new(8, 100);
        index.invalidate(&change_set(0, 1, vec![])).unwrap();
        // Jump from revision 1 to 5: not continuous.
        let inv = index
            .invalidate(&change_set(4, 5, vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]))
            .unwrap();
        assert!(inv.rebuild_snapshot);
    }

    #[test]
    fn self_dependency_is_rejected() {
        let mut index = DependencyIndex::new(8, 100);
        assert!(index.register(ObjectId(1), ObjectId(1)).is_err());
    }
}
