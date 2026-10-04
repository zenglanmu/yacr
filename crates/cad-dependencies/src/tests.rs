//! Unit tests.

use super::*;
use cad_db::{ChangeSet, ObjectChange};

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
        .invalidate(&change_set(
            0,
            1,
            vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)],
        ))
        .unwrap();
    assert!(!inv.rebuild_snapshot);
    assert!(inv.representations.contains(&ObjectId(100)));
}

#[test]
fn metadata_only_change_does_not_invalidate_representations() {
    let mut index = DependencyIndex::new(8, 100);
    index.register(ObjectId(100), ObjectId(1)).unwrap();
    let inv = index
        .invalidate(&change_set(
            0,
            1,
            vec![ObjectChange::Update(ObjectId(1), ChangeMask::METADATA)],
        ))
        .unwrap();
    assert!(
        inv.is_empty(),
        "metadata change must not invalidate representations"
    );
}

#[test]
fn transitive_dependencies_propagate() {
    let mut index = DependencyIndex::new(8, 100);
    index.register(ObjectId(2), ObjectId(1)).unwrap();
    index.register(ObjectId(3), ObjectId(2)).unwrap();
    let inv = index
        .invalidate(&change_set(
            0,
            1,
            vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)],
        ))
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
        .invalidate(&change_set(
            4,
            5,
            vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)],
        ))
        .unwrap();
    assert!(inv.rebuild_snapshot);
}

#[test]
fn self_dependency_is_rejected() {
    let mut index = DependencyIndex::new(8, 100);
    assert!(index.register(ObjectId(1), ObjectId(1)).is_err());
}

fn assert_snapshot_rebuild(inv: Invalidation) {
    assert!(inv.rebuild_snapshot);
    assert!(!inv.is_empty());
    assert!(inv.representations.is_empty());
    assert!(inv.bounds.is_empty());
    assert!(inv.spatial.is_empty());
    assert!(inv.query.is_empty());
}

#[test]
fn successor_after_with_wrong_before_requires_rebuild() {
    let mut index = DependencyIndex::default();
    index.invalidate(&change_set(0, 1, vec![])).unwrap();
    assert_snapshot_rebuild(index.invalidate(&change_set(0, 2, vec![])).unwrap());
}

#[test]
fn matching_before_does_not_allow_revision_jump_or_regression() {
    for after in [0, 3] {
        let mut index = DependencyIndex::default();
        index.invalidate(&change_set(0, 1, vec![])).unwrap();
        assert_snapshot_rebuild(index.invalidate(&change_set(1, after, vec![])).unwrap());
    }
}

#[test]
fn no_op_commit_at_current_revision_is_accepted() {
    let mut index = DependencyIndex::default();
    assert!(index
        .invalidate(&change_set(4, 4, vec![]))
        .unwrap()
        .is_empty());
    assert!(index
        .invalidate(&change_set(4, 4, vec![]))
        .unwrap()
        .is_empty());
    let inv = index
        .invalidate(&change_set(4, 5, vec![ObjectChange::Insert(ObjectId(1))]))
        .unwrap();
    assert!(!inv.rebuild_snapshot);
    assert_eq!(inv.representations, vec![ObjectId(1)]);
}

#[test]
fn no_op_at_a_different_revision_requires_rebuild() {
    for revision in [0, 2] {
        let mut index = DependencyIndex::default();
        index.invalidate(&change_set(0, 1, vec![])).unwrap();
        assert_snapshot_rebuild(
            index
                .invalidate(&change_set(revision, revision, vec![]))
                .unwrap(),
        );
    }
}

#[test]
fn nonempty_change_without_revision_advance_requires_rebuild() {
    let mut index = DependencyIndex::default();
    assert_snapshot_rebuild(
        index
            .invalidate(&change_set(1, 1, vec![ObjectChange::Insert(ObjectId(1))]))
            .unwrap(),
    );
}

#[test]
fn different_database_requires_rebuild_even_for_no_op() {
    for (before, after) in [(1, 1), (1, 2)] {
        let mut index = DependencyIndex::default();
        index.invalidate(&change_set(0, 1, vec![])).unwrap();
        let mut changes = change_set(before, after, vec![]);
        changes.database = DatabaseId(2);
        assert_snapshot_rebuild(index.invalidate(&changes).unwrap());
    }
}

#[test]
fn maximum_revision_accepts_no_op_and_rejects_wrapped_successor() {
    let mut index = DependencyIndex::default();
    index
        .invalidate(&change_set(u64::MAX - 1, u64::MAX, vec![]))
        .unwrap();
    assert!(index
        .invalidate(&change_set(u64::MAX, u64::MAX, vec![]))
        .unwrap()
        .is_empty());
    assert_snapshot_rebuild(index.invalidate(&change_set(u64::MAX, 0, vec![])).unwrap());
}

#[test]
fn clear_removes_edges_but_preserves_revision_and_database_tracking() {
    let mut index = DependencyIndex::default();
    index.register(ObjectId(2), ObjectId(1)).unwrap();
    index.invalidate(&change_set(0, 1, vec![])).unwrap();
    index.clear();
    let inv = index
        .invalidate(&change_set(1, 2, vec![ObjectChange::Insert(ObjectId(1))]))
        .unwrap();
    assert_eq!(inv.representations, vec![ObjectId(1)]);
    index.clear();
    assert_snapshot_rebuild(index.invalidate(&change_set(3, 4, vec![])).unwrap());
    index.clear();
    let mut changes = change_set(4, 5, vec![]);
    changes.database = DatabaseId(2);
    assert_snapshot_rebuild(index.invalidate(&changes).unwrap());
}

#[test]
fn affected_limit_applies_to_unique_roots() {
    let mut index = DependencyIndex::new(8, 1);
    let inv = index
        .invalidate(&change_set(
            0,
            1,
            vec![
                ObjectChange::Insert(ObjectId(1)),
                ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY),
            ],
        ))
        .unwrap();
    assert!(!inv.rebuild_snapshot);
    assert_eq!(inv.representations, vec![ObjectId(1)]);
    assert_snapshot_rebuild(
        index
            .invalidate(&change_set(
                1,
                2,
                vec![
                    ObjectChange::Insert(ObjectId(1)),
                    ObjectChange::Insert(ObjectId(2)),
                ],
            ))
            .unwrap(),
    );
}

#[test]
fn zero_affected_limit_accepts_metadata_but_rejects_geometry_root() {
    let mut index = DependencyIndex::new(8, 0);
    assert!(index
        .invalidate(&change_set(
            0,
            1,
            vec![ObjectChange::Update(ObjectId(1), ChangeMask::METADATA)],
        ))
        .unwrap()
        .is_empty());
    assert_snapshot_rebuild(
        index
            .invalidate(&change_set(1, 2, vec![ObjectChange::Insert(ObjectId(1))]))
            .unwrap(),
    );
}

#[test]
fn propagation_limit_requests_snapshot_without_partial_results() {
    let mut index = DependencyIndex::new(8, 1);
    index.register(ObjectId(2), ObjectId(1)).unwrap();
    assert_snapshot_rebuild(
        index
            .invalidate(&change_set(0, 1, vec![ObjectChange::Insert(ObjectId(1))]))
            .unwrap(),
    );
}

#[test]
fn leaf_at_depth_boundary_does_not_require_rebuild() {
    for max_depth in [0, 1] {
        let mut index = DependencyIndex::new(max_depth, 2);
        if max_depth == 1 {
            index.register(ObjectId(2), ObjectId(1)).unwrap();
        }
        let inv = index
            .invalidate(&change_set(0, 1, vec![ObjectChange::Insert(ObjectId(1))]))
            .unwrap();
        assert!(!inv.rebuild_snapshot);
        assert_eq!(inv.representations.len(), max_depth + 1);
    }
}

#[test]
fn unseen_consumer_beyond_depth_boundary_requires_rebuild() {
    for max_depth in [0, 1] {
        let mut index = DependencyIndex::new(max_depth, 100);
        index.register(ObjectId(2), ObjectId(1)).unwrap();
        index.register(ObjectId(3), ObjectId(2)).unwrap();
        assert_snapshot_rebuild(
            index
                .invalidate(&change_set(0, 1, vec![ObjectChange::Insert(ObjectId(1))]))
                .unwrap(),
        );
    }
}

#[test]
fn cycle_and_filtered_edges_at_limits_do_not_require_rebuild() {
    let mut index = DependencyIndex::new(1, 2);
    index.register(ObjectId(2), ObjectId(1)).unwrap();
    index.register(ObjectId(1), ObjectId(2)).unwrap();
    index
        .register_masked(ObjectId(3), ObjectId(2), ChangeMask::METADATA)
        .unwrap();
    let inv = index
        .invalidate(&change_set(0, 1, vec![ObjectChange::Insert(ObjectId(1))]))
        .unwrap();
    assert!(!inv.rebuild_snapshot);
    assert_eq!(inv.representations, vec![ObjectId(1), ObjectId(2)]);
}

#[test]
fn converging_dependencies_count_each_consumer_once() {
    let mut index = DependencyIndex::new(1, 3);
    index.register(ObjectId(3), ObjectId(1)).unwrap();
    index.register(ObjectId(3), ObjectId(2)).unwrap();
    let inv = index
        .invalidate(&change_set(
            0,
            1,
            vec![
                ObjectChange::Insert(ObjectId(1)),
                ObjectChange::Insert(ObjectId(2)),
            ],
        ))
        .unwrap();
    assert!(!inv.rebuild_snapshot);
    assert_eq!(
        inv.representations,
        vec![ObjectId(1), ObjectId(2), ObjectId(3)]
    );
}
