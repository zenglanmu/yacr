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

fn entity(id: u128, x: f64) -> DbEntity {
    DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3 { x, y: 0.0, z: 0.0 },
            end: Point3 {
                x: x + 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    }
}

#[test]
fn a_drawing_record_keeps_the_shared_undo_order_and_routes_separately() {
    let mut history = History::default();
    // An annotation step, then a drawing create step.
    history
        .record(UndoRecord {
            transaction: TransactionId(1),
            label: "annotate".into(),
            patches: vec![patch(AnnotationId(1), None, Some(ann(1, "n")))],
            merge_key: None,
        })
        .unwrap();
    history
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(2),
            label: "create line".into(),
            patches: vec![drawing_patch(EntityId(5), None, Some(entity(5, 1.0)))],
            merge_key: None,
        })
        .unwrap();
    assert_eq!(history.undo_depth(), 2, "one entry per user action");
    assert!(history.pop_drawing_for_undo());

    // The annotation undo path refuses to touch a drawing step.
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    assert!(history.undo(&mut db).is_err());

    // The drawing path yields the patch and moves to redo on finish.
    let record = history.begin_drawing_undo().unwrap();
    assert_eq!(record.patches[0].id, EntityId(5));
    assert_eq!(history.undo_depth(), 1);
    history.finish_drawing().unwrap();
    assert!(history.next_redo_is_drawing());

    let redone = history.begin_drawing_redo().unwrap();
    assert_eq!(redone.transaction, TransactionId(2));
    history.finish_drawing().unwrap();
    assert_eq!(history.undo_depth(), 2, "redo restores the shared ordering");
    assert!(!history.next_redo_is_drawing());
}

#[test]
fn cancelling_a_pending_drawing_step_restores_it() {
    let mut history = History::default();
    history
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(3),
            label: "create".into(),
            patches: vec![drawing_patch(EntityId(1), None, Some(entity(1, 0.0)))],
            merge_key: None,
        })
        .unwrap();
    let _ = history.begin_drawing_undo().unwrap();
    assert_eq!(history.undo_depth(), 0);
    history.cancel_pending_drawing();
    assert_eq!(history.undo_depth(), 1, "the rejected step is not lost");
    assert!(history.pop_drawing_for_undo());
}

#[test]
fn a_drawing_record_needs_at_least_one_patch() {
    let mut history = History::default();
    let result = history.record_drawing(DrawingUndoRecord {
        transaction: TransactionId(1),
        label: "noop".into(),
        patches: Vec::new(),
        merge_key: None,
    });
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert!(!history.can_undo());
}

fn annotation_record(id: u128, merge_key: Option<&str>) -> UndoRecord {
    UndoRecord {
        transaction: TransactionId(1),
        label: "create".into(),
        patches: vec![patch(AnnotationId(id), None, Some(ann(id, "note")))],
        merge_key: merge_key.map(str::to_owned),
    }
}

fn drawing_record(id: u128) -> DrawingUndoRecord {
    DrawingUndoRecord {
        transaction: TransactionId(id),
        label: "create line".into(),
        patches: vec![drawing_patch(EntityId(id), None, Some(entity(id, 0.0)))],
        merge_key: None,
    }
}

#[test]
fn rejected_annotation_undo_preserves_history_and_database() {
    let mut db = database_with();
    let mut history = History::default();
    history.record(annotation_record(5, None)).unwrap();
    let bytes = history.used_bytes();
    let revision = db.revision();
    assert!(history.undo(&mut db).is_err());
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 0);
    assert_eq!(history.used_bytes(), bytes);
    assert_eq!(db.revision(), revision);
    db.apply_annotation_changes(
        "create",
        TransactionId(1),
        vec![(AnnotationId(5), Some(ann(5, "note")))],
    )
    .unwrap();
    history.undo(&mut db).unwrap();
}

#[test]
fn rejected_annotation_redo_preserves_history_and_database() {
    let mut db = database_with();
    let mut history = History::default();
    history
        .record(UndoRecord {
            transaction: TransactionId(1),
            label: "delete".into(),
            patches: vec![patch(AnnotationId(5), Some(ann(5, "note")), None)],
            merge_key: None,
        })
        .unwrap();
    history.undo(&mut db).unwrap();
    db.apply_annotation_changes(
        "external delete",
        TransactionId(2),
        vec![(AnnotationId(5), None)],
    )
    .unwrap();
    let revision = db.revision();
    assert!(history.redo(&mut db).is_err());
    assert_eq!(history.undo_depth(), 0);
    assert_eq!(history.redo_depth(), 1);
    assert_eq!(history.redo_markers.len(), 1);
    assert_eq!(history.used_bytes(), 0);
    assert_eq!(db.revision(), revision);
    db.apply_annotation_changes(
        "restore",
        TransactionId(3),
        vec![(AnnotationId(5), Some(ann(5, "note")))],
    )
    .unwrap();
    history.redo(&mut db).unwrap();
}

#[test]
fn record_size_is_independent_of_identifier_value() {
    assert_eq!(
        annotation_record(1, None).approx_bytes(),
        annotation_record(u128::MAX, None).approx_bytes()
    );
    assert_eq!(
        drawing_record(1).approx_bytes(),
        drawing_record(u128::MAX).approx_bytes()
    );
}

#[test]
fn merged_record_enforces_budget() {
    let first = annotation_record(1, None);
    let second = annotation_record(2, Some("drag"));
    let budget = first.approx_bytes() + second.approx_bytes();
    let mut history = History::new(budget);
    history.record(first).unwrap();
    history.record(second).unwrap();
    let mut larger = annotation_record(2, Some("drag"));
    larger.patches[0].after.as_mut().unwrap().text = "x".repeat(budget);
    history.record(larger).unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn drawing_accounting_survives_undo_redo_and_cancel() {
    let mut history = History::default();
    history.record_drawing(drawing_record(42)).unwrap();
    let bytes = history.used_bytes();
    assert_eq!(
        bytes,
        history.undo[0].approx_bytes() + history.drawing_undo[0].approx_bytes()
    );
    history.begin_drawing_undo().unwrap();
    assert_eq!(history.used_bytes(), 0);
    history.cancel_pending_drawing();
    assert_eq!(history.used_bytes(), bytes);
    history.begin_drawing_undo().unwrap();
    history.finish_drawing().unwrap();
    history.begin_drawing_redo().unwrap();
    history.cancel_pending_drawing();
    history.begin_drawing_redo().unwrap();
    history.finish_drawing().unwrap();
    assert_eq!(history.used_bytes(), bytes);
}

#[test]
fn pending_drawing_rejects_reentrant_history_mutations() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    history.record_drawing(drawing_record(2)).unwrap();
    history.begin_drawing_undo().unwrap();
    let mut db = database_with();
    assert!(history.begin_drawing_undo().is_err());
    assert!(history.begin_drawing_redo().is_err());
    assert!(history.record(annotation_record(3, None)).is_err());
    assert!(history.record_drawing(drawing_record(3)).is_err());
    assert!(history.undo(&mut db).is_err());
    assert!(history.redo(&mut db).is_err());
    history.cancel_pending_drawing();
    assert_eq!(history.undo_depth(), 2);
    assert_eq!(
        history.begin_drawing_undo().unwrap().transaction,
        TransactionId(2)
    );
    history.finish_drawing().unwrap();
    history.begin_drawing_redo().unwrap();
    assert!(history.record(annotation_record(3, None)).is_err());
    assert!(history.begin_drawing_undo().is_err());
    history.cancel_pending_drawing();
    assert_eq!(
        history.begin_drawing_redo().unwrap().transaction,
        TransactionId(2)
    );
}

#[test]
fn annotation_record_cannot_impersonate_drawing_marker() {
    let mut history = History::default();
    assert!(history
        .record(annotation_record(1, Some(DRAWING_MARKER)))
        .is_err());
    assert!(!history.can_undo());
}

#[test]
fn drawing_budget_eviction_keeps_payloads_in_step() {
    let record = drawing_record(1);
    let budget = record.approx_bytes()
        + drawing_marker(record.transaction, record.label.clone()).approx_bytes();
    let mut history = History::new(budget);
    history.record_drawing(record).unwrap();
    history.record_drawing(drawing_record(2)).unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.drawing_undo.len(), 1);
    assert_eq!(history.used_bytes(), budget);
    assert_eq!(
        history.begin_drawing_undo().unwrap().transaction,
        TransactionId(2)
    );
}
