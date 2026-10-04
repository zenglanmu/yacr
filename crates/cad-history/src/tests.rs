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
fn returning_to_original_state_preserves_the_earlier_annotation_entry() {
    let mut db = database_with();
    let mut history = History::default();
    for (transaction, before, after, merge_key) in [
        (1, "a", "earlier", None),
        (2, "earlier", "temporary", Some("drag")),
        (3, "temporary", "earlier", Some("drag")),
    ] {
        db.apply_annotation_changes(
            "edit",
            TransactionId(transaction),
            vec![(AnnotationId(1), Some(ann(1, after)))],
        )
        .unwrap();
        history
            .record(UndoRecord {
                transaction: TransactionId(transaction),
                label: "edit".into(),
                patches: vec![patch(
                    AnnotationId(1),
                    Some(ann(1, before)),
                    Some(ann(1, after)),
                )],
                merge_key: merge_key.map(str::to_owned),
            })
            .unwrap();
    }
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.used_bytes(), history.undo[0].approx_bytes());
    history.undo(&mut db).unwrap();
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "a");
    history.redo(&mut db).unwrap();
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "earlier");
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
fn coalesced_creation_and_deletion_remove_the_entry() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut history = History::default();
    db.apply_annotation_changes(
        "create",
        TransactionId(1),
        vec![(AnnotationId(5), Some(ann(5, "note")))],
    )
    .unwrap();
    history.record(annotation_record(5, Some("edit"))).unwrap();
    db.apply_annotation_changes("delete", TransactionId(2), vec![(AnnotationId(5), None)])
        .unwrap();
    history
        .record(UndoRecord {
            transaction: TransactionId(2),
            label: "delete".into(),
            patches: vec![patch(AnnotationId(5), Some(ann(5, "note")), None)],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.used_bytes(), 0);
    let revision = db.revision();
    assert!(history.undo(&mut db).is_err());
    assert_eq!(db.revision(), revision);
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
fn canceled_merge_preserves_earlier_drawing_history_and_clears_redo() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    let bytes = history.used_bytes();
    history.record(annotation_record(5, Some("edit"))).unwrap();
    history.record_drawing(drawing_record(2)).unwrap();
    history.begin_drawing_undo().unwrap();
    history.finish_drawing().unwrap();
    assert!(history.next_redo_is_drawing());
    history
        .record(UndoRecord {
            transaction: TransactionId(3),
            label: "delete".into(),
            patches: vec![patch(AnnotationId(5), Some(ann(5, "note")), None)],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.used_bytes(), bytes);
    assert!(history.redo.is_empty());
    assert!(history.redo_markers.is_empty());
    assert!(history.drawing_redo.is_empty());
    assert_eq!(history.drawing_undo.len(), 1);
    assert_eq!(
        history.begin_drawing_undo().unwrap().transaction,
        TransactionId(1)
    );
    history.cancel_pending_drawing();
    assert_eq!(history.used_bytes(), bytes);
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
fn partially_canceled_merge_retains_reversible_patches_and_accounting() {
    let mut db = database_with();
    let mut history = History::default();
    db.apply_annotation_changes(
        "edit",
        TransactionId(1),
        vec![
            (AnnotationId(1), Some(ann(1, "edited"))),
            (AnnotationId(5), Some(ann(5, "note"))),
        ],
    )
    .unwrap();
    history
        .record(UndoRecord {
            transaction: TransactionId(1),
            label: "edit".into(),
            patches: vec![
                patch(AnnotationId(1), Some(ann(1, "a")), Some(ann(1, "edited"))),
                patch(AnnotationId(5), None, Some(ann(5, "note"))),
            ],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    db.apply_annotation_changes("delete", TransactionId(2), vec![(AnnotationId(5), None)])
        .unwrap();
    history
        .record(UndoRecord {
            transaction: TransactionId(2),
            label: "delete".into(),
            patches: vec![patch(AnnotationId(5), Some(ann(5, "note")), None)],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.undo[0].patches.len(), 1);
    assert_eq!(history.used_bytes(), history.undo[0].approx_bytes());
    history.undo(&mut db).unwrap();
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "a");
    assert_eq!(history.used_bytes(), 0);
    history.redo(&mut db).unwrap();
    assert_eq!(db.get(AnnotationId(1)).unwrap().text, "edited");
    assert!(db.get(AnnotationId(5)).is_none());
    assert_eq!(history.used_bytes(), history.undo[0].approx_bytes());
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

#[test]
fn journal_rejects_malformed_records_without_appending() {
    let mut journal = MemoryJournal::new();
    let valid = annotation_record(7, None);
    journal.append(&valid).unwrap();
    let mut empty = valid.clone();
    empty.patches.clear();
    let mut duplicate = valid.clone();
    duplicate.patches.push(duplicate.patches[0].clone());
    let mut wrong_before = valid.clone();
    wrong_before.patches[0].before = Some(ann(99, "wrong"));
    let mut wrong_after = valid.clone();
    wrong_after.patches[0].after = Some(ann(99, "wrong"));
    let mut absent = valid.clone();
    absent.patches[0].after = None;
    let marker = drawing_marker(TransactionId(8), "drawing".into());
    for record in [empty, duplicate, wrong_before, wrong_after, absent, marker] {
        assert!(matches!(
            journal.append(&record),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(journal.len(), 1);
    }
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    assert_eq!(journal.recover(&mut db).unwrap().len(), 1);
    assert_eq!(db.get(AnnotationId(7)), valid.patches[0].after.as_ref());
}

#[test]
fn failed_journal_recovery_preserves_exported_state_and_can_retry() {
    let mut db = database_with();
    db.mark_exported(db.revision()).unwrap();
    let revision = db.revision();
    let annotations: Vec<_> = db.annotations().cloned().collect();
    let mut journal = MemoryJournal::new();
    journal.append(&annotation_record(7, None)).unwrap();
    journal
        .append(&UndoRecord {
            transaction: TransactionId(2),
            label: "delete".into(),
            patches: vec![patch(AnnotationId(8), Some(ann(8, "note")), None)],
            merge_key: None,
        })
        .unwrap();
    assert!(journal.recover(&mut db).is_err());
    assert_eq!(db.id(), DatabaseId(1));
    assert_eq!(db.revision(), revision);
    assert!(!db.is_dirty());
    assert_eq!(db.annotations().cloned().collect::<Vec<_>>(), annotations);
    assert_eq!(journal.len(), 2);

    db.apply_annotation_changes(
        "restore missing annotation",
        TransactionId(3),
        vec![(AnnotationId(8), Some(ann(8, "note")))],
    )
    .unwrap();
    let before = db.revision();
    let sets = journal.recover(&mut db).unwrap();
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].before, before);
    assert_eq!(sets[0].after, sets[1].before);
    assert_eq!(sets[1].after, db.revision());
    assert_eq!(sets[0].transaction, TransactionId(1));
    assert_eq!(sets[1].transaction, TransactionId(2));
    assert_eq!(db.get(AnnotationId(7)), Some(&ann(7, "note")));
    assert!(db.get(AnnotationId(8)).is_none());
    assert!(db.is_dirty());
}

#[test]
fn invalid_later_journal_geometry_does_not_publish_earlier_update() {
    let mut db = database_with();
    let revision = db.revision();
    let mut journal = MemoryJournal::new();
    journal
        .append(&UndoRecord {
            transaction: TransactionId(1),
            label: "update".into(),
            patches: vec![patch(
                AnnotationId(1),
                Some(ann(1, "a")),
                Some(ann(1, "changed")),
            )],
            merge_key: None,
        })
        .unwrap();
    let mut invalid = annotation_record(7, None);
    invalid.patches[0].after.as_mut().unwrap().geometry = AnnotationGeometry::Text(Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    });
    journal.append(&invalid).unwrap();
    assert!(journal.recover(&mut db).is_err());
    assert_eq!(db.revision(), revision);
    assert_eq!(db.get(AnnotationId(1)), Some(&ann(1, "a")));
    assert_eq!(db.len(), 1);
    assert!(db.is_dirty());
    assert_eq!(journal.len(), 2);
}

#[test]
fn journal_replays_sequential_states_and_empty_recovery_is_unchanged() {
    let mut db = database_with();
    db.mark_exported(db.revision()).unwrap();
    let revision = db.revision();
    let mut journal = MemoryJournal::new();
    assert!(journal.recover(&mut db).unwrap().is_empty());
    assert_eq!(db.revision(), revision);
    assert!(!db.is_dirty());
    journal.append(&annotation_record(7, None)).unwrap();
    journal
        .append(&UndoRecord {
            transaction: TransactionId(2),
            label: "delete recovered annotation".into(),
            patches: vec![patch(AnnotationId(7), Some(ann(7, "note")), None)],
            merge_key: None,
        })
        .unwrap();
    let sets = journal.recover(&mut db).unwrap();
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].database, db.id());
    assert_eq!(sets[1].database, db.id());
    assert_eq!(sets[0].before, revision);
    assert_eq!(sets[0].after, Revision(revision.0 + 1));
    assert_eq!(sets[1].before, sets[0].after);
    assert_eq!(sets[1].after, Revision(revision.0 + 2));
    assert_eq!(db.revision(), sets[1].after);
    assert_eq!(db.get(AnnotationId(1)), Some(&ann(1, "a")));
    assert!(db.get(AnnotationId(7)).is_none());
    assert!(db.is_dirty());
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
fn malformed_annotation_records_are_rejected_atomically() {
    let mut history = History::default();
    history.record(annotation_record(1, Some("edit"))).unwrap();
    history.record_drawing(drawing_record(2)).unwrap();
    history.begin_drawing_undo().unwrap();
    history.finish_drawing().unwrap();
    let bytes = history.used_bytes();
    let mut duplicate = annotation_record(1, Some("edit"));
    duplicate.patches.push(duplicate.patches[0].clone());
    let mut wrong_before = annotation_record(1, Some("edit"));
    wrong_before.patches[0].before = Some(ann(99, "wrong"));
    let mut wrong_after = annotation_record(1, Some("edit"));
    wrong_after.patches[0].after = Some(ann(99, "wrong"));
    let mut absent = annotation_record(1, Some("edit"));
    absent.patches[0].after = None;
    for record in [duplicate, wrong_before, wrong_after, absent] {
        assert!(matches!(
            history.record(record),
            Err(CadError::InvalidInput(_))
        ));
    }
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 1);
    assert_eq!(history.used_bytes(), bytes);
    assert!(history.next_redo_is_drawing());
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

#[test]
fn malformed_drawing_records_are_rejected_atomically() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    history.record_drawing(drawing_record(2)).unwrap();
    history.begin_drawing_undo().unwrap();
    history.finish_drawing().unwrap();
    let bytes = history.used_bytes();
    let mut duplicate = drawing_record(3);
    duplicate.patches.push(duplicate.patches[0].clone());
    let mut wrong_before = drawing_record(3);
    wrong_before.patches[0].before = Some(entity(99, 0.0));
    let mut wrong_after = drawing_record(3);
    wrong_after.patches[0].after = Some(entity(99, 0.0));
    let mut wrong_object_before = drawing_record(3);
    let mut wrong = entity(3, 0.0);
    wrong.object.id = ObjectId(99);
    wrong_object_before.patches[0].before = Some(wrong.clone());
    let mut wrong_object_after = drawing_record(3);
    wrong_object_after.patches[0].after = Some(wrong);
    let mut absent = drawing_record(3);
    absent.patches[0].after = None;
    for record in [
        duplicate,
        wrong_before,
        wrong_after,
        wrong_object_before,
        wrong_object_after,
        absent,
    ] {
        assert!(matches!(
            history.record_drawing(record),
            Err(CadError::InvalidInput(_))
        ));
    }
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 1);
    assert_eq!(history.used_bytes(), bytes);
    assert_eq!(history.drawing_undo.len(), 1);
    assert_eq!(history.drawing_redo.len(), 1);
    assert!(history.next_redo_is_drawing());
}
