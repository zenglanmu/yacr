//! Unit tests.

use super::*;
use cad_db::{AnnotationGeometry, AnnotationStyle};

fn ann(id: u128, text: &str) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        space: SpaceId::Model,
        geometry: AnnotationGeometry::Text(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }),
        text: text.to_string(),
        style: AnnotationStyle::default(),
        created_unix_ms: 0,
        modified_unix_ms: 0,
        anchor: None,
        precision: Precision::Analytic,
    }
}

fn database_with() -> AnnotationDatabase {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    db.apply_annotation_changes(
        "seed",
        TransactionId(0),
        vec![(AnnotationId(1), Some(ann(1, "a")))],
    )
    .unwrap();
    db
}

#[test]
fn create_then_undo_removes_and_redo_restores() {
    let mut db = database_with();
    let mut history = History::default();
    // The creation of annotation 5 was already committed elsewhere:
    db.apply_annotation_changes(
        "create",
        TransactionId(1),
        vec![(AnnotationId(5), Some(ann(5, "b")))],
    )
    .unwrap();
    history
        .record(UndoRecord {
            transaction: TransactionId(1),
            label: "create b".into(),
            patches: vec![patch(AnnotationId(5), None, Some(ann(5, "b")))],
            merge_key: None,
        })
        .unwrap();

    history.undo(&mut db).unwrap();
    assert!(db.get(AnnotationId(5)).is_none());
    history.redo(&mut db).unwrap();
    assert!(db.get(AnnotationId(5)).is_some());
}

#[test]
fn update_restores_previous_value() {
    let mut db = database_with();
    let mut history = History::default();
    db.apply_annotation_changes(
        "update",
        TransactionId(2),
        vec![(AnnotationId(1), Some(ann(1, "edited")))],
    )
    .unwrap();
    history
        .record(UndoRecord {
            transaction: TransactionId(2),
            label: "edit a".into(),
            patches: vec![patch(
                AnnotationId(1),
                Some(ann(1, "a")),
                Some(ann(1, "edited")),
            )],
            merge_key: None,
        })
        .unwrap();
    history.undo(&mut db).unwrap();
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "a");
}

#[test]
fn merge_key_coalesces_records() {
    let mut history = History::default();
    let r = |t: &str| UndoRecord {
        transaction: TransactionId(1),
        label: "drag".into(),
        patches: vec![patch(AnnotationId(1), Some(ann(1, "a")), Some(ann(1, t)))],
        merge_key: Some("drag-1".into()),
    };
    history.record(r("mid")).unwrap();
    history.record(r("end")).unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn empty_record_is_rejected() {
    let mut history = History::default();
    let r = UndoRecord {
        transaction: TransactionId(1),
        label: "noop".into(),
        patches: vec![],
        merge_key: None,
    };
    assert!(history.record(r).is_err());
}

#[test]
fn budget_evicts_oldest() {
    let mut history = History::new(1);
    for i in 0..5 {
        history
            .record(UndoRecord {
                transaction: TransactionId(1),
                label: format!("step {i}"),
                patches: vec![patch(AnnotationId(i), None, Some(ann(i, "x")))],
                merge_key: None,
            })
            .unwrap();
    }
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn journal_recovers_state() {
    let mut journal = MemoryJournal::new();
    journal
        .append(&UndoRecord {
            transaction: TransactionId(9),
            label: "create".into(),
            patches: vec![patch(AnnotationId(7), None, Some(ann(7, "recovered")))],
            merge_key: None,
        })
        .unwrap();
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let sets = journal.recover(&mut db).unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(db.get(AnnotationId(7)).unwrap().text, "recovered");
}
