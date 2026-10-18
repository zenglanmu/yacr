use super::*;
use crate::{host::HostController, AppMode, CommandPayload, DrawToolKind, ToolState};
use cad_db::{BlockDefinition, DrawingDatabaseBuilder, Layer};
use cad_domain::{BlockId, InstancePath, TransactionId};

fn core() -> HostController {
    HostController::with_demo_document([800.0, 600.0]).unwrap()
}

fn target(core: &HostController) -> ApiTarget {
    ApiTarget {
        document: core.document_id,
        database: core.drawing().unwrap().id(),
        viewport: core.viewport_id,
    }
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn top_ref(core: &HostController, id: u128) -> SelectionRef {
    SelectionRef {
        document: core.document_id,
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn insert_line(start: Point3, end: Point3) -> EntityEdit {
    EntityEdit::Insert {
        layer: LayerId(0),
        space: SpaceId::Model,
        entity: NewEntity::Line { start, end },
    }
}

fn commit(core: &mut HostController, edits: Vec<EntityEdit>) -> CadResult<ApiOutcome> {
    let target = target(core);
    let revision = core.drawing().unwrap().revision();
    core.application.execute_api(
        &mut core.session,
        target,
        ApiRequest::Commit {
            expected_revision: revision,
            edits,
        },
    )
}

fn prepare_pending_undo_with_redo(core: &mut HostController) {
    core.create_line(point(0.0, 0.0), point(1.0, 0.0)).unwrap();
    core.create_line(point(2.0, 0.0), point(3.0, 0.0)).unwrap();
    core.execute(Command {
        schema_version: 1,
        id: CommandId::Undo,
        document: core.document_id,
        viewport: core.viewport_id,
        payload: CommandPayload::None,
    })
    .unwrap();
    core.application
        .history
        .get_mut(&core.document_id)
        .unwrap()
        .begin_drawing_undo()
        .unwrap();
}

#[test]
fn batch_commit_is_one_transaction_and_one_undo_redo_step() {
    let mut core = core();
    let old_revision = core.drawing().unwrap().revision();
    let outcome = commit(
        &mut core,
        vec![
            insert_line(point(0.0, 0.0), point(2.0, 0.0)),
            EntityEdit::Insert {
                layer: LayerId(1),
                space: SpaceId::Model,
                entity: NewEntity::Circle {
                    center: point(4.0, 0.0),
                    radius: 1.0,
                },
            },
        ],
    )
    .unwrap();
    let changes = outcome.changes.unwrap();
    assert_eq!(changes.before, old_revision);
    assert_eq!(changes.after.0, old_revision.0 + 1);
    assert_eq!(outcome.created.len(), 2);
    assert_eq!(core.application.history[&core.document_id].undo_depth(), 1);
    core.execute(Command {
        schema_version: 1,
        id: CommandId::Undo,
        document: core.document_id,
        viewport: core.viewport_id,
        payload: CommandPayload::None,
    })
    .unwrap();
    assert!(outcome.created.iter().all(|id| core
        .drawing()
        .unwrap()
        .entity(EntityId(id.0))
        .is_none()));
    core.execute(Command {
        schema_version: 1,
        id: CommandId::Redo,
        document: core.document_id,
        viewport: core.viewport_id,
        payload: CommandPayload::None,
    })
    .unwrap();
    assert!(outcome.created.iter().all(|id| core
        .drawing()
        .unwrap()
        .entity(EntityId(id.0))
        .is_some()));
}

#[test]
fn failed_late_edit_keeps_authoritative_db_allocator_history_and_redo_unchanged() {
    let mut core = core();
    let text_id = {
        let document = core
            .application
            .workspace
            .documents
            .get_mut(&core.document_id)
            .unwrap();
        let mut candidate = document.drawing.as_ref().clone();
        let id = candidate.allocate_entity_id();
        let geometry = SemanticGeometry::Opaque {
            type_key: "AcDbOpaque".into(),
            version: 1,
            payload: vec![1],
        };
        let entity = DbEntity {
            object: DbObject {
                id: ObjectId(id.0),
                type_key: "AcDbOpaque".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id,
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order: 20,
        };
        candidate
            .apply_drawing_changes("test fixture", TransactionId(700), vec![(id, Some(entity))])
            .unwrap();
        document.drawing = Arc::new(candidate);
        id
    };
    let first = commit(
        &mut core,
        vec![insert_line(point(0.0, 0.0), point(1.0, 0.0))],
    )
    .unwrap();
    let _second = commit(
        &mut core,
        vec![insert_line(point(2.0, 0.0), point(3.0, 0.0))],
    )
    .unwrap();
    core.execute(Command {
        schema_version: 1,
        id: CommandId::Undo,
        document: core.document_id,
        viewport: core.viewport_id,
        payload: CommandPayload::None,
    })
    .unwrap();
    {
        let history = core.application.history.get_mut(&core.document_id).unwrap();
        history.begin_drawing_undo().unwrap();
    }
    let before = core.drawing().unwrap();
    let allocator = before.next_entity_id();
    let history_before = {
        let history = core.application.history.get(&core.document_id).unwrap();
        (history.undo_depth(), history.redo_depth())
    };
    let refs = [
        EntityEdit::Insert {
            layer: LayerId(0),
            space: SpaceId::Model,
            entity: NewEntity::Line {
                start: point(2.0, 0.0),
                end: point(3.0, 0.0),
            },
        },
        EntityEdit::Translate {
            reference: top_ref(&core, text_id.0),
            delta: point(1.0, 0.0),
        },
    ];
    assert!(matches!(
        commit(&mut core, refs.to_vec()),
        Err(CadError::Unsupported(_))
    ));
    assert!(matches!(
        commit(
            &mut core,
            vec![insert_line(point(5.0, 0.0), point(6.0, 0.0))]
        ),
        Err(CadError::InvalidInput(_))
    ));
    let after = core.drawing().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(after.next_entity_id(), allocator);
    let history = core.application.history.get(&core.document_id).unwrap();
    assert_eq!((history.undo_depth(), history.redo_depth()), history_before);
    assert_eq!(history.redo_depth(), 1);
    core.application
        .history
        .get_mut(&core.document_id)
        .unwrap()
        .cancel_pending_drawing();
    assert!(after.entity(EntityId(first.created[0].0)).is_some());
}

#[test]
fn empty_and_zero_translation_do_not_bump_revision_or_history() {
    let mut core = core();
    let revision = core.drawing().unwrap().revision();
    let depth = core
        .application
        .history
        .get(&core.document_id)
        .map_or(0, |h| h.undo_depth());
    assert!(commit(&mut core, Vec::new()).unwrap().changes.is_none());
    let no_op = EntityEdit::Translate {
        reference: top_ref(&core, 1),
        delta: point(0.0, -0.0),
    };
    assert!(commit(&mut core, vec![no_op]).unwrap().changes.is_none());
    assert_eq!(core.drawing().unwrap().revision(), revision);
    assert_eq!(
        core.application
            .history
            .get(&core.document_id)
            .map_or(0, |h| h.undo_depth()),
        depth
    );
}

#[test]
fn stale_identity_revision_and_viewer_write_are_refused() {
    let mut core = core();
    let target = target(&core);
    let revision = core.drawing().unwrap().revision();
    let commit_request = || ApiRequest::Commit {
        expected_revision: revision,
        edits: vec![insert_line(point(0.0, 0.0), point(1.0, 0.0))],
    };
    let mut wrong_db = target;
    wrong_db.database = DatabaseId(999);
    assert!(matches!(
        core.application
            .execute_api(&mut core.session, wrong_db, commit_request()),
        Err(CadError::StaleResult)
    ));
    assert!(matches!(
        core.application.execute_api(
            &mut core.session,
            target,
            ApiRequest::Commit {
                expected_revision: Revision(revision.0 + 1),
                edits: vec![insert_line(point(0.0, 0.0), point(1.0, 0.0))]
            }
        ),
        Err(CadError::StaleResult)
    ));
    core.session.switch_mode(AppMode::Viewer).unwrap();
    assert_eq!(
        core.application
            .execute_api(&mut core.session, target, commit_request())
            .unwrap_err(),
        CadError::PermissionDenied
    );
}

#[test]
fn selection_validates_entire_batch_and_toggle_deduplicates_once() {
    let mut core = core();
    let reference = top_ref(&core, 1);
    let invalid = top_ref(&core, 9999);
    let target = target(&core);
    let old_generation = core.session.tool_generation;
    assert!(core
        .application
        .execute_api(
            &mut core.session,
            target,
            ApiRequest::SelectByRefs {
                refs: vec![reference.clone(), invalid],
                mode: SelectionMode::Replace,
            }
        )
        .is_err());
    assert!(core.session.selection.is_empty());
    assert_eq!(core.session.tool_generation, old_generation);
    core.application
        .execute_api(
            &mut core.session,
            target,
            ApiRequest::SelectByRefs {
                refs: vec![reference.clone(), reference.clone()],
                mode: SelectionMode::Toggle,
            },
        )
        .unwrap();
    assert_eq!(core.session.selection.refs(), &[reference]);
}

#[test]
fn valid_block_instance_reference_selects_but_object_id_leaf_is_rejected() {
    let mut core = core();
    let document_id = core.document_id;
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(41));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(20)],
            dynamic_visibility: None,
        })
        .unwrap();
    let member_geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(1.0, 0.0),
    };
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(20),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(20),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(2)),
            geometry: member_geometry,
            draw_order: 0,
        })
        .unwrap();
    let insert_geometry = SemanticGeometry::Insert {
        block: BlockId(2),
        transform: Transform3::identity(),
    };
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(10),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(10),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: insert_geometry,
            draw_order: 0,
        })
        .unwrap();
    let drawing = Arc::new(builder.finish().unwrap());
    core.application
        .workspace
        .documents
        .get_mut(&document_id)
        .unwrap()
        .drawing = drawing;
    let target = target(&core);
    let nested = SelectionRef {
        document: document_id,
        entity: EntityId(20),
        instance: InstancePath(vec![EntityId(10)]),
        sub_element: None,
    };
    core.application
        .execute_api(
            &mut core.session,
            target,
            ApiRequest::SelectByRefs {
                refs: vec![nested.clone()],
                mode: SelectionMode::Replace,
            },
        )
        .unwrap();
    assert_eq!(core.session.selection.refs(), &[nested]);
    assert!(core
        .application
        .execute_api(
            &mut core.session,
            target,
            ApiRequest::SelectByObjectIds {
                ids: vec![ObjectId(20)],
                mode: SelectionMode::Replace
            }
        )
        .is_err());
}

#[test]
fn command_trace_shares_arc_and_failed_draw_confirmation_keeps_capture() {
    let mut core = core();
    let traced = core
        .execute_traced(Command {
            schema_version: 1,
            id: CommandId::FitDrawing,
            document: core.document_id,
            viewport: core.viewport_id,
            payload: CommandPayload::None,
        })
        .unwrap();
    assert_eq!(traced.commands.len(), 1);
    assert!(Arc::ptr_eq(
        traced.result.as_ref().unwrap(),
        traced.commands[0].result.as_ref().unwrap()
    ));

    core.begin_draw_tool(DrawToolKind::Line).unwrap();
    core.capture_tool_point(point(1.0, 1.0)).unwrap();
    core.capture_tool_point(point(1.0, 1.0)).unwrap();
    let failed = core.confirm_draw_traced().unwrap();
    assert!(failed.result.is_err());
    assert_eq!(failed.commands.len(), 1);
    assert!(matches!(&core.session.tool, ToolState::Drawing(_)));
}

#[test]
fn traced_create_line_history_rejection_preserves_database_allocator_and_redo() {
    let mut core = core();
    prepare_pending_undo_with_redo(&mut core);
    let before = core.drawing().unwrap();
    let allocator = before.next_entity_id();
    let history_before = {
        let history = core.application.history.get(&core.document_id).unwrap();
        (history.undo_depth(), history.redo_depth())
    };
    let trace = core
        .execute_traced(Command {
            schema_version: 1,
            id: CommandId::CreateLine,
            document: core.document_id,
            viewport: core.viewport_id,
            payload: CommandPayload::Points(vec![point(5.0, 0.0), point(6.0, 0.0)]),
        })
        .unwrap();
    assert!(trace.result.is_err());
    assert_eq!(trace.commands.len(), 1);
    assert!(trace.commands[0].result.is_err());
    let after = core.drawing().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(after.next_entity_id(), allocator);
    let history = core.application.history.get(&core.document_id).unwrap();
    assert_eq!((history.undo_depth(), history.redo_depth()), history_before);
    core.application
        .history
        .get_mut(&core.document_id)
        .unwrap()
        .cancel_pending_drawing();
}

#[test]
fn traced_confirm_history_rejection_preserves_capture_database_allocator_and_redo() {
    let mut core = core();
    prepare_pending_undo_with_redo(&mut core);
    core.begin_draw_tool(DrawToolKind::Line).unwrap();
    core.capture_tool_point(point(5.0, 0.0)).unwrap();
    core.capture_tool_point(point(6.0, 0.0)).unwrap();
    let intent_before = core.session.draw_intent().unwrap();
    let generation_before = core.session.tool_generation;
    let before = core.drawing().unwrap();
    let allocator = before.next_entity_id();
    let history_before = {
        let history = core.application.history.get(&core.document_id).unwrap();
        (history.undo_depth(), history.redo_depth())
    };
    let trace = core.confirm_draw_traced().unwrap();
    assert!(trace.result.is_err());
    assert_eq!(trace.commands.len(), 1);
    assert!(trace.commands[0].result.is_err());
    assert_eq!(core.session.draw_intent().unwrap(), intent_before);
    assert_eq!(core.session.tool_generation, generation_before);
    let after = core.drawing().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(after.next_entity_id(), allocator);
    let history = core.application.history.get(&core.document_id).unwrap();
    assert_eq!((history.undo_depth(), history.redo_depth()), history_before);
    core.application
        .history
        .get_mut(&core.document_id)
        .unwrap()
        .cancel_pending_drawing();
}
