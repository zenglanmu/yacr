use super::*;
use crate::{SelectionToolPayload, SetByLayerSettings};
use cad_db::BlockDefinition;
use cad_domain::BlockId;

// ---------------------------------------------------------------------------
// SETBYLAYER
// ---------------------------------------------------------------------------

fn explicit_display() -> EntityRenderAttributes {
    EntityRenderAttributes {
        clip: None,
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
fn setbylayer_resets_colour_linetype_lineweight_transparency_and_preserves_everything_else() {
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
    assert_eq!(attributes.transparency, EntityTransparency::ByLayer);
    // Not reset: the entity's own linetype scale, the row fields.
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
    assert!(!message.contains("transparency"), "{message}");
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
    assert_eq!(restored.transparency, EntityTransparency::Explicit(0.5));
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

/// A drawing with block 0 (one member entity 100) and model-space INSERT 1.
///
/// `locked` records the locked flag on that layer's attribute row. The member
/// lives in `SpaceId::Block(0)`, exactly like a real block definition.
fn app_with_insert(member_layer: LayerId, locked: Option<LayerId>) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
        let attributes = LayerAttributes {
            locked: locked == Some(id),
            ..LayerAttributes::default()
        };
        builder.set_layer_attributes(id, attributes).unwrap();
    }
    builder
        .insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();

    let mut member = line(100, member_layer, 0.0);
    member.space = SpaceId::Block(BlockId(0));
    member.draw_order = 0;
    builder.insert_entity(member).unwrap();

    let mut insert = line(1, LayerId(0), 5.0);
    insert.object.type_key = "AcDbBlockReference".into();
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(0),
        transform: Transform3::identity(),
    };
    insert.draw_order = 1;
    builder.insert_entity(insert).unwrap();

    builder
        .set_entity_render_attributes(EntityId(1), explicit_display())
        .unwrap();
    builder
        .set_entity_render_attributes(EntityId(100), explicit_display())
        .unwrap();

    let drawing = builder.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([13u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn setbylayer_payload(include_blocks: bool) -> CommandPayload {
    CommandPayload::SelectionTool(Box::new(SelectionToolPayload::SetByLayer(
        SetByLayerSettings { include_blocks },
    )))
}

#[test]
fn setbylayer_resets_transparency_only_changes() {
    let (mut app, mut session) = app_with_display(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                clip: None,
                transparency: EntityTransparency::Explicit(0.5),
                ..EntityRenderAttributes::default()
            },
        )],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect("a transparency-only override is a real change");

    assert_eq!(
        render_attributes(&app, 1).transparency,
        EntityTransparency::ByLayer
    );
    assert_eq!(diagnostic_codes(&outcome)[0], "setbylayer.applied");
    assert_eq!(state(&app), (Revision(before.0 .0 + 1), before.1 + 1));
}

#[test]
fn include_blocks_rewrites_member_display_attributes_in_one_undo_step() {
    let (mut app, mut session) = app_with_insert(LayerId(0), None);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        setbylayer_payload(true),
    )
    .expect("Include blocks resets the INSERT and its members");

    for id in [1, 100] {
        let attributes = render_attributes(&app, id);
        assert_eq!(attributes.color, EntityColor::ByLayer, "entity {id}");
        assert_eq!(attributes.linetype, EntityLineType::ByLayer, "entity {id}");
        assert_eq!(
            attributes.lineweight,
            EntityLineWeight::ByLayer,
            "entity {id}"
        );
        assert_eq!(
            attributes.transparency,
            EntityTransparency::ByLayer,
            "entity {id}"
        );
        assert_eq!(attributes.linetype_scale, 2.5, "entity {id}");
    }
    assert_eq!(outcome.objects, vec![ObjectId(1), ObjectId(100)]);
    assert_eq!(state(&app), (Revision(before.0 .0 + 1), before.1 + 1));
    assert!(outcome.diagnostics[0].message.contains("block member"));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the block-member display write is undoable");
    assert_eq!(render_attributes(&app, 100), explicit_display());
    assert_eq!(render_attributes(&app, 1), explicit_display());
}

#[test]
fn a_bare_setbylayer_does_not_touch_block_members() {
    let (mut app, mut session) = app_with_insert(LayerId(0), None);
    select(&mut app, &mut session, &[1]);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .expect("the bare command resets the selected INSERT only");

    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert_eq!(render_attributes(&app, 1).color, EntityColor::ByLayer);
    assert_eq!(
        render_attributes(&app, 100),
        explicit_display(),
        "Include blocks was not requested"
    );
}

#[test]
fn include_blocks_refuses_a_locked_block_member() {
    let (mut app, mut session) = app_with_insert(LayerId(3), Some(LayerId(3)));
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        setbylayer_payload(true),
    )
    .expect_err("a locked block member refuses the whole command");
    assert!(refusal(error).starts_with("setbylayer.locked:"));
    assert_eq!(
        render_attributes(&app, 1),
        explicit_display(),
        "no partial reset"
    );
    assert_eq!(render_attributes(&app, 100), explicit_display());
    assert_untouched(&app, before);
}

#[test]
fn setbylayer_viewer_mode_is_refused_before_any_write() {
    let (mut app, mut session) = app_with_display(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
        vec![(EntityId(1), explicit_display())],
    );
    select(&mut app, &mut session, &[1]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = state(&app);

    assert!(execute(
        &mut app,
        &mut session,
        CommandId::SetByLayer,
        CommandPayload::None,
    )
    .is_err());

    assert_eq!(render_attributes(&app, 1), explicit_display());
    assert_untouched(&app, before);
}
