//! Write-back contracts for `REFCLOSE SAVE` and the `REFSET` work-set semantics.
//!
//! Split out of `tests/refedit.rs` so both files stay inside the 1000-line code
//! gate. The shared fixtures and helpers live in the parent module; `use
//! super::*` is the same crate-root prelude those fixtures use.

use super::*;

#[test]
fn refclose_save_writes_back_member_layer_and_draw_order() {
    let (mut app, mut session) = two_member_app();
    let clones = begin(&mut app, &mut session);

    // Layer: move the clone of member 100 to WALLS.
    select(&mut session, &[clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: "layer".into(),
            value: "WALLS".into(),
        },
    )
    .expect("CHPROP moves the clone to WALLS");
    assert_eq!(
        drawing_of(&app).entity(EntityId(clones[0])).unwrap().layer,
        LayerId(3)
    );

    // Draw order: make clone 101 the reference and move clone 100 above it, so
    // the model-space relative order is swapped.
    select(&mut session, &[clones[1], clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        CommandPayload::Draw2(Box::new(Draw2Payload::DrawOrder(DrawOrderMode::Above))),
    )
    .expect("DRAWORDER ABOVE swaps the clones");

    let revision_before_save = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE writes layer and draw order back");

    assert_eq!(
        revision(&app),
        Revision(revision_before_save.0 + 1),
        "SAVE is still one revision"
    );
    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(101), EntityId(100)],
        "the block member order follows the clones' relative draw order"
    );
    assert_eq!(
        drawing.entity(EntityId(100)).unwrap().layer,
        LayerId(3),
        "the member takes the clone's layer"
    );
    assert_eq!(drawing.entity(EntityId(100)).unwrap().draw_order, 1);
    assert_eq!(drawing.entity(EntityId(101)).unwrap().draw_order, 0);

    // One undo step restores both the layer and the pre-save member order.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the save");
    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)]
    );
    assert_eq!(drawing.entity(EntityId(100)).unwrap().layer, LayerId(0));
    assert_eq!(drawing.entity(EntityId(100)).unwrap().draw_order, 0);
    assert_eq!(drawing.entity(EntityId(101)).unwrap().draw_order, 1);
}

#[test]
fn refclose_discard_writes_neither_layer_nor_draw_order() {
    let (mut app, mut session) = two_member_app();
    let clones = begin(&mut app, &mut session);

    select(&mut session, &[clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: "layer".into(),
            value: "WALLS".into(),
        },
    )
    .expect("the clone changes layer");
    select(&mut session, &[clones[1], clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::DrawOrder,
        CommandPayload::Draw2(Box::new(Draw2Payload::DrawOrder(DrawOrderMode::Above))),
    )
    .expect("the clones swap order");

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Discard),
    )
    .expect("REFCLOSE DISCARD only deletes the clones");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)],
        "DISCARD never rewrites the member order"
    );
    assert_eq!(
        drawing.entity(EntityId(100)).unwrap().layer,
        LayerId(0),
        "DISCARD never rewrites the member layer"
    );
    assert_eq!(drawing.entity(EntityId(100)).unwrap().draw_order, 0);
    assert_eq!(drawing.entity(EntityId(101)).unwrap().draw_order, 1);
    for clone in clones {
        assert!(
            drawing.entity(EntityId(clone)).is_none(),
            "clone {clone} is deleted"
        );
    }
}

/// A block whose dynamic-visibility state `A` shows member 100 and hides 101.
fn dynamic_visibility_app() -> (Application, SessionState) {
    use cad_db::{DynamicBlockState, DynamicBlockVisibility};

    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(100), EntityId(101)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(part(
        100,
        line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
        LayerId(0),
        SpaceId::Block(BlockId(1)),
        "AcDbLine",
        0,
    ))
    .unwrap();
    b.insert_entity(part(
        101,
        circle(p3(0.0, 0.0, 0.0), 1.0),
        LayerId(3),
        SpaceId::Block(BlockId(1)),
        "AcDbCircle",
        1,
    ))
    .unwrap();
    b.set_block_dynamic_visibility(
        BlockId(1),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(100), EntityId(101)],
            states: vec![
                DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(100)],
                },
                DynamicBlockState {
                    name: "B".into(),
                    entities: vec![EntityId(101)],
                },
            ],
            active_state: Some("A".into()),
        },
    )
    .unwrap();
    b.insert_entity(part(
        1,
        insert(1, placement()),
        LayerId(0),
        SpaceId::Model,
        "AcDbBlockReference",
        0,
    ))
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

#[test]
fn refclose_save_preserves_a_hidden_dynamic_visibility_member_and_its_descriptor() {
    let (mut app, mut session) = dynamic_visibility_app();
    let clones = begin(&mut app, &mut session);
    assert_eq!(clones.len(), 1, "only the visible member is extracted");
    assert_eq!(
        session.refedit.clone_of(EntityId(clones[0])),
        Some(EntityId(100))
    );

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE keeps the hidden member");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)],
        "the hidden member stays a member"
    );
    assert_eq!(
        drawing.entity(EntityId(101)).unwrap().geometry,
        circle(p3(0.0, 0.0, 0.0), 1.0),
        "the hidden member's geometry is untouched"
    );
    let visibility = drawing
        .block_dynamic_visibility(BlockId(1))
        .expect("the descriptor survives");
    assert_eq!(
        visibility.member_entities,
        vec![EntityId(100), EntityId(101)]
    );
    assert_eq!(visibility.active_state.as_deref(), Some("A"));
    assert_eq!(
        drawing.block_visible_entities(BlockId(1)),
        Some(vec![EntityId(100)]),
        "the hidden member is still hidden under state A"
    );
}

#[test]
fn refclose_save_writes_back_render_attributes_and_keeps_visibility_membership() {
    let (mut app, mut session) = dynamic_visibility_app();
    let clones = begin(&mut app, &mut session);
    assert_eq!(clones.len(), 1);
    let membership_before = drawing_of(&app).entity_visibility_membership(EntityId(100));
    assert!(
        !membership_before.visibility.is_empty(),
        "the fixture member is governed by the visibility descriptor"
    );

    select(&mut session, &[clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: "color".into(),
            value: "aci:1".into(),
        },
    )
    .expect("the clone takes an explicit color");
    let clone_attributes = drawing_of(&app).entity_render_attributes(EntityId(clones[0]));
    assert_ne!(
        clone_attributes.color,
        drawing_of(&app)
            .entity_render_attributes(EntityId(100))
            .color,
        "the clone and the member differ before the save"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE writes the render attributes back");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.entity_render_attributes(EntityId(100)),
        clone_attributes,
        "the member takes the clone's render attributes"
    );
    assert_eq!(
        drawing.entity_visibility_membership(EntityId(100)),
        membership_before,
        "the member's block/visibility membership is not copied from the clone"
    );
    assert_eq!(
        drawing.block_visible_entities(BlockId(1)),
        Some(vec![EntityId(100)])
    );
}

#[test]
fn refset_remove_prunes_a_deleted_member_from_the_visibility_descriptor() {
    let (mut app, mut session) = dynamic_visibility_app();
    let clones = begin(&mut app, &mut session);
    assert_eq!(clones.len(), 1);

    select(&mut session, &[clones[0]]);
    execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::RefSet(RefSetOption::Remove),
    )
    .expect("REFSET REMOVE drops the visible member from the working set");
    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE deletes the removed member");

    let drawing = drawing_of(&app);
    assert!(
        drawing.entity(EntityId(100)).is_none(),
        "the removed member leaves the block"
    );
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(101)],
        "only the untouched hidden member remains"
    );
    let visibility = drawing
        .block_dynamic_visibility(BlockId(1))
        .expect("the descriptor survives");
    assert_eq!(
        visibility.member_entities,
        vec![EntityId(101)],
        "no dangling member id may remain in the descriptor"
    );
    assert!(
        visibility.states[0].entities.is_empty(),
        "state A lists no deleted member"
    );
    assert_eq!(visibility.states[1].entities, vec![EntityId(101)]);
    assert_eq!(
        drawing.block_visible_entities(BlockId(1)),
        Some(Vec::new()),
        "state A is now empty rather than naming a deleted entity"
    );
}

/// Block 1 owns member 100 and a nested INSERT 101 of block 2 (member 200).
fn nested_member_app() -> (Application, SessionState) {
    let nested = Transform3::translation(p3(2.0, 0.0, 0.0));
    build(
        vec![
            part(
                100,
                line(p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbLine",
                0,
            ),
            part(
                101,
                insert(2, nested),
                LayerId(0),
                SpaceId::Block(BlockId(1)),
                "AcDbBlockReference",
                1,
            ),
            part(
                200,
                line(p3(0.0, 0.0, 0.0), p3(3.0, 0.0, 0.0)),
                LayerId(0),
                SpaceId::Block(BlockId(2)),
                "AcDbLine",
                0,
            ),
            part(
                1,
                insert(1, placement()),
                LayerId(0),
                SpaceId::Model,
                "AcDbBlockReference",
                0,
            ),
        ],
        vec![
            BlockDefinition {
                id: BlockId(1),
                entities: vec![EntityId(100), EntityId(101)],
                dynamic_visibility: None,
            },
            BlockDefinition {
                id: BlockId(2),
                entities: vec![EntityId(200)],
                dynamic_visibility: None,
            },
        ],
    )
}

fn assert_matrix_close(actual: &Transform3, expected: &Transform3) {
    for (row_actual, row_expected) in actual.matrix.iter().zip(expected.matrix.iter()) {
        for (a, e) in row_actual.iter().zip(row_expected.iter()) {
            assert!((a - e).abs() < 1e-9, "{actual:?} != {expected:?}");
        }
    }
}

#[test]
fn refedit_preserves_a_nested_insert_member_as_a_reference() {
    let (mut app, mut session) = nested_member_app();
    let clones = begin(&mut app, &mut session);
    assert_eq!(clones.len(), 2, "both visible members are extracted");

    let nested_in_block = Transform3::translation(p3(2.0, 0.0, 0.0));
    match &drawing_of(&app)
        .entity(EntityId(clones[1]))
        .expect("the nested clone")
        .geometry
    {
        SemanticGeometry::Insert {
            block, transform, ..
        } => {
            assert_eq!(*block, BlockId(2), "the nested reference is preserved");
            assert_matrix_close(transform, &placement().matrix_mul(&nested_in_block));
        }
        other => panic!("the nested member must stay an INSERT, got {other:?}"),
    }

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE restores the nested member exactly");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100), EntityId(101)]
    );
    match &drawing.entity(EntityId(101)).expect("member 101").geometry {
        SemanticGeometry::Insert {
            block, transform, ..
        } => {
            assert_eq!(*block, BlockId(2));
            assert_matrix_close(transform, &nested_in_block);
        }
        other => panic!("member 101 must stay an INSERT, got {other:?}"),
    }
    assert_eq!(
        drawing.block(BlockId(2)).unwrap().entities,
        vec![EntityId(200)],
        "the nested block definition is untouched"
    );
    assert_eq!(
        drawing.entity(EntityId(200)).unwrap().geometry,
        line(p3(0.0, 0.0, 0.0), p3(3.0, 0.0, 0.0))
    );
}

#[test]
fn refedit_and_refset_refuse_a_nested_instance_selection_without_writing() {
    let (mut app, mut session) = nested_member_app();
    let before = revision(&app);
    let nested_ref = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(200),
        instance: InstancePath(vec![EntityId(1), EntityId(101)]),
        sub_element: None,
    };

    session.selection = SelectionSet::from_refs([nested_ref.clone()]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefEdit,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.nested"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert!(session.hidden_entities.is_empty());
    assert!(!session.refedit.is_active());

    // Start a real session, then refuse the nested ref through REFSET too.
    let _clones = begin(&mut app, &mut session);
    let revision_after_begin = revision(&app);
    session.selection = SelectionSet::from_refs([nested_ref]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("refedit.nested"), "{error}");
    assert_eq!(revision(&app), revision_after_begin);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn refset_add_and_remove_are_session_state_only_and_add_is_a_move_on_save() {
    let (mut app, mut session) = one_member_app_with_host_line();
    let clones = begin(&mut app, &mut session);
    assert_eq!(clones.len(), 1);
    let revision_after_begin = revision(&app);

    // ADD the host LINE (id 2) twice: the second add is a no-op, and neither add
    // writes the drawing.
    select(&mut session, &[2]);
    execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::None,
    )
    .expect("REFSET ADD adds the host line");
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::RefSet(RefSetOption::Add),
    )
    .expect("adding it again is a no-op");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "refset.add"
                && diagnostic.message.contains("added 0 object(s)")),
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), revision_after_begin, "REFSET never writes");
    assert_eq!(undo_depth(&app), 1);

    // REMOVE it again before saving: the host line must stay an ordinary
    // model-space object (ADD only becomes a move when SAVE writes the block).
    execute(
        &mut app,
        &mut session,
        CommandId::RefSet,
        CommandPayload::RefSet(RefSetOption::Remove),
    )
    .expect("REFSET REMOVE drops the host line");
    assert!(!session.refedit.working().unwrap().contains(&EntityId(2)));

    execute(
        &mut app,
        &mut session,
        CommandId::RefClose,
        CommandPayload::RefClose(RefCloseOption::Save),
    )
    .expect("REFCLOSE SAVE with no working-set change");

    let drawing = drawing_of(&app);
    assert_eq!(
        drawing.block(BlockId(1)).unwrap().entities,
        vec![EntityId(100)],
        "the removed host line never enters the block"
    );
    let host = drawing.entity(EntityId(2)).expect("the host line stays");
    assert_eq!(host.space, SpaceId::Model);
    assert_eq!(
        host.geometry,
        line(p3(10.0, 10.0, 0.0), p3(11.0, 10.0, 0.0)),
        "the host geometry is untouched"
    );
}
