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
        block_patches: Vec::new(),
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
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
        block_patches: Vec::new(),
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
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
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
            merge_key: Some("edit".into()),
        })
        .unwrap();
    history
        .record_drawing(DrawingUndoRecord {
            transaction: TransactionId(2),
            label: "delete".into(),
            patches: vec![drawing_patch(EntityId(5), Some(entity(5, 1.0)), None)],
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            layout_patches: Vec::new(),
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
        block_patches: Vec::new(),
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
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

fn block_row(id: u128, name: &str, members: &[u128]) -> cad_db::BlockRow {
    cad_db::BlockRow {
        definition: cad_db::BlockDefinition {
            id: BlockId(id),
            entities: members.iter().copied().map(EntityId).collect(),
            dynamic_visibility: None,
        },
        metadata: Some(cad_db::BlockMetadata {
            name: name.into(),
            base_point: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        }),
    }
}

fn block_patch(
    id: u128,
    before: Option<cad_db::BlockRow>,
    after: Option<cad_db::BlockRow>,
) -> cad_db::BlockPatch {
    cad_db::BlockPatch {
        id: BlockId(id),
        before,
        after,
    }
}

/// A block-only record is a complete undo step: the definition row (and its
/// metadata) is the whole edit, so no entity patch is required.
#[test]
fn block_only_record_is_accepted_and_round_trips() {
    let mut history = History::default();
    let record = DrawingUndoRecord {
        transaction: TransactionId(9),
        label: "create block".into(),
        patches: Vec::new(),
        block_patches: vec![block_patch(3, None, Some(block_row(3, "B1", &[1, 2])))],
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
        merge_key: None,
    };
    history.record_drawing(record.clone()).unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert!(history.used_bytes() > 0);

    let back = history.begin_drawing_undo().unwrap();
    assert_eq!(back, record, "the block patch is retained verbatim");
    history.finish_drawing().unwrap();
    let forward = history.begin_drawing_redo().unwrap();
    assert_eq!(forward, record);
    history.finish_drawing().unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn block_patch_without_a_before_or_after_state_is_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.block_patches = vec![block_patch(3, None, None)];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

#[test]
fn duplicate_block_ids_are_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.block_patches = vec![
        block_patch(3, None, Some(block_row(3, "B1", &[1]))),
        block_patch(3, None, Some(block_row(3, "B2", &[2]))),
    ];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

/// Block changes are never coalesced: merging a definition into an earlier
/// record could silently drop it, so a merge key is refused outright and the
/// existing history stays byte-for-byte unchanged.
#[test]
fn merge_key_with_block_patches_is_refused_without_touching_history() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    let bytes = history.used_bytes();

    let mut merged = drawing_record(2);
    merged.merge_key = Some("drag".into());
    merged.block_patches = vec![block_patch(3, None, Some(block_row(3, "B1", &[1])))];

    assert!(matches!(
        history.record_drawing(merged.clone()),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(history.undo_depth(), 1, "no record was merged or added");
    assert_eq!(history.used_bytes(), bytes);
    assert!(!history.can_redo());

    // A block-only merged record is refused by the same rule.
    let block_only = DrawingUndoRecord {
        transaction: TransactionId(4),
        label: "block".into(),
        patches: Vec::new(),
        block_patches: vec![block_patch(3, None, Some(block_row(3, "B1", &[1])))],
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
        merge_key: Some("drag".into()),
    };
    assert!(matches!(
        history.record_drawing(block_only),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn approx_bytes_counts_block_rows_members_and_metadata() {
    let plain = drawing_record(1);
    let mut with_block = drawing_record(1);
    with_block.block_patches = vec![block_patch(3, None, Some(block_row(3, "B1", &[1])))];
    assert!(
        with_block.approx_bytes() > plain.approx_bytes(),
        "a retained block row must be accounted for"
    );

    let mut more_members = with_block.clone();
    more_members.block_patches[0]
        .after
        .as_mut()
        .unwrap()
        .definition
        .entities
        .extend([EntityId(2), EntityId(3)]);
    assert!(more_members.approx_bytes() > with_block.approx_bytes());

    let mut longer_name = with_block.clone();
    longer_name.block_patches[0]
        .after
        .as_mut()
        .unwrap()
        .metadata
        .as_mut()
        .unwrap()
        .name = "B".repeat(128);
    assert!(longer_name.approx_bytes() > with_block.approx_bytes());
}

fn layer_row(id: u128, name: &str, frozen: bool) -> cad_db::LayerRow {
    cad_db::LayerRow {
        layer: cad_db::Layer {
            id: LayerId(id),
            name: name.into(),
            visible: !frozen,
        },
        attributes: cad_db::LayerAttributes {
            frozen,
            ..Default::default()
        },
    }
}

/// A layer-only record is a complete undo step: the row plus its attributes is
/// the whole edit, so no entity patch is required.
#[test]
fn layer_only_record_is_accepted_and_round_trips() {
    let mut history = History::default();
    let record = DrawingUndoRecord {
        transaction: TransactionId(12),
        label: "freeze layer".into(),
        patches: Vec::new(),
        block_patches: Vec::new(),
        layer_patches: vec![layer_patch(
            LayerId(3),
            Some(layer_row(3, "WALLS", false)),
            Some(layer_row(3, "WALLS", true)),
        )],
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: Vec::new(),
        merge_key: None,
    };
    history.record_drawing(record.clone()).unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert!(
        history.used_bytes() > std::mem::size_of::<DrawingUndoRecord>(),
        "the retained layer rows are accounted for"
    );

    let back = history.begin_drawing_undo().unwrap();
    assert_eq!(back, record, "the layer patch is retained verbatim");
    history.finish_drawing().unwrap();
    let forward = history.begin_drawing_redo().unwrap();
    assert_eq!(forward, record);
    history.finish_drawing().unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn layer_patch_without_a_before_or_after_state_is_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.layer_patches = vec![layer_patch(LayerId(3), None, None)];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

#[test]
fn duplicate_layer_ids_are_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.layer_patches = vec![
        layer_patch(LayerId(3), None, Some(layer_row(3, "A", false))),
        layer_patch(LayerId(3), None, Some(layer_row(3, "B", false))),
    ];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

/// Layer rows are never coalesced: merging an attribute write into an earlier
/// record could silently drop or reorder it, so a merge key is refused outright
/// and the existing history stays byte-for-byte unchanged.
#[test]
fn merge_key_with_layer_patches_is_refused_without_touching_history() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    let bytes = history.used_bytes();

    let mut merged = drawing_record(2);
    merged.merge_key = Some("drag".into());
    merged.layer_patches = vec![layer_patch(
        LayerId(3),
        Some(layer_row(3, "WALLS", false)),
        Some(layer_row(3, "WALLS", true)),
    )];

    assert!(matches!(
        history.record_drawing(merged),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(history.undo_depth(), 1, "no record was merged or added");
    assert_eq!(history.used_bytes(), bytes);
    assert!(!history.can_redo());
}

/// A layout row carrying one plot settings record, keyed to the row id.
fn layout_table_row(id: u128, name: &str, paper: &str) -> cad_db::LayoutRow {
    cad_db::LayoutRow {
        layout: cad_db::Layout {
            id: LayoutId(id),
            name: name.into(),
            viewports: Vec::new(),
        },
        plot: Some(cad_db::PlotSettingsRecord {
            layout: LayoutId(id),
            paper_size_name: paper.into(),
            paper_width: 210.0,
            paper_height: 297.0,
            margins: cad_db::PlotMargins::uniform(5.0),
            rotation: cad_db::PlotRotation::None,
            scale_numerator: 1.0,
            scale_denominator: 1.0,
            plot_type: cad_db::PlotType::Layout,
            paper_units: cad_db::PlotPaperUnits::Millimeters,
            provenance: cad_db::PlotProvenance::Imported,
        }),
    }
}

/// A layout-only record is a complete undo step: the row plus its plot settings
/// is the whole edit, so no entity patch is required, and its bytes are counted.
#[test]
fn layout_only_record_is_accepted_and_round_trips() {
    let mut history = History::default();
    let record = DrawingUndoRecord {
        transaction: TransactionId(30),
        label: "create layout".into(),
        patches: Vec::new(),
        block_patches: Vec::new(),
        layer_patches: Vec::new(),
        style_patches: Vec::new(),
        dim_style_patches: Vec::new(),
        mleader_style_patches: Vec::new(),
        layout_patches: vec![layout_patch(
            LayoutId(7),
            None,
            Some(layout_table_row(7, "SheetLayout", "ISO_A4")),
        )],
        merge_key: None,
    };
    assert!(
        record.approx_bytes()
            > std::mem::size_of::<DrawingUndoRecord>()
                + std::mem::size_of::<cad_db::LayoutRow>()
                + "SheetLayout".len()
                + "ISO_A4".len(),
        "the layout name and paper size are accounted for"
    );
    history.record_drawing(record.clone()).unwrap();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.used_bytes(), record.approx_bytes());

    let back = history.begin_drawing_undo().unwrap();
    assert_eq!(back, record, "the layout patch is retained verbatim");
    history.finish_drawing().unwrap();
    let forward = history.begin_drawing_redo().unwrap();
    assert_eq!(forward, record);
    history.finish_drawing().unwrap();
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn layout_patch_without_a_before_or_after_state_is_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.layout_patches = vec![layout_patch(LayoutId(7), None, None)];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

#[test]
fn duplicate_layout_ids_are_rejected() {
    let mut history = History::default();
    let mut record = drawing_record(1);
    record.layout_patches = vec![
        layout_patch(LayoutId(7), None, Some(layout_table_row(7, "A", "ISO_A4"))),
        layout_patch(LayoutId(7), None, Some(layout_table_row(7, "B", "ISO_A3"))),
    ];
    assert!(matches!(
        history.record_drawing(record),
        Err(CadError::InvalidInput(_))
    ));
    assert!(!history.can_undo());
}

/// Layout rows are never coalesced: merging a layout write into an earlier
/// record could silently drop or reorder it, so a merge key is refused outright
/// and the existing history stays byte-for-byte unchanged.
#[test]
fn merge_key_with_layout_patches_is_refused_without_touching_history() {
    let mut history = History::default();
    history.record_drawing(drawing_record(1)).unwrap();
    let bytes = history.used_bytes();

    let mut merged = drawing_record(2);
    merged.merge_key = Some("drag".into());
    merged.layout_patches = vec![layout_patch(
        LayoutId(7),
        Some(layout_table_row(7, "Sheet", "ISO_A4")),
        Some(layout_table_row(7, "Sheet A", "ISO_A4")),
    )];

    assert!(matches!(
        history.record_drawing(merged),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(history.undo_depth(), 1, "no record was merged or added");
    assert_eq!(history.used_bytes(), bytes);
    assert!(!history.can_redo());
}
