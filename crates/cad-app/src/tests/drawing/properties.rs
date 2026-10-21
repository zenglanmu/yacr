//! `SETENTITYPROPERTY` contracts: the typed property write path behind the
//! properties panel (`CommandId::SetEntityProperty` +
//! `CommandPayload::EntityProperty`).
//!
//! These pin the accepted value grammar, the all-or-nothing behaviour, the
//! one-transaction / one-undo-step granularity, undo/redo over **both** state
//! layers (entity row + display attributes) and the propagation mask. No GPU is
//! involved: the invalidation mechanism is the published `ChangeSet` mask, the
//! same one every other style edit uses.

use super::*;
use crate::property_value;

use cad_db::{
    BlockDefinition, EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes,
    EntityTransparency, Layer, LineType, LinetypePattern,
};
use cad_dependencies::DependencyIndex;

/// A drawing with layers `0`/`WALLS`, linetypes `CONTINUOUS`/`DASHED`, two
/// independent lines (ids 1 and 2), a block with one member (id 100) and an
/// INSERT (id 3) so a block-instance ref can be selected.
fn property_app() -> (Application, SessionState) {
    property_app_with(None)
}

fn property_app_with(attributes: Option<EntityRenderAttributes>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for (id, name, pattern) in [
        (LinetypeId(0), "CONTINUOUS", LinetypePattern::continuous()),
        (
            LinetypeId(1),
            "DASHED",
            LinetypePattern::from_elements([0.5, -0.25]),
        ),
    ] {
        b.insert_linetype(LineType {
            id,
            name: name.into(),
            pattern,
            complex: false,
        })
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(ent(
        100,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Line {
            start: point(0.0, 1.0),
            end: point(10.0, 1.0),
        },
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        3,
    ))
    .unwrap();
    if let Some(attributes) = attributes {
        b.set_entity_render_attributes(EntityId(1), attributes)
            .unwrap();
    }
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn set_property(
    app: &mut Application,
    session: &mut SessionState,
    key: &str,
    value: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: key.into(),
            value: value.into(),
        },
    )
}

fn stored_drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

fn stored_layer(app: &Application, entity: u128) -> LayerId {
    stored_drawing(app)
        .entity(EntityId(entity))
        .expect("the entity")
        .layer
}

fn stored_attributes(app: &Application, entity: u128) -> EntityRenderAttributes {
    stored_drawing(app).entity_render_attributes(EntityId(entity))
}

#[test]
fn layer_applies_to_the_whole_selection_as_one_undo_step() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let outcome = set_property(&mut app, &mut session, "layer", "WALLS").unwrap();

    // One transaction: one revision bump, every selected entity changed.
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(stored_layer(&app, 1), LayerId(1));
    assert_eq!(stored_layer(&app, 2), LayerId(1));
    let changes = outcome.changes.expect("the edit publishes a ChangeSet");
    assert_eq!(changes.changes.len(), 2);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "drawing.property.set"));

    // The selection survives the edit and the rows show the new layer.
    assert_eq!(session.selection.len(), 2);
    let drawing = stored_drawing(&app);
    let props = SelectionProperties::extract(&drawing, &session.selection);
    let layer = props.rows.iter().find(|row| row.key == "layer").unwrap();
    assert_eq!(layer.value.as_deref(), Some("WALLS (1)"));
    assert!(!layer.mixed);

    // Exactly one undo step restores both entities, and nothing else remains.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_layer(&app, 1), LayerId(0));
    assert_eq!(stored_layer(&app, 2), LayerId(0));
    assert!(!app.can_undo(&DocumentId(1)));

    // Redo restores the change.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_layer(&app, 1), LayerId(1));
    assert_eq!(stored_layer(&app, 2), LayerId(1));
}

#[test]
fn invalid_layer_name_is_refused_without_writing_or_history() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let error = set_property(&mut app, &mut session, "layer", "NO_SUCH_LAYER").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.unknown_layer"),
        "a stable code, got {error:?}"
    );
    assert_eq!(revision(&app), before, "no revision bump");
    assert_eq!(stored_layer(&app, 1), LayerId(0));
    assert_eq!(stored_layer(&app, 2), LayerId(0));
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn transparency_grammar_is_exact_and_never_clamps() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    for invalid in ["91", "-1", "abc", "5.0", "", "+5", " 5"] {
        let error = set_property(&mut app, &mut session, "transparency", invalid).unwrap_err();
        assert!(
            format!("{error:?}").contains("property.invalid_value"),
            "{invalid:?} must be refused, got {error:?}"
        );
    }
    assert_eq!(revision(&app), before, "refusals write nothing");
    assert!(!app.can_undo(&DocumentId(1)));

    // The bounds are accepted, and the percent is converted to opacity once.
    set_property(&mut app, &mut session, "transparency", "90").unwrap();
    let EntityTransparency::Explicit(opacity) = stored_attributes(&app, 1).transparency else {
        panic!("90 percent is an explicit opacity");
    };
    assert!(
        (opacity - 0.1).abs() < 1e-6,
        "90 percent is opacity 0.1, got {opacity}"
    );
    assert_eq!(
        property_value::property_value(
            &stored_drawing(&app),
            stored_drawing(&app).entity(EntityId(1)).unwrap(),
            "transparency",
        )
        .as_deref(),
        Some("90")
    );

    set_property(&mut app, &mut session, "transparency", "0").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).transparency,
        EntityTransparency::Explicit(1.0)
    );
    assert_eq!(
        property_value::property_value(
            &stored_drawing(&app),
            stored_drawing(&app).entity(EntityId(1)).unwrap(),
            "transparency",
        )
        .as_deref(),
        Some("0")
    );
}

#[test]
fn rewriting_the_displayed_percentage_is_a_no_op() {
    // The importer once derived opacity from a source byte (1 - byte/255), so
    // the stored float need not equal the exact percent-derived float. The no-op
    // rule compares the displayed percentage.
    let attributes = EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(1.0 - 140.0 / 255.0),
        ..EntityRenderAttributes::default()
    };
    let (mut app, mut session) = property_app_with(Some(attributes));
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);
    let displayed = transparency_percent(&app);
    assert_eq!(displayed, 55, "140/255 displays as 55 percent");

    let outcome = set_property(&mut app, &mut session, "transparency", "55").unwrap();
    assert!(
        outcome.changes.is_none(),
        "the displayed value is unchanged"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

fn transparency_percent(app: &Application) -> i32 {
    let drawing = stored_drawing(app);
    let value = property_value::property_value(
        &drawing,
        drawing.entity(EntityId(1)).unwrap(),
        "transparency",
    )
    .expect("a projected transparency");
    value.parse().expect("an integer percent")
}

#[test]
fn non_addressable_ref_refuses_the_whole_selection() {
    let (mut app, mut session) = property_app();
    let instance_member = SelectionRef {
        instance: InstancePath(vec![EntityId(3)]),
        ..ref_of(2)
    };
    let sub_element = SelectionRef {
        sub_element: Some(cad_domain::SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(2)
    };
    for bad in [instance_member, sub_element] {
        // One editable ref plus one non-editable ref: the whole command refuses.
        session.selection.replace([ref_of(1), bad]);
        let before = revision(&app);
        let error = set_property(&mut app, &mut session, "layer", "WALLS").unwrap_err();
        assert!(
            format!("{error:?}").contains("property.not_addressable"),
            "got {error:?}"
        );
        assert_eq!(revision(&app), before);
        assert_eq!(stored_layer(&app, 1), LayerId(0), "no partial write");
        assert_eq!(stored_layer(&app, 2), LayerId(0));
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn setting_the_current_value_creates_no_undo_entry() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let outcome = set_property(&mut app, &mut session, "layer", "0").unwrap();
    assert!(outcome.changes.is_none(), "a no-op publishes no ChangeSet");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "property.unchanged"));
    assert_eq!(revision(&app), before, "no revision bump");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn only_the_entities_that_change_are_patched() {
    let (mut app, mut session) = property_app();
    // Entity 1 already sits on WALLS; only entity 2 changes.
    session.selection.replace([ref_of(1)]);
    set_property(&mut app, &mut session, "layer", "WALLS").unwrap();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let outcome = set_property(&mut app, &mut session, "layer", "WALLS").unwrap();
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert!(outcome.changes.is_some());
    assert_eq!(stored_layer(&app, 1), LayerId(1));
    assert_eq!(stored_layer(&app, 2), LayerId(1));

    // The undo record holds exactly the one changed entity; undoing leaves the
    // already-WALLS entity untouched.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_layer(&app, 1), LayerId(1));
    assert_eq!(stored_layer(&app, 2), LayerId(0));
}

#[test]
fn color_linetype_lineweight_round_trip_through_db_and_projection() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1)]);

    // Color: ACI 7 is white; the canonical projection returns the index.
    set_property(&mut app, &mut session, "color", "aci:7").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).color,
        EntityColor::Explicit([255, 255, 255])
    );
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(&drawing, drawing.entity(EntityId(1)).unwrap(), "color")
            .as_deref(),
        Some("aci:7")
    );

    // A true colour not in the palette round-trips as #RRGGBB.
    set_property(&mut app, &mut session, "color", "#0a141e").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).color,
        EntityColor::Explicit([10, 20, 30])
    );
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(&drawing, drawing.entity(EntityId(1)).unwrap(), "color")
            .as_deref(),
        Some("#0A141E")
    );

    // Symbolic values are preserved, never resolved away.
    for (value, expected) in [
        ("ByLayer", EntityColor::ByLayer),
        ("ByBlock", EntityColor::ByBlock),
    ] {
        set_property(&mut app, &mut session, "color", value).unwrap();
        assert_eq!(stored_attributes(&app, 1).color, expected);
        let drawing = stored_drawing(&app);
        assert_eq!(
            property_value::property_value(&drawing, drawing.entity(EntityId(1)).unwrap(), "color")
                .as_deref(),
            Some(value)
        );
    }

    // Linetype: a named table entry stores the table's own pattern and keeps
    // the entity's scale (1.0 when the previous value was symbolic).
    set_property(&mut app, &mut session, "linetype", "DASHED").unwrap();
    let linetype = stored_attributes(&app, 1).linetype;
    assert_eq!(
        linetype,
        EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale: 1.0,
        }
    );
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(&drawing, drawing.entity(EntityId(1)).unwrap(), "linetype")
            .as_deref(),
        Some("DASHED")
    );

    // A symbolic linetype is preserved as such.
    set_property(&mut app, &mut session, "linetype", "ByBlock").unwrap();
    assert_eq!(stored_attributes(&app, 1).linetype, EntityLineType::ByBlock);
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(&drawing, drawing.entity(EntityId(1)).unwrap(), "linetype")
            .as_deref(),
        Some("ByBlock")
    );

    // Lineweight: only the canonical millimetre list is accepted.
    let error = set_property(&mut app, &mut session, "lineweight", "0.37").unwrap_err();
    assert!(format!("{error:?}").contains("property.invalid_value"));
    set_property(&mut app, &mut session, "lineweight", "0.35").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).lineweight,
        EntityLineWeight::Explicit(0.35)
    );
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(
            &drawing,
            drawing.entity(EntityId(1)).unwrap(),
            "lineweight"
        )
        .as_deref(),
        Some("0.35")
    );

    set_property(&mut app, &mut session, "lineweight", "Default").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).lineweight,
        EntityLineWeight::Default
    );
    let drawing = stored_drawing(&app);
    assert_eq!(
        property_value::property_value(
            &drawing,
            drawing.entity(EntityId(1)).unwrap(),
            "lineweight"
        )
        .as_deref(),
        Some("Default")
    );
}

#[test]
fn linetype_edit_preserves_an_existing_scale() {
    // The entity's own linetype scale is an entity value, not a table value, so
    // switching the named linetype keeps it.
    let attributes = EntityRenderAttributes {
        linetype: EntityLineType::Explicit {
            name: "CONTINUOUS".into(),
            pattern: LinetypePattern::continuous(),
            scale: 2.5,
        },
        ..EntityRenderAttributes::default()
    };
    let (mut app, mut session) = property_app_with(Some(attributes));
    session.selection.replace([ref_of(1)]);

    set_property(&mut app, &mut session, "linetype", "DASHED").unwrap();
    assert_eq!(
        stored_attributes(&app, 1).linetype,
        EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale: 2.5,
        }
    );
}

#[test]
fn change_propagation_marks_the_entity_for_a_representation_rebuild() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1)]);

    let outcome = set_property(&mut app, &mut session, "color", "aci:1").unwrap();
    let changes = outcome.changes.expect("the edit publishes a ChangeSet");
    let mask = changes
        .changes
        .iter()
        .find_map(|change| match change {
            cad_db::ObjectChange::Update(object, mask) if object.0 == 1 => Some(*mask),
            _ => None,
        })
        .expect("an update for entity 1");
    assert!(
        mask.contains(cad_db::ChangeMask::STYLE) && mask.invalidates_representation(),
        "a display-attribute edit is a style change: {mask:?}"
    );

    // The same mechanism the other edits use: the dependency index reports the
    // entity as a representation to rebuild.
    let mut index = DependencyIndex::default();
    let invalidation = index.invalidate(&changes).unwrap();
    assert!(!invalidation.rebuild_snapshot);
    assert!(invalidation
        .representations
        .contains(&cad_domain::ObjectId(1)));
}

#[test]
fn text_has_its_own_path_and_is_refused_here() {
    let (mut app, mut session) = property_app();
    session.selection.replace([ref_of(1)]);
    let before = revision(&app);

    let error = set_property(&mut app, &mut session, "text", "NEW").unwrap_err();
    assert!(
        format!("{error:?}").contains("property.unsupported_key"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));

    let error = set_property(&mut app, &mut session, "id", "2").unwrap_err();
    assert!(format!("{error:?}").contains("property.unsupported_key"));
}

#[test]
fn empty_selection_is_refused_explicitly() {
    let (mut app, mut session) = property_app();
    let error = set_property(&mut app, &mut session, "layer", "WALLS").unwrap_err();
    assert!(format!("{error:?}").contains("property.selection_empty"));
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn property_grammar_accepts_and_refuses_exactly() {
    // The grammar is also a standalone contract: canonical output parses back to
    // the same stored value.
    let (app, _) = property_app();
    let drawing = stored_drawing(&app);
    for (key, value) in [
        ("color", "aci:1"),
        ("color", "aci:256"),
        ("color", "aci:0"),
        ("color", "#0A141E"),
        ("linetype", "DASHED"),
        ("linetype", "ByLayer"),
        ("lineweight", "0.05"),
        ("lineweight", "ByBlock"),
        ("transparency", "42"),
        ("layer", "WALLS"),
    ] {
        property_value::parse(&drawing, key, value)
            .unwrap_or_else(|error| panic!("{key}={value} must parse: {error:?}"));
    }
    for (key, value) in [
        ("color", "aci:257"),
        ("color", "aci:-1"),
        ("color", "aci:+5"),
        ("color", "ACI:"),
        ("color", "#12345"),
        ("linetype", "NOPE"),
        ("lineweight", "2.12"),
        ("transparency", "91"),
        ("layer", "NOPE"),
    ] {
        assert!(
            property_value::parse(&drawing, key, value).is_err(),
            "{key}={value} must be refused"
        );
    }
}
