//! History-step rules for parametric commits (ADR-0043 D8).
//!
//! The ADR deliberately relaxes the exclusivity the BIM batch enforces: a
//! parametric step **may** carry `record.patches`, because the parameter table
//! and the re-derived element rows go through one `apply_parametric_update`
//! revision. It still may not share a step with a BIM patch or a linetype row
//! (two write paths would mean two revisions), and a merge key is refused like
//! every other table batch.

use cad_domain::param::{Expr, ParamBindings, ParamKind, ParamRow, ParamSeq};
use cad_domain::{EntityId, LayerId, ObjectId, Revision, TransactionId};

use crate::{drawing_patch, DrawingHistoryStep, DrawingUndoRecord, History};

fn entity(id: u128) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: cad_domain::SpaceId::Model,
        geometry: cad_domain::SemanticGeometry::Line {
            start: cad_domain::Point3::default(),
            end: cad_domain::Point3 {
                x: 10.0,
                ..cad_domain::Point3::default()
            },
        },
        draw_order: 0,
    }
}

fn param_row(seq: u64) -> ParamRow {
    ParamRow::new(
        ParamSeq(seq),
        format!("p{seq}"),
        ParamKind::Length,
        Expr::num(1.0),
    )
    .expect("row")
}

fn param_patch(seq: u64) -> cad_db::ParamPatch {
    cad_db::ParamPatch {
        seq: ParamSeq(seq),
        before: None,
        after: Some(param_row(seq)),
    }
}

fn binding_patch(id: u128) -> cad_db::ParamBindingPatch {
    cad_db::ParamBindingPatch {
        id: EntityId(id),
        before: None,
        after: Some(
            ParamBindings::from_entries([("radius".to_string(), Expr::reference(ParamSeq(1)))])
                .expect("bindings"),
        ),
        before_geometry: None,
        after_geometry: None,
    }
}

fn bim_element() -> cad_db::BimElement {
    cad_db::BimElement {
        category: None,
        name: None,
        source: cad_db::BimSource {
            connector: "ifc".into(),
            document_id: None,
            id: None,
            native_id: None,
        },
        level: None,
        element_type: None,
        material: None,
        psets: Vec::new(),
        evidence: cad_db::BimEvidence::Declared,
    }
}

fn bim_patch(id: u128) -> cad_db::BimPatch {
    cad_db::BimPatch {
        id: EntityId(id),
        before: bim_element(),
        after: bim_element(),
    }
}

fn linetype_patch() -> cad_db::LinetypePatch {
    cad_db::LinetypePatch {
        id: cad_domain::LinetypeId(1),
        before: None,
        after: Some(cad_db::LineType {
            id: cad_domain::LinetypeId(1),
            name: "DASHED".into(),
            pattern: cad_db::LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        }),
    }
}

fn step() -> DrawingHistoryStep {
    DrawingHistoryStep {
        record: DrawingUndoRecord::entity_only(TransactionId(1), "param", Vec::new()),
        linetype_patches: Vec::new(),
        bim_patches: Vec::new(),
        param_patches: Vec::new(),
        param_binding_patches: Vec::new(),
        group_patches: Vec::new(),
        view_patches: Vec::new(),
        material_patches: Vec::new(),
        scene_patches: Vec::new(),
        display_patches: Vec::new(),
    }
}

fn record_with_entity() -> DrawingUndoRecord {
    DrawingUndoRecord::entity_only(
        TransactionId(1),
        "re-derived geometry",
        vec![drawing_patch(EntityId(1), Some(entity(1)), Some(entity(1)))],
    )
}

#[test]
fn a_parametric_step_cannot_carry_entity_patches() {
    // The replay branch never applies record rows; derived geometry travels on
    // the binding patches. A mixed step would be silently dropped, so refuse it.
    let mut step = step();
    step.record = record_with_entity();
    step.param_patches = vec![param_patch(1)];
    step.param_binding_patches = vec![binding_patch(1)];
    assert!(step.validate().is_err());
}

#[test]
fn a_parametric_step_alone_is_not_empty() {
    let mut step = step();
    step.param_patches = vec![param_patch(1)];
    step.validate()
        .expect("the parameter row is content on its own");
}

#[test]
fn a_binding_only_step_is_not_empty() {
    let mut step = step();
    step.param_binding_patches = vec![binding_patch(1)];
    step.validate()
        .expect("the binding row is content on its own");
}

#[test]
fn a_parametric_step_cannot_share_a_step_with_bim_patches() {
    let mut step = step();
    step.record = record_with_entity();
    step.param_patches = vec![param_patch(1)];
    step.bim_patches = vec![bim_patch(1)];
    assert!(step.validate().is_err(), "two write paths, two revisions");
}

#[test]
fn a_parametric_step_cannot_carry_a_linetype_row() {
    let mut step = step();
    step.param_patches = vec![param_patch(1)];
    step.linetype_patches = vec![linetype_patch()];
    assert!(step.validate().is_err());
}

#[test]
fn a_parametric_step_cannot_be_merged() {
    let mut step = step();
    step.record = record_with_entity();
    step.record.merge_key = Some("drag".into());
    step.param_patches = vec![param_patch(1)];
    assert!(step.validate().is_err());
}

#[test]
fn a_duplicate_parameter_sequence_is_refused() {
    let mut step = step();
    step.param_patches = vec![param_patch(1), param_patch(1)];
    assert!(step.validate().is_err());
}

#[test]
fn a_duplicate_binding_id_is_refused() {
    let mut step = step();
    step.param_binding_patches = vec![binding_patch(1), binding_patch(1)];
    assert!(step.validate().is_err());
}

#[test]
fn a_parametric_patch_without_either_side_is_refused() {
    let mut step = step();
    step.param_patches = vec![cad_db::ParamPatch {
        seq: ParamSeq(1),
        before: None,
        after: None,
    }];
    assert!(step.validate().is_err());
}

#[test]
fn the_recorder_initialises_the_parametric_batches_empty() {
    let mut history = History::default();
    history
        .record_drawing(record_with_entity())
        .expect("a plain entity record still records");
    let step = history.begin_drawing_undo().expect("undo");
    assert!(step.param_patches.is_empty());
    assert!(step.param_binding_patches.is_empty());
    history.cancel_pending_drawing();
}

#[test]
fn a_record_with_no_entity_patches_records_a_param_only_step() {
    let mut history = History::default();
    let record = DrawingUndoRecord::entity_only(TransactionId(2), "param", Vec::new());
    history
        .record_drawing_with_params(record, vec![param_patch(1)], vec![binding_patch(1)])
        .expect("a param-only step is content");
    assert_eq!(history.undo_depth(), 1);
    let step = history.begin_drawing_undo().expect("undo");
    assert_eq!(step.param_patches, vec![param_patch(1)]);
    assert_eq!(step.param_binding_patches, vec![binding_patch(1)]);
    assert!(step.record.patches.is_empty());
    history.cancel_pending_drawing();

    // The recorder runs `validate` first, so a mix with another write path is
    // refused and the retained history is untouched.
    let mut mixed = crate::param_tests::step();
    mixed.param_patches = vec![param_patch(1)];
    mixed.bim_patches = vec![bim_patch(1)];
    assert!(mixed.validate().is_err());
    let mut with_linetype = crate::param_tests::step();
    with_linetype.param_patches = vec![param_patch(1)];
    with_linetype.linetype_patches = vec![linetype_patch()];
    assert!(with_linetype.validate().is_err());
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn a_parametric_step_cannot_carry_another_record_table_batch() {
    // No record batch may ride with a parametric step; a style row is one more
    // example of the same rule.
    let mut step = step();
    step.param_patches = vec![param_patch(1)];
    step.record.style_patches = vec![cad_db::StylePatch {
        id: cad_domain::StyleId(1),
        before: None,
        after: Some(cad_db::Style {
            id: cad_domain::StyleId(1),
            name: "Notes".into(),
            resource_keys: Vec::new(),
        }),
    }];
    assert!(step.validate().is_err());
}
