use super::*;

// ---------------------------------------------------------------------------
// SETBYLAYER
// ---------------------------------------------------------------------------

fn explicit_display() -> EntityRenderAttributes {
    EntityRenderAttributes {
        color: EntityColor::Explicit([255, 0, 0]),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::default(),
            scale: 2.5,
        },
        lineweight: EntityLineWeight::Explicit(0.5),
        transparency: EntityTransparency::Explicit(0.5),
        linetype_scale: 2.5,
        ..EntityRenderAttributes::default()
    }
}

fn render_attributes(app: &Application, entity: u128) -> EntityRenderAttributes {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(EntityId(entity))
}

#[test]
fn setbylayer_resets_colour_linetype_and_lineweight_and_preserves_everything_else() {
    let entity = line(1, LayerId(3), 0.0);
    let geometry = entity.geometry.clone();
    let (mut app, mut session) = app_with_display(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![entity],
        vec![(EntityId(1), explicit_display())],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect("SETBYLAYER resets the display overrides");

    let attributes = render_attributes(&app, 1);
    assert_eq!(attributes.color, EntityColor::ByLayer);
    assert_eq!(attributes.linetype, EntityLineType::ByLayer);
    assert_eq!(attributes.lineweight, EntityLineWeight::ByLayer);
    // Not reset: transparency, the entity's own linetype scale, the row fields.
    assert_eq!(attributes.transparency, EntityTransparency::Explicit(0.5));
    assert_eq!(attributes.linetype_scale, 2.5);
    let stored = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    assert_eq!(stored.layer, LayerId(3));
    assert_eq!(stored.space, SpaceId::Model);
    assert_eq!(stored.geometry, geometry);
    // One transaction / one undo step.
    assert_eq!(revision(&app), Revision(before.0 .0 + 1));
    assert_eq!(undo_depth(&app), before.1 + 1);
    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["setbylayer.applied", "setbylayer.properties_not_reset"]
    );
    let message = &outcome.diagnostics[1].message;
    assert!(message.contains("transparency"));
    assert!(message.contains("materials"));
    assert!(message.contains("Include blocks"));
}

#[test]
fn setbylayer_is_undoable_and_restores_the_explicit_display_state() {
    let (mut app, mut session) = app_with_display(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
        vec![(EntityId(1), explicit_display())],
    );
    select(&mut app, &mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect("SETBYLAYER resets the display overrides");
    assert_eq!(render_attributes(&app, 1).color, EntityColor::ByLayer);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the display write is undoable");
    let restored = render_attributes(&app, 1);
    assert_eq!(restored.color, EntityColor::Explicit([255, 0, 0]));
    assert_eq!(restored.lineweight, EntityLineWeight::Explicit(0.5));
    assert_eq!(restored.linetype_scale, 2.5);
}

#[test]
fn setbylayer_refuses_the_whole_command_when_any_layer_is_locked() {
    let locked = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with_display(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", locked),
        ],
        vec![line(1, LayerId(0), 0.0), line(2, LayerId(3), 5.0)],
        vec![
            (EntityId(1), explicit_display()),
            (EntityId(2), explicit_display()),
        ],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect_err("a locked layer refuses the whole command");
    assert!(refusal(error).starts_with("setbylayer.locked:"));
    assert_eq!(
        render_attributes(&app, 1).color,
        EntityColor::Explicit([255, 0, 0]),
        "no partial reset"
    );
    assert_untouched(&app, before);
}

#[test]
fn setbylayer_reports_unchanged_when_every_override_is_already_bylayer() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect("an all-ByLayer selection is a no-op");
    assert_eq!(state(&app), before, "no transaction, no undo entry");
    assert!(outcome.changes.is_none());
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["setbylayer.unchanged", "setbylayer.properties_not_reset"]
    );
}

#[test]
fn setbylayer_refuses_empty_and_non_addressable_selections() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("setbylayer.selection:"));
    assert_untouched(&app, before);

    let paper = SelectionRef {
        document: DocumentId(2),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([paper]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect_err("another document's ref is refused");
    assert!(refusal(error).starts_with("setbylayer.not_addressable:"));
    assert_untouched(&app, before);
}
