//! `LAYMRG` (AutoCAD 2026): merge the selection's layers into the active layer.
//!
//! Declared deviation: sources come from the current selection and the target is
//! the session's active layer (no interactive source/target prompts). Every
//! entity on a source layer is rewritten to the active layer and the source
//! layer rows are deleted in one transaction / one undo step; each deleted source
//! layer's key is pruned from every paper viewport's `layer_overrides` map in that
//! same step, so one undo restores the rows, the entities' original layers and the
//! exact override maps. Refusals: empty selection, a payload, a source equal to
//! the active layer, and the default layer `0`.

use super::*;
use cad_db::PaperViewport;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
    ]
}

#[test]
fn laymrg_moves_every_entity_from_the_selected_layers_into_the_active_layer() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(3), 5.0),
            line(3, LayerId(5), 10.0),
        ],
    );
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect("LAYMRG merges the selected layer");

    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(
        entity_layer(&app, 2),
        LayerId(5),
        "the whole source layer moves"
    );
    assert_eq!(entity_layer(&app, 3), LayerId(5));
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.layer(LayerId(3)).is_none(),
        "the source row is deleted"
    );
    assert!(drawing.layer(LayerId(5)).is_some());
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one entity transaction plus one layer deletion"
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step covers both the entities and the source layer row"
    );
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["laymrg.applied"]);

    // Undo restores the source layer row and puts its entities back on it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 3));
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS")
    );
    assert_eq!(entity_layer(&app, 1), LayerId(3));
    assert_eq!(entity_layer(&app, 2), LayerId(3));
    assert_eq!(entity_layer(&app, 3), LayerId(5));

    // Redo re-applies the merge and removes the source row again.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 4));
    assert!(drawing.layer(LayerId(3)).is_none());
    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(entity_layer(&app, 2), LayerId(5));
    assert_eq!(entity_layer(&app, 3), LayerId(5));
}

/// Layers `0`/`WALLS`/`DOORS`, layout `Sheet1` with one paper viewport whose
/// per-layer override map is `overrides`, entities 1 (model) and 2 (paper) on
/// `WALLS` and the control entity 3 on `DOORS`.
fn sheet_app(overrides: Vec<(LayerId, bool)>) -> (Application, SessionState) {
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
    builder.insert_entity(line(1, LayerId(3), 0.0)).unwrap();
    builder
        .insert_entity(DbEntity {
            space: SpaceId::Paper(LayoutId(1)),
            ..line(2, LayerId(3), 5.0)
        })
        .unwrap();
    builder.insert_entity(line(3, LayerId(5), 10.0)).unwrap();
    app_with_database(builder.finish().unwrap())
}

#[test]
fn laymrg_prunes_only_the_source_layers_viewport_overrides() {
    let (mut app, mut session) = sheet_app(vec![(LayerId(3), true), (LayerId(5), false)]);
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);

    execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect("LAYMRG merges the selected layer");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.layer(LayerId(3)).is_none(),
        "the source row is gone"
    );
    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(
        entity_layer(&app, 2),
        LayerId(5),
        "the paper-space member moves too"
    );
    assert_eq!(entity_layer(&app, 3), LayerId(5));
    let overrides = &drawing.layout(LayoutId(1)).expect("layout").viewports[0].layer_overrides;
    assert_eq!(
        overrides.get(&LayerId(3)),
        None,
        "no override may outlive its layer row"
    );
    assert_eq!(
        overrides.get(&LayerId(5)),
        Some(&false),
        "the surviving layer's override is untouched, false included"
    );
    assert_eq!(overrides.len(), 1, "only the source key was removed");
}

#[test]
fn laymrg_undo_restores_the_pruned_viewport_overrides_and_redo_prunes_them() {
    let (mut app, mut session) = sheet_app(vec![(LayerId(3), true), (LayerId(5), false)]);
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect("LAYMRG merges the selected layer");
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS"),
        "the source row is back"
    );
    assert_eq!(entity_layer(&app, 1), LayerId(3));
    assert_eq!(entity_layer(&app, 2), LayerId(3));
    assert_eq!(entity_layer(&app, 3), LayerId(5));
    let overrides = &drawing.layout(LayoutId(1)).expect("layout").viewports[0].layer_overrides;
    assert_eq!(
        overrides.get(&LayerId(3)),
        Some(&true),
        "restored with the row"
    );
    assert_eq!(overrides.get(&LayerId(5)), Some(&false));
    assert_eq!(overrides.len(), 2, "the exact override map is back");

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.layer(LayerId(3)).is_none());
    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(entity_layer(&app, 2), LayerId(5));
    let overrides = &drawing.layout(LayoutId(1)).expect("layout").viewports[0].layer_overrides;
    assert_eq!(
        overrides.get(&LayerId(3)),
        None,
        "redo prunes the source key again"
    );
    assert_eq!(overrides.get(&LayerId(5)), Some(&false));
}

#[test]
fn laymrg_refuses_a_source_that_is_the_active_layer() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("merging a layer into itself is refused");
    assert!(refusal(error).starts_with("laymrg.source_is_active:"));
    assert_untouched(&app, before);
}

#[test]
fn laymrg_refuses_the_default_layer_as_a_source() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(0), 0.0)]);
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("layer 0 cannot be merged away");
    assert!(refusal(error).starts_with("laymrg.default_layer:"));
    assert_untouched(&app, before);
}

#[test]
fn laymrg_refuses_an_empty_selection_a_payload_and_instance_refs() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(5), 5.0)],
    );
    session.active_layer = LayerId(5);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laymrg.selection:"));
    assert_untouched(&app, before);

    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::LayerVisibilities(vec![(LayerId(0), false)]),
    )
    .expect_err("a payload is refused");
    assert!(refusal(error).starts_with("laymrg.payload:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laymrg.not_addressable:"));
    assert_untouched(&app, before);
}
