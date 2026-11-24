use super::*;

// ---------------------------------------------------------------------------
// LAYFRZ
// ---------------------------------------------------------------------------

#[test]
fn layfrz_freezes_every_distinct_selected_layer_and_preserves_other_attributes() {
    let walls = LayerAttributes {
        locked: true,
        frozen_in_new_viewport: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", walls),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(4), 5.0),
            line(3, LayerId(3), 10.0),
        ],
    );
    select(&mut app, &mut session, &[1, 2, 3]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("LAYFRZ freezes the selection's layers");

    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(
        layer_attributes(&app, LayerId(4)).frozen,
        "one write per distinct layer"
    );
    assert!(
        layer_attributes(&app, LayerId(3)).locked,
        "the other flags are preserved"
    );
    assert!(
        layer_attributes(&app, LayerId(3)).frozen_in_new_viewport,
        "the all-viewports freeze does not touch the new-viewport flag"
    );
    assert!(!layer_attributes(&app, LayerId(0)).frozen);
    // One `set_layer_attributes` per changed layer: two revision bumps, but the
    // whole command is one undo step.
    assert_eq!(revision(&app), Revision(before.0 .0 + 2));
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step for the whole layer batch"
    );
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.applied"]);
    assert!(outcome.diagnostics[0].message.contains('2'));
    assert!(outcome.diagnostics[0].message.contains("one transaction"));

    // Undo restores the exact before attributes (frozen back to false, other
    // flags untouched); redo freezes again.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYFRZ");
    assert_eq!(revision(&app), Revision(before.0 .0 + 3));
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).locked);
    assert!(layer_attributes(&app, LayerId(3)).frozen_in_new_viewport);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYFRZ");
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(layer_attributes(&app, LayerId(4)).frozen);
}

#[test]
fn layfrz_refuses_the_whole_command_when_the_current_layer_is_selected() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(4), 5.0)],
    );
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("the current layer cannot be frozen");
    assert!(refusal(error).starts_with("layfrz.current_layer:"));
    assert!(
        !layer_attributes(&app, LayerId(4)).frozen,
        "no partial freeze"
    );
    assert_untouched(&app, before);
}

#[test]
fn layfrz_reports_unchanged_when_every_layer_is_already_frozen() {
    let frozen = LayerAttributes {
        frozen: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", frozen),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("an already-frozen selection is a no-op");
    assert_eq!(state(&app), before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.unchanged"]);
}

#[test]
fn layfrz_refuses_empty_and_non_addressable_selections() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("layfrz.selection:"));
    assert_untouched(&app, before);

    let sub = SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "edge".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([sub]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("a sub-element is refused");
    assert!(refusal(error).starts_with("layfrz.not_addressable:"));
    assert_untouched(&app, before);
}

// ---------------------------------------------------------------------------
// LAYFRZ across spaces: paper-space and block-member picks
// ---------------------------------------------------------------------------

/// The real-machine case (the same one `LAYDEL`'s paper-space test records): the
/// user is in a paper layout, selects the layout's own object and runs LAYFRZ.
/// The layer write only needs the layer row, so a paper-space pick is a legal
/// input; freezing it must be exactly the drawing-level freeze.
#[test]
fn layfrz_accepts_a_paper_space_selection() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, &[2]);
    assert_eq!(
        entity_layer(&app, 2),
        LayerId(5),
        "the fixture's paper-space member is on DOORS"
    );
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("a paper-space pick names a layer");

    assert!(layer_attributes(&app, LayerId(5)).frozen);
    assert!(
        !layer_attributes(&app, LayerId(3)).frozen,
        "another layer is untouched"
    );
    assert!(
        !layer_attributes(&app, LayerId(0)).frozen,
        "layer 0 is untouched"
    );
    assert_eq!(revision(&app), Revision(before.0 .0 + 1));
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.applied"]);
    assert!(
        viewport_overrides(&app, LayoutId(1), 0).is_empty()
            && viewport_overrides(&app, LayoutId(1), 1).is_empty(),
        "the drawing-level form writes no per-viewport override"
    );
}

/// A block member is a row of the same `entities` table and carries its layer,
/// so both shapes the picker can produce are legal inputs: a direct
/// block-definition member, and a nested pick through the `INSERT` that displays
/// it. A member drawn on layer `0` follows its `INSERT`'s layer — the one
/// effective-layer walk the visibility answer uses — so that is the layer the
/// freeze must land on.
#[test]
fn layfrz_accepts_block_member_selections_and_follows_the_effective_layer() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    // A direct block-definition member (space `Block`), stored on WALLS.
    select(&mut app, &mut session, &[5]);
    let before = state(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("a block-definition member names the layer it is stored on");
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    // A nested pick of the INSERT's member stored on WALLS names WALLS too.
    select_refs(&mut app, &mut session, vec![nested_ref(4, 5)]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("a nested pick names its effective layer");
    assert!(layer_attributes(&app, LayerId(3)).frozen);

    // The member on layer `0` inside the DOORS INSERT is displayed on DOORS, so
    // that is the layer AutoCAD's freeze must hide, not layer `0`.
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    select_refs(&mut app, &mut session, vec![nested_ref(4, 6)]);
    let before = state(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("a layer-0 block member follows its INSERT's layer");
    assert!(layer_attributes(&app, LayerId(5)).frozen);
    assert!(
        !layer_attributes(&app, LayerId(0)).frozen,
        "layer 0 itself is not what the geometry is drawn on"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");

    // An unresolvable nested path is still a named refusal, not a guess.
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    select_refs(&mut app, &mut session, vec![nested_ref(999, 5)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("a nested path that does not resolve is refused");
    assert!(refusal(error).starts_with("layfrz.not_addressable:"));
    assert_untouched(&app, before);
}

// ---------------------------------------------------------------------------
// LAYFRZ Settings > Viewports: the per-viewport freeze (`Vpfreeze`)
// ---------------------------------------------------------------------------

/// `CommandPayload::SelectionTool(LayFrzViewports)` — the typed Viewports scope.
fn viewports_scope(indices: &[usize]) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(crate::SelectionToolPayload::LayFrzViewports(
        indices.to_vec(),
    )))
}

#[test]
fn layfrz_viewports_form_freezes_only_the_named_viewports_and_undo_restores() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, &[2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[1]),
    )
    .expect("the Viewports form writes the per-viewport overrides");

    assert_eq!(
        viewport_overrides(&app, LayoutId(1), 0),
        std::collections::BTreeMap::new(),
        "a viewport outside the list keeps no override"
    );
    assert_eq!(
        viewport_overrides(&app, LayoutId(1), 1).get(&LayerId(5)),
        Some(&true),
        "the named viewport freezes the selected layer"
    );
    assert!(
        !layer_attributes(&app, LayerId(5)).frozen,
        "the layer table is untouched: the layer still draws in the other viewport"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one layout write, one revision"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.applied"]);
    assert!(outcome.diagnostics[0].message.contains("per-viewport"));
    assert!(outcome.diagnostics[0].message.contains("one transaction"));

    // Undo clears exactly that override.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYFRZ");
    assert!(viewport_overrides(&app, LayoutId(1), 1).is_empty());
    assert!(!layer_attributes(&app, LayerId(5)).frozen);

    // Redo writes it again, still without a layer-table change.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYFRZ");
    assert_eq!(
        viewport_overrides(&app, LayoutId(1), 1).get(&LayerId(5)),
        Some(&true)
    );
    assert!(!layer_attributes(&app, LayerId(5)).frozen);

    // The override is the state `LAYTHW` already thaws: there is no second
    // per-viewport model hiding from the rest of the layer batch.
    execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("LAYTHW clears the per-viewport override");
    assert!(viewport_overrides(&app, LayoutId(1), 1).is_empty());
    assert!(!layer_attributes(&app, LayerId(5)).frozen);

    // A repeat with every targeted override already `true` is a declared no-op.
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[1]),
    )
    .expect("re-freeze the viewport override");
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[1]),
    )
    .expect("an already-frozen viewport override is a no-op");
    assert_untouched(&app, before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.unchanged"]);
}

#[test]
fn layfrz_viewports_form_treats_an_empty_list_as_every_viewport() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, &[2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[]),
    )
    .expect("an empty list is the whole active layout");

    for index in 0..2 {
        assert_eq!(
            viewport_overrides(&app, LayoutId(1), index).get(&LayerId(5)),
            Some(&true),
            "viewport {index} receives the override"
        );
    }
    assert!(!layer_attributes(&app, LayerId(5)).frozen);
    assert_eq!(revision(&app), Revision(before.0 .0 + 1));
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.applied"]);
}

#[test]
fn layfrz_viewports_form_refuses_without_writing() {
    // A model-space session has no viewport to freeze in.
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    select(&mut app, &mut session, &[2]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[1]),
    )
    .expect_err("the Viewports form needs a paper layout");
    assert!(refusal(error).starts_with("layfrz.paper_space_required:"));
    assert_untouched(&app, before);

    // An index outside the layout is refused before the valid one is written.
    session.active_space = SpaceId::Paper(LayoutId(1));
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[0, 2]),
    )
    .expect_err("a viewport index outside the layout is refused");
    assert!(refusal(error).starts_with("layfrz.unknown_viewport:"));
    assert_untouched(&app, before);
    assert!(
        viewport_overrides(&app, LayoutId(1), 0).is_empty(),
        "nothing was frozen before the bad index was reported"
    );

    // The current layer stays refused in this scope too.
    session.active_layer = LayerId(5);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[1]),
    )
    .expect_err("the current layer cannot be frozen");
    assert!(refusal(error).starts_with("layfrz.current_layer:"));
    assert_untouched(&app, before);
    assert!(viewport_overrides(&app, LayoutId(1), 1).is_empty());

    // Any other payload is refused by name, never ignored.
    session.active_layer = LayerId(0);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::LayerVisibilities(vec![(LayerId(5), false)]),
    )
    .expect_err("a foreign payload is refused");
    assert!(refusal(error).starts_with("layfrz.payload:"));
    assert_untouched(&app, before);
}

#[test]
fn layfrz_viewports_form_refuses_a_layout_without_viewports() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "DOORS")] {
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
            name: "Empty".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.insert_entity(line(1, LayerId(5), 0.0)).unwrap();
    let (mut app, mut session) = app_with_database(builder.finish().unwrap());
    session.active_space = SpaceId::Paper(LayoutId(1));
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        viewports_scope(&[]),
    )
    .expect_err("a layout without viewports has nowhere to freeze");
    assert!(refusal(error).starts_with("layfrz.no_viewport:"));
    assert_untouched(&app, before);
    assert!(!layer_attributes(&app, LayerId(5)).frozen);
}
