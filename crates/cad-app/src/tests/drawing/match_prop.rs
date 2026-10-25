//! `MATCHPROP` contracts: the selection-based property painter
//! (`CommandId::MatchProp`, `CommandPayload::None`).
//!
//! These pin the documented selection semantics (the current selection's
//! **first** entity is the source, every other selected entity is a
//! destination; the order is the pick order, never sorted by id), the exact
//! transferred-property set (`layer`, `color`, `linetype`, `lineweight`,
//! `transparency` — the `SETENTITYPROPERTY` values), the all-or-nothing refusals
//! (zero database change, zero undo entry), the per-destination no-op rule and
//! the one-transaction / one-undo-step granularity. No GPU is involved: the
//! propagation mask is the published `ChangeSet` mask, the same one every other
//! style edit uses.

use super::*;
use crate::property_value;

use cad_db::{
    BlockDefinition, ChangeMask, EntityColor, EntityLineType, EntityLineWeight,
    EntityRenderAttributes, EntityTransparency, Layer, LineType, LinetypePattern, ObjectChange,
};

/// A drawing with layers `0`/`WALLS`, linetypes `CONTINUOUS`/`DASHED`, a block
/// with one member (id 100) plus an INSERT (id 4), and three model-space
/// entities:
///
/// * id 1 — a **property-rich** LINE on `WALLS` (the usual source);
/// * id 2 — a plain LINE with an explicit `CONTINUOUS` linetype scale 2.5;
/// * id 3 — a plain ARC (a different kind, valid destination).
fn match_app() -> (Application, SessionState) {
    match_app_with_source(source_attributes())
}

/// The same fixture with the source entity's display attributes replaced, so a
/// test can store a legal-but-unwritable display state on the source.
fn match_app_with_source(attributes: EntityRenderAttributes) -> (Application, SessionState) {
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

    let mut source = ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        1,
    );
    source.layer = LayerId(1);
    b.insert_entity(source).unwrap();
    b.set_entity_render_attributes(EntityId(1), attributes)
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
    b.set_entity_render_attributes(EntityId(2), destination_attributes())
        .unwrap();

    b.insert_entity(ent(
        3,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        3,
    ))
    .unwrap();
    b.insert_entity(ent(
        4,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        4,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
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

/// The source's five writable display values: `layer` lives on the entity row
/// (set by the fixture), the other four here.
fn source_attributes() -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(0.5),
        color: EntityColor::Explicit([255, 0, 0]),
        lineweight: EntityLineWeight::Explicit(0.35),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale: 1.0,
        },
        ..EntityRenderAttributes::default()
    }
}

/// A destination whose display values differ from [`source_attributes`] in every
/// attribute, with a linetype scale the paint must **not** change (the scale is
/// not one of the five transferred keys).
fn destination_attributes() -> EntityRenderAttributes {
    EntityRenderAttributes {
        linetype: EntityLineType::Explicit {
            name: "CONTINUOUS".into(),
            pattern: LinetypePattern::continuous(),
            scale: 2.5,
        },
        ..EntityRenderAttributes::default()
    }
}

/// The display values a destination is expected to hold after being painted from
/// the fixture's source: the source's four attributes plus the destination's own
/// linetype scale.
fn painted_attributes(scale: f64) -> EntityRenderAttributes {
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(0.5),
        color: EntityColor::Explicit([255, 0, 0]),
        lineweight: EntityLineWeight::Explicit(0.35),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale,
        },
        ..EntityRenderAttributes::default()
    }
}

fn run_match(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::MatchProp, CommandPayload::None)
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

fn undo(app: &mut Application, session: &mut SessionState) {
    execute(app, session, CommandId::Undo, CommandPayload::None).unwrap();
}

#[test]
fn same_kind_copies_all_five_properties_in_one_undo_step() {
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let outcome = run_match(&mut app, &mut session).unwrap();

    // One transaction: one revision bump, one changed entity.
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(
        stored_layer(&app, 2),
        LayerId(1),
        "layer comes from the source"
    );
    assert_eq!(
        stored_attributes(&app, 2),
        painted_attributes(2.5),
        "colour/linetype/lineweight/transparency come from the source; the \
         destination keeps its own linetype scale"
    );
    // The source itself is untouched.
    assert_eq!(stored_layer(&app, 1), LayerId(1));
    assert_eq!(stored_attributes(&app, 1), source_attributes());

    let changes = outcome.changes.expect("the edit publishes a ChangeSet");
    assert_eq!(changes.changes.len(), 1, "only the destination changed");
    let mask = changes
        .changes
        .iter()
        .find_map(|change| match change {
            ObjectChange::Update(object, mask) if object.0 == 2 => Some(*mask),
            _ => None,
        })
        .expect("an update for the destination");
    assert!(
        mask.contains(ChangeMask::STYLE) && mask.invalidates_representation(),
        "a display-attribute edit is a style change: {mask:?}"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "drawing.matchprop"));

    // Exactly one undo step restores both state layers of the destination, and
    // nothing else remains to undo.
    undo(&mut app, &mut session);
    assert_eq!(stored_layer(&app, 2), LayerId(0));
    assert_eq!(stored_attributes(&app, 2), destination_attributes());
    assert_eq!(
        stored_layer(&app, 1),
        LayerId(1),
        "the source was never staged"
    );
    assert!(!app.can_undo(&DocumentId(1)), "exactly one undo entry");

    // Redo restores the painted values.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_layer(&app, 2), LayerId(1));
    assert_eq!(stored_attributes(&app, 2), painted_attributes(2.5));
}

#[test]
fn different_kinds_copy_when_every_value_is_valid() {
    let (mut app, mut session) = match_app();
    // A LINE source painted onto an ARC destination: different kinds are the
    // point of MATCHPROP, and every one of the five values is valid for an ARC.
    session.selection.replace([ref_of(1), ref_of(3)]);
    let before = revision(&app);

    run_match(&mut app, &mut session).unwrap();

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(stored_layer(&app, 3), LayerId(1));
    assert_eq!(
        stored_attributes(&app, 3),
        painted_attributes(1.0),
        "the ARC's own (symbolic) linetype had scale 1.0"
    );
}

#[test]
fn destination_already_matching_contributes_no_change() {
    let (mut app, mut session) = match_app();
    // Paint entity 2 once so it matches the source, then select it again together
    // with a still-mismatching destination (the ARC).
    session.selection.replace([ref_of(1), ref_of(2)]);
    run_match(&mut app, &mut session).unwrap();
    assert_eq!(stored_attributes(&app, 2), painted_attributes(2.5));

    session.selection.replace([ref_of(1), ref_of(2), ref_of(3)]);
    let before = revision(&app);
    let outcome = run_match(&mut app, &mut session).unwrap();

    // Only the ARC changes: one revision bump, one ChangeSet entry, one undo entry.
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let changes = outcome.changes.expect("the edit publishes a ChangeSet");
    assert_eq!(changes.changes.len(), 1);
    assert!(matches!(
        changes.changes[0],
        ObjectChange::Update(object, _) if object.0 == 3
    ));

    undo(&mut app, &mut session);
    assert_eq!(
        stored_attributes(&app, 3),
        EntityRenderAttributes::default(),
        "undo restores only the entity that changed"
    );
    assert_eq!(
        stored_attributes(&app, 2),
        painted_attributes(2.5),
        "the already-matching destination was never staged"
    );
    // The second command recorded exactly one entry: undoing it removed only the
    // ARC. The earlier paint is still undoable and restores entity 2.
    assert!(app.can_undo(&DocumentId(1)), "the earlier paint remains");
    undo(&mut app, &mut session);
    assert_eq!(stored_attributes(&app, 2), destination_attributes());
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn all_destinations_matching_is_a_no_op_without_an_undo_entry() {
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    run_match(&mut app, &mut session).unwrap();
    let before = revision(&app);

    let outcome = run_match(&mut app, &mut session).unwrap();

    assert!(outcome.changes.is_none(), "a no-op publishes no ChangeSet");
    assert_eq!(revision(&app), before, "no revision bump");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "matchprop.unchanged"),
        "an explicit nothing-to-change completion: {:?}",
        outcome.diagnostics
    );
    assert_eq!(stored_attributes(&app, 2), painted_attributes(2.5));
    // Undo removes the *first* paint (the only real entry), never an empty one.
    undo(&mut app, &mut session);
    assert_eq!(stored_attributes(&app, 2), destination_attributes());
    assert!(!app.can_undo(&DocumentId(1)), "no empty undo entry");
}

#[test]
fn fewer_than_two_entities_is_refused_by_name() {
    let (mut app, mut session) = match_app();
    let before = revision(&app);

    // Zero selection and a single selection are the same refusal: MATCHPROP has
    // no source-without-destination form.
    for selection in [Vec::new(), vec![ref_of(1)]] {
        session.selection.replace(selection);
        let error = run_match(&mut app, &mut session).unwrap_err();
        assert!(
            format!("{error:?}").contains("matchprop.needs_two"),
            "got {error:?}"
        );
        assert_eq!(revision(&app), before, "a refusal writes nothing");
        assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
    }
}

#[test]
fn non_addressable_source_or_destination_refuses_the_whole_command() {
    let (mut app, mut session) = match_app();
    let instance_member = SelectionRef {
        instance: InstancePath(vec![EntityId(4)]),
        ..ref_of(2)
    };
    let sub_element = SelectionRef {
        sub_element: Some(cad_domain::SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(2)
    };
    // (a) a non-addressable **source** (first selected), (b) a non-addressable
    // **destination**. Both are refused before anything is resolved or staged.
    let cases = [
        vec![instance_member.clone(), ref_of(2)],
        vec![ref_of(1), sub_element],
    ];
    for selection in cases {
        session.selection.replace(selection);
        let before = revision(&app);
        let error = run_match(&mut app, &mut session).unwrap_err();
        assert!(
            format!("{error:?}").contains("property.not_addressable"),
            "got {error:?}"
        );
        assert_eq!(revision(&app), before);
        assert_eq!(stored_layer(&app, 1), LayerId(1), "no partial write");
        assert_eq!(stored_layer(&app, 2), LayerId(0), "no partial write");
        assert_eq!(stored_attributes(&app, 2), destination_attributes());
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn a_source_value_no_destination_can_accept_is_refused_by_name() {
    // A stored `ByBlock` transparency is legal display state but has no writable
    // form in the shared grammar, so the destination could not accept it. The
    // refusal names the source entity **and** the property and keeps the
    // parser's own `property.*` code.
    let (mut app, mut session) = match_app_with_source(EntityRenderAttributes {
        transparency: EntityTransparency::ByBlock,
        ..source_attributes()
    });
    assert_eq!(
        property_value::property_value(
            &stored_drawing(&app),
            stored_drawing(&app).entity(EntityId(1)).unwrap(),
            "transparency",
        )
        .as_deref(),
        Some("ByBlock"),
        "the fixture stores a value the write grammar cannot express"
    );

    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);
    let error = run_match(&mut app, &mut session).unwrap_err();
    let message = format!("{error:?}");
    assert!(message.contains("matchprop.source_value"), "{message}");
    assert!(message.contains("source entity 1"), "{message}");
    assert!(message.contains("transparency"), "{message}");
    assert!(
        message.contains("property.invalid_value"),
        "the shared parser's code survives: {message}"
    );
    assert_eq!(revision(&app), before, "nothing written");
    assert_eq!(stored_attributes(&app, 2), destination_attributes());
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn the_first_selected_entity_is_the_source_regardless_of_id_order() {
    // Selection order, not id order, decides: the ARC (id 3) is selected first
    // and paints the two lines, even though its id is the highest.
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(3), ref_of(1), ref_of(2)]);
    run_match(&mut app, &mut session).unwrap();

    // Entity 3 is the untouched source; both destinations get its (plain) values.
    assert_eq!(stored_layer(&app, 3), LayerId(0));
    assert_eq!(
        stored_attributes(&app, 3),
        EntityRenderAttributes::default()
    );
    assert_eq!(
        stored_layer(&app, 1),
        LayerId(0),
        "the source painted layer 0"
    );
    assert_eq!(
        stored_attributes(&app, 1),
        EntityRenderAttributes::default()
    );
    assert_eq!(stored_layer(&app, 2), LayerId(0));
    assert_eq!(
        stored_attributes(&app, 2),
        EntityRenderAttributes::default(),
        "a symbolic source linetype replaces the destination's named value outright \
         (only a *named* target preserves the destination's scale)"
    );

    // The mirrored order on a fresh drawing makes the property-rich line the
    // source instead — same two entities, opposite outcome.
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(1), ref_of(3), ref_of(2)]);
    run_match(&mut app, &mut session).unwrap();
    assert_eq!(stored_layer(&app, 3), LayerId(1));
    assert_eq!(stored_attributes(&app, 3), painted_attributes(1.0));
    assert_eq!(stored_layer(&app, 2), LayerId(1));
    assert_eq!(stored_attributes(&app, 2), painted_attributes(2.5));
}

#[test]
fn a_payload_is_refused_explicitly() {
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    let before = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::MatchProp,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("matchprop.invalid_payload"),
        "got {error:?}"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn viewer_mode_is_refused_before_any_write() {
    let (mut app, mut session) = match_app();
    session.selection.replace([ref_of(1), ref_of(2)]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);

    assert!(run_match(&mut app, &mut session).is_err());

    assert_eq!(revision(&app), before);
    assert_eq!(stored_layer(&app, 2), LayerId(0));
    assert!(!app.can_undo(&DocumentId(1)));
}
