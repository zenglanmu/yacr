//! `LAYDEL` (AutoCAD 2026): delete the selection's layers and their model entities.
//!
//! The source layers are the distinct layers of the current selection. Every
//! **model-space** entity on those layers is deleted in one transaction / one undo
//! step; the layer rows are then deleted (not undoable). Refusals: empty
//! selection, a payload, a source equal to the active layer, the default layer
//! `0`, and a source that still has paper-space or block members (the command
//! only deletes model-space entities).

use super::*;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
    ]
}

#[test]
fn laydel_deletes_the_model_entities_and_the_layer_rows() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(3), 5.0),
            line(3, LayerId(5), 10.0),
        ],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("LAYDEL deletes the selection's layer");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(
        drawing.entity(EntityId(2)).is_none(),
        "the whole layer's model entities go"
    );
    assert!(
        drawing.entity(EntityId(3)).is_some(),
        "another layer is untouched"
    );
    assert!(drawing.layer(LayerId(3)).is_none());
    assert!(drawing.layer(LayerId(5)).is_some());
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one entity transaction plus one layer deletion"
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "only the entity half is undoable"
    );
    assert!(outcome.changes.is_some());
    assert!(diagnostic_codes(&outcome).contains(&"laydel.applied"));
    assert!(
        session.selection.refs().is_empty(),
        "the deleted entities are dropped from the selection"
    );
}

#[test]
fn laydel_refuses_the_active_layer_and_the_default_layer() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("the active layer cannot be deleted");
    assert!(refusal(error).starts_with("laydel.current_layer:"));
    assert_untouched(&app, before);

    session.active_layer = LayerId(5);
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(0), 0.0)]);
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("layer 0 cannot be deleted");
    assert!(refusal(error).starts_with("laydel.default_layer:"));
    assert_untouched(&app, before);
}

#[test]
fn laydel_refuses_an_empty_selection_a_payload_and_instance_refs() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laydel.selection:"));
    assert_untouched(&app, before);

    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::LayerVisibilities(vec![(LayerId(0), false)]),
    )
    .expect_err("a payload is refused");
    assert!(refusal(error).starts_with("laydel.payload:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laydel.not_addressable:"));
    assert_untouched(&app, before);
}

#[test]
fn laydel_refuses_a_layer_that_still_has_paper_space_members() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    let paper = DbEntity {
        space: SpaceId::Paper(LayoutId(1)),
        ..line(9, LayerId(3), 0.0)
    };
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layer(Layer {
            id: LayerId(3),
            name: "WALLS".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    builder.insert_entity(paper).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());

    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("a paper-space member blocks the layer delete");
    assert!(refusal(error).starts_with("laydel.layer_in_use:"));
    assert_untouched(&app, before);
}
