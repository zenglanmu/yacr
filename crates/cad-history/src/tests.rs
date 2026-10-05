//! Unit tests.

use super::*;

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

fn drawing_record(id: u128) -> DrawingUndoRecord {
    DrawingUndoRecord {
        transaction: TransactionId(id),
        label: "create line".into(),
        patches: vec![drawing_patch(EntityId(id), None, Some(entity(id, 0.0)))],
        merge_key: None,
    }
}

#[test]
fn create_then_undo_moves_to_redo_and_redo_restores() {
    let mut history = History::default();
    history.record_drawing(drawing_record(5)).unwrap();
    assert!(history.can_undo());
    assert!(!history.can_redo());

    let record = history.begin_drawing_undo().unwrap();
    assert_eq!(record.patches[0].id, EntityId(5));
    assert_eq!(history.undo_depth(), 0);
    assert!(!history.can_undo());
    history.finish_drawing().unwrap();
    assert!(history.can_redo());

    let redone = history.begin_drawing_redo().unwrap();
    assert_eq!(redone.transaction, TransactionId(5));
    history.finish_drawing().unwrap();
    assert!(history.can_undo());
    assert!(!history.can_redo());
}

#[test]
fn cancelled_undo_restores_the_step() {
    let mut history = History::default();
    history.record_drawing(drawing_record(3)).unwrap();
    let _ = history.begin_drawing_undo().unwrap();
    assert_eq!(history.undo_depth(), 0);
    history.cancel_pending_drawing();
    assert_eq!(history.undo_depth(), 1, "the rejected step is not lost");
    assert!(history.can_undo());
}

#[test]
fn merge_key_coalesces_records() {
    let mut history = History::default();
    let r = |t: f64| DrawingUndoRecord {
        transaction: TransactionId(1),
        label: "drag".into(),
        patches: vec![drawing_patch(
            EntityId(1),
            Some(entity(1, 0.0)),
            Some(entity(1, t)),
        )],
        merge_key: Some("drag-1".into()),
    };
    history.record_drawing(r(1.0)).unwrap();
    history.record_drawing(r(2.0)).unwrap();
    assert_eq!(history.undo_depth(), 1);
    // The merged record keeps the earliest before and latest after.
    let back = history.begin_drawing_undo().unwrap();
    assert_eq!(back.patches[0].before, Some(entity(1, 0.0)));
    assert_eq!(back.patches[0].after, Some(entity(1, 2.0)));
}

#[test]
fn coalesced_creation_and_deletion_remove_the_entry() {
    let mut history = History::default();
    history
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(1),
            label: "edit".into(),
            patches: vec![drawing_patch(EntityId(5), None, Some(entity(5, 1.0)))],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    history
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(2),
            label: "delete".into(),
            patches: vec![drawing_patch(EntityId(5), Some(entity(5, 1.0)), None)],
            merge_key: Some("edit".into()),
        })
        .unwrap();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.used_bytes(), 0);
    assert!(history.begin_drawing_undo().is_err());
}

#[test]
fn empty_record_is_rejected() {
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

#[test]
fn budget_evicts_oldest() {
    let mut history = History::new(1);
    for i in 0..5 {
        history.record_drawing(drawing_record(i)).unwrap();
    }
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn merged_record_enforces_budget() {
    let first = drawing_record(1);
    let mut second = drawing_record(2);
    second.merge_key = Some("drag".into());
    let budget = first.approx_bytes() + second.approx_bytes();
    let mut history = History::new(budget);
    history.record_drawing(first).unwrap();
    history.record_drawing(second).unwrap();
    let mut larger = drawing_record(2);
    larger.merge_key = Some("drag".into());
    // A growing entity payload expands the retained record past the budget; the
    // merge must then evict the oldest entry so the newest step is kept.
    let mut big = entity(2, 0.0);
    big.object.type_key = "X".repeat(budget);
    larger.patches[0].after = Some(big);
    history.record_drawing(larger).unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn record_size_is_independent_of_identifier_value() {
    assert_eq!(
        drawing_record(1).approx_bytes(),
        drawing_record(u128::MAX).approx_bytes()
    );
}

#[test]
fn accounting_survives_undo_redo_and_cancel() {
    let mut history = History::default();
    history.record_drawing(drawing_record(42)).unwrap();
    let bytes = history.used_bytes();
    assert_eq!(bytes, history.undo[0].approx_bytes());
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
    assert!(history.begin_drawing_undo().is_err());
    assert!(history.begin_drawing_redo().is_err());
    assert!(history.record_drawing(drawing_record(3)).is_err());
    history.cancel_pending_drawing();
    assert_eq!(history.undo_depth(), 2);
    assert_eq!(
        history.begin_drawing_undo().unwrap().transaction,
        TransactionId(2)
    );
    history.finish_drawing().unwrap();
    history.begin_drawing_redo().unwrap();
    assert!(history.begin_drawing_undo().is_err());
    history.cancel_pending_drawing();
    assert_eq!(
        history.begin_drawing_redo().unwrap().transaction,
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
}

#[test]
fn clear_discards_everything() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    history.begin_drawing_undo().unwrap();
    history.finish_drawing().unwrap();
    history.clear();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.used_bytes(), 0);
}
