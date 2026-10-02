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
