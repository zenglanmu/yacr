//! `LAYDEL` (AutoCAD 2026): delete the selection's layers and everything on them.
//!
//! The source layers are the distinct layers of the current selection. `cad-db`
//! keeps model-space entities, paper-space layout entities and block-definition
//! members in one `entities` table, so every entity on those layers is deleted
//! regardless of its space, together with each paper viewport override that
//! names the layer, and the layer rows are deleted too — all in one candidate /
//! one transaction / one undo step; undo restores every entity, the pruned
//! override and the layer row. Refusals: empty selection, a payload, a source
//! equal to the active layer, the default layer `0`, and a reference the
//! database itself refuses to remove (a locked layer's entities: `layer.locked`).

use super::*;
use cad_db::{BlockDefinition, PaperViewport};

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
        "one undo step covers both the entities and the layer row"
    );
    assert!(outcome.changes.is_some());
    assert!(diagnostic_codes(&outcome).contains(&"laydel.applied"));
    assert!(
        session.selection.refs().is_empty(),
        "the deleted entities are dropped from the selection"
    );

    // Undo restores the layer row first and then its entities, all in one
    // revision: no entity is left pointing at a missing layer.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 3));
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS")
    );
    assert_eq!(entity_layer(&app, 1), LayerId(3));
    assert_eq!(entity_layer(&app, 2), LayerId(3));
    assert_eq!(entity_layer(&app, 3), LayerId(5));

    // Redo removes the layer rows and the entities again.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 4));
    assert!(drawing.layer(LayerId(3)).is_none());
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_none());
    assert!(drawing.entity(EntityId(3)).is_some());
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
fn laydel_refuses_an_empty_selection_a_payload_and_block_instance_refs() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert_eq!(
        refusal(error),
        "laydel.selection: LAYDEL needs at least one selected object in model space or a \
         paper-space layout",
        "the diagnostic no longer claims model space only"
    );
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

    // A block-instance member has no directly addressable identity: refused by
    // name, and the refusal says which form it is.
    let block_space =
        "laydel.block_space: entity 1 is a block-instance member, a sub-element or a \
                       block-definition member; LAYDEL needs a top-level object in model space or \
                       a paper-space layout";
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
    assert_eq!(refusal(error), block_space);
    assert_untouched(&app, before);

    // A sub-element ref is refused by the same stable reason.
    session.selection = SelectionSet::from_refs([SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    }]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("a sub-element is refused");
    assert_eq!(refusal(error), block_space);
    assert_untouched(&app, before);
}

#[test]
fn laydel_accepts_a_paper_space_selection_and_deletes_every_space() {
    // The case the real-machine run exposed: the user switches to paper space,
    // selects the layout's `border` entity and runs LAYDEL. The selection is a
    // paper-space object, but the command deletes that layer's members in all
    // spaces, so it must be a legal input.
    let (mut app, mut session) = spaces_app(Vec::new());
    select(&mut app, &mut session, &[2]);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(2))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Paper(LayoutId(1))),
        "the selection is the layout's paper-space member"
    );
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("a paper-space selection names the layer");
    assert!(diagnostic_codes(&outcome).contains(&"laydel.applied"));

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_count(),
        1,
        "the layer's model, paper and block members are all deleted"
    );
    assert!(drawing.entity(EntityId(1)).is_none(), "model member");
    assert!(drawing.entity(EntityId(2)).is_none(), "paper member");
    assert!(drawing.entity(EntityId(5)).is_none(), "block member");
    assert!(
        drawing
            .block(BlockId(0))
            .is_some_and(|block| block.entities.is_empty()),
        "the block definition is kept but its deleted member is pruned"
    );
    assert!(drawing.layer(LayerId(3)).is_none(), "the layer row is gone");
    assert_eq!(
        drawing.entity(EntityId(3)).map(|entity| entity.layer),
        Some(LayerId(5)),
        "another layer's entity is untouched"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 4, "every member is restored");
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS")
    );
}

#[test]
fn laydel_deletes_a_paper_space_member_with_the_layer() {
    let (mut app, mut session) = spaces_app(Vec::new());
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(2))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Paper(LayoutId(1))),
        "the fixture's paper-space member is inside the layout"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("a paper-space member no longer blocks the layer delete");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(2)).is_none(),
        "the paper-space member is deleted with its layer"
    );
    assert!(
        drawing.entity(EntityId(1)).is_none(),
        "and the model member"
    );
    assert!(drawing.layer(LayerId(3)).is_none(), "the layer row is gone");
    assert!(
        drawing.layout(LayoutId(1)).is_some(),
        "the layout itself is untouched"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    // Undo puts the paper-space member back into its layout.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing
            .entity(EntityId(2))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Paper(LayoutId(1)))
    );
    assert!(drawing.layer(LayerId(3)).is_some());
}

#[test]
fn laydel_deletes_a_block_definition_member_and_drops_the_layer_row() {
    let (mut app, mut session) = spaces_app(Vec::new());
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("a block-definition member no longer blocks the layer delete");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.entity(EntityId(5)).is_none(),
        "the block-definition member is deleted with its layer"
    );
    assert!(drawing.layer(LayerId(3)).is_none());
    let block = drawing
        .block(BlockId(0))
        .expect("the block definition itself stays");
    assert!(
        block.entities.is_empty(),
        "the deleted member is pruned from the definition, not left dangling: {:?}",
        block.entities
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    // Undo restores the member row **and** its membership in the definition.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing
            .entity(EntityId(5))
            .map(|entity| entity.space.clone()),
        Some(SpaceId::Block(BlockId(0)))
    );
    assert_eq!(
        drawing
            .block(BlockId(0))
            .map(|block| block.entities.clone()),
        Some(vec![EntityId(5)]),
        "the display state a delete prunes is restored with the row"
    );
    assert!(drawing.layer(LayerId(3)).is_some());
}

#[test]
fn laydel_one_undo_step_restores_every_space_and_the_layer_table() {
    let (mut app, mut session) = spaces_app(Vec::new());
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 4, "model + paper + block + other");
    assert_eq!(drawing.layers().count(), 3);
    let table_before: Vec<String> = drawing.layers().map(|row| row.name.clone()).collect();

    execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("LAYDEL deletes a layer that has members in every space");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_count(),
        1,
        "only the other layer's model entity survives"
    );
    assert_eq!(
        drawing
            .layers()
            .map(|row| row.name.clone())
            .collect::<Vec<_>>(),
        vec!["0".to_string(), "DOORS".to_string()],
        "the layer table loses exactly the deleted row"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step, not three");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_count(),
        4,
        "every deleted entity is back in one step"
    );
    assert_eq!(
        drawing
            .layers()
            .map(|row| row.name.clone())
            .collect::<Vec<_>>(),
        table_before,
        "and so is the layer row"
    );
    assert_eq!(undo_depth(&app), before.1, "undo consumed the step");

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 1);
    assert!(drawing.layer(LayerId(3)).is_none());
}

#[test]
fn laydel_prunes_a_viewport_layer_override_and_undo_restores_it() {
    let (mut app, mut session) = spaces_app(vec![(LayerId(3), true)]);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("a per-viewport override dies with its layer");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.layer(LayerId(3)).is_none());
    assert!(
        drawing.layout(LayoutId(1)).expect("layout").viewports[0]
            .layer_overrides
            .is_empty(),
        "no override may outlive its layer row"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYDEL");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.layer(LayerId(3)).is_some());
    assert_eq!(
        drawing.layout(LayoutId(1)).expect("layout").viewports[0]
            .layer_overrides
            .get(&LayerId(3)),
        Some(&true),
        "the pruned override is restored with the layer row"
    );
}

#[test]
fn laydel_keeps_the_named_refusal_for_a_reference_it_cannot_delete() {
    // A locked layer's entities cannot be removed, so the layer cannot be
    // deleted either. The database's own named refusal is what reaches the
    // caller: no silent partial delete and no invented success.
    let locked = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", locked),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect_err("a locked layer's entities cannot be deleted");
    assert_eq!(
        refusal(error),
        "layer.locked: cannot delete entity 1 on locked layer 'WALLS' (3); unlock the layer first"
    );
    assert_untouched(&app, before);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "the refused delete removed nothing"
    );
}

/// The real-machine case (Wayland GUI run on an imported drawing): the layer
/// being deleted also owns **imported** zero-length lines — one in a paper layout
/// and one in a block definition. The database holds them (the importer is
/// permissive) even though no new write could create them, so a delete is fine
/// and the undo must restore them verbatim: history replay reinstalls rows the
/// database itself held, and re-validating their geometry used to refuse the
/// whole step with `Invariant("invalid entity geometry: line is zero length")`,
/// leaving the drawing deleted.
#[test]
fn laydel_undo_restores_imported_zero_length_members() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(5), EntityId(6)],
            dynamic_visibility: None,
        })
        .unwrap();
    // A degenerate row is a zero-length line: exactly what `validate_geometry`
    // refuses on a new write, and exactly what the importer stores.
    let degenerate = |id: u128, space: SpaceId, y: f64| DbEntity {
        space,
        geometry: SemanticGeometry::Line {
            start: point(0.0, y),
            end: point(0.0, y),
        },
        ..line(id, LayerId(3), y)
    };
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Paper(LayoutId(1)),
            ..line(2, LayerId(3), 5.0)
        })
        .unwrap();
    builder
        .insert_entity(degenerate(4, SpaceId::Paper(LayoutId(1)), 6.0))
        .unwrap();
    builder
        .insert_entity(degenerate(5, SpaceId::Block(BlockId(0)), 7.0))
        .unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Block(BlockId(0)),
            ..line(6, LayerId(3), 8.0)
        })
        .unwrap();
    builder.insert_entity(line(3, LayerId(5), 10.0)).unwrap();
    let (mut app, mut session) = app_with_database(builder.finish().unwrap());

    let stored: Vec<DbEntity> = [1u128, 2, 4, 5, 6]
        .iter()
        .map(|id| {
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(*id))
                .unwrap()
                .clone()
        })
        .collect();
    // The GUI path: the user is in paper space and selects the layout's `border`
    // member; LAYDEL still deletes every space's members of that layer.
    select(&mut app, &mut session, &[2]);
    let before = state(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::LayDel,
        CommandPayload::None,
    )
    .expect("a delete has no geometry to validate");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.entity_count(),
        1,
        "model, paper and block members of WALLS are all gone"
    );
    assert!(drawing.entity(EntityId(4)).is_none(), "paper degenerate");
    assert!(drawing.entity(EntityId(5)).is_none(), "block degenerate");
    assert!(
        drawing
            .block(BlockId(0))
            .is_some_and(|block| block.entities.is_empty()),
        "the definition stays, its deleted members are pruned"
    );
    assert!(drawing.layer(LayerId(3)).is_none());
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    // One UNDO restores every row — including the two the write validator would
    // reject — plus the block membership and the layer row.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo restores the database's own rows");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 6, "every deleted row is back");
    for row in &stored {
        assert_eq!(
            drawing.entity(row.id),
            Some(row),
            "entity {:?} is restored byte-for-byte",
            row.id
        );
    }
    assert_eq!(
        drawing
            .block(BlockId(0))
            .map(|block| block.entities.clone()),
        Some(vec![EntityId(5), EntityId(6)]),
        "the block member list comes back in draw order"
    );
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS"),
        "and the layer row with it"
    );
    assert_eq!(undo_depth(&app), before.1, "undo consumed the step");

    // REDO replays the recorded rows again: delete only, no geometry validation.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo deletes the same rows");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.entity_count(), 1);
    assert!(drawing.entity(EntityId(4)).is_none());
    assert!(drawing.entity(EntityId(5)).is_none());
    assert!(drawing.layer(LayerId(3)).is_none());
}

/// Layers `0`/`WALLS`/`DOORS`, layout `Sheet1` with entity 2 in paper space,
/// block `0` holding entity 5, entity 1 in model space and entity 3 on `DOORS`
/// (the control). `overrides` seeds the layout viewport's per-layer override map.
fn spaces_app(overrides: Vec<(LayerId, bool)>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Sheet1".into(),
            viewports: vec![PaperViewport {
                clip: vec![
                    point(0.0, 0.0),
                    point(100.0, 0.0),
                    point(100.0, 50.0),
                    point(0.0, 50.0),
                ],
                model_to_paper: Transform3::scale(100.0),
                completeness: Completeness::Complete,
                layer_overrides: overrides.into_iter().collect(),
            }],
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(5)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Paper(LayoutId(1)),
            ..line(2, LayerId(3), 5.0)
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Block(BlockId(0)),
            ..line(5, LayerId(3), 8.0)
        })
        .unwrap();
    builder.insert_entity(line(3, LayerId(5), 10.0)).unwrap();
    app_with_database(builder.finish().unwrap())
}
