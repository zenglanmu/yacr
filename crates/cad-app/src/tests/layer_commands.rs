//! Layer **selection** command contracts (AutoCAD 2026).
//!
//! These commands are **pre-selection based** in this build: the fixed capture
//! machine cannot ask AutoCAD's *Select objects* prompt, so the current session
//! selection is the whole input (`LAYCUR` here; the rest of the run is added
//! beside it). Pinned per command: the success shape, one transaction / one
//! undo entry where entity rows are written, **no** undo entry for layer-table
//! edits, every refusal leaving revision + history untouched, the
//! addressability preflight, the current-layer refusal for LAYFRZ, the locked
//! destination refusal and (later) SETBYLAYER's preserved fields.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, LayerAttributes, Layout};

fn line(id: u128, layer: LayerId, y: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, y),
            end: point(10.0, y),
        },
        draw_order: id as i64,
    }
}

/// A work-mode application with exactly `layers` and `entities`.
fn app_with(
    layers: Vec<(LayerId, &str, LayerAttributes)>,
    entities: Vec<DbEntity>,
) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name, attributes) in layers {
        let visible = attributes.is_visible();
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
        builder.set_layer_attributes(id, attributes).unwrap();
    }
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();

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

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn select(app: &mut Application, session: &mut SessionState, entities: &[u128]) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(entities.iter().copied().map(ref_of).collect()),
    )
    .unwrap();
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn state(app: &Application) -> (Revision, usize) {
    (revision(app), undo_depth(app))
}

fn assert_untouched(app: &Application, before: (Revision, usize)) {
    assert_eq!(revision(app), before.0, "a refusal bumps no revision");
    assert_eq!(undo_depth(app), before.1, "a refusal records no undo entry");
}

fn refusal(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn entity_layer(app: &Application, entity: u128) -> LayerId {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} must exist"))
        .layer
}

fn diagnostic_codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// LAYCUR
// ---------------------------------------------------------------------------

#[test]
fn laycur_moves_every_selected_entity_to_the_current_layer() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(3), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect("LAYCUR moves the selection");

    assert_eq!(entity_layer(&app, 1), LayerId(0));
    assert_eq!(entity_layer(&app, 2), LayerId(0));
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one revision bump"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(outcome.objects.len(), 2);
    assert!(outcome.changes.is_some());
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["laycur.applied"],
        "the success diagnostic names the layer and count"
    );
    assert!(outcome.diagnostics[0].message.contains("'0'"));
    assert!(outcome.diagnostics[0].message.contains('2'));
}

#[test]
fn laycur_is_a_noop_when_every_entity_is_already_on_the_current_layer() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(0), 0.0), line(2, LayerId(0), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect("LAYCUR reports the no-op");

    assert_eq!(state(&app), before, "no transaction, no undo entry");
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laycur.unchanged"]);
}

#[test]
fn laycur_refuses_an_empty_selection_without_touching_anything() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laycur.selection:"));
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_a_missing_current_layer() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    session.active_layer = LayerId(99);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a missing active layer is a refusal");
    assert!(refusal(error).starts_with("laycur.active_layer_missing:"));
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_moving_entities_onto_a_locked_layer() {
    let locked = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", locked),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("the database refuses a write on a locked layer");
    assert!(refusal(error).contains("layer.locked"));
    assert_eq!(entity_layer(&app, 1), LayerId(3), "nothing moved");
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_foreign_instance_and_non_model_refs() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(0), 0.0)],
    );
    session.active_layer = LayerId(3);

    let foreign = SelectionRef {
        document: DocumentId(2),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([foreign]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("another document's ref is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);

    let paper = DbEntity {
        space: SpaceId::Paper(LayoutId(1)),
        ..line(1, LayerId(0), 0.0)
    };
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new({
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
        builder.insert_entity(paper).unwrap();
        builder.finish().unwrap()
    });
    session.selection = SelectionSet::from_refs([ref_of(1)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a paper-space entity is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);
}

// ---------------------------------------------------------------------------
// LAYM C U R
// ---------------------------------------------------------------------------

#[test]
fn laymcur_makes_the_first_selected_objects_layer_current() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(0), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads the first pick");

    assert_eq!(session.active_layer, LayerId(3));
    assert_eq!(state(&app), before, "session-only: no revision, no undo");
    assert!(outcome.changes.is_none());
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["laymcur.applied", "laymcur.first_pick"]
    );
    assert!(outcome.diagnostics[0].message.contains("WALLS"));

    // The pick order is the contract: reversing the selection changes the answer.
    select(&mut app, &mut session, &[2, 1]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads the new first pick");
    assert_eq!(session.active_layer, LayerId(0));
}

#[test]
fn laymcur_refuses_an_empty_selection() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laymcur.selection:"));
    assert_untouched(&app, before);
    assert_eq!(session.active_layer, LayerId(0));
}

#[test]
fn laymcur_refuses_a_locked_frozen_or_off_first_layer() {
    for (attributes, code) in [
        (
            LayerAttributes {
                locked: true,
                ..LayerAttributes::default()
            },
            "layer.current_locked",
        ),
        (
            LayerAttributes {
                frozen: true,
                ..LayerAttributes::default()
            },
            "layer.current_frozen",
        ),
        (
            LayerAttributes {
                off: true,
                ..LayerAttributes::default()
            },
            "layer.current_off",
        ),
    ] {
        let (mut app, mut session) = app_with(
            vec![
                (LayerId(0), "0", LayerAttributes::default()),
                (LayerId(3), "WALLS", attributes),
            ],
            vec![line(1, LayerId(3), 0.0)],
        );
        select(&mut app, &mut session, &[1]);
        let before = state(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::LayMCur,
            CommandPayload::None,
        )
        .expect_err("an unusable layer cannot become current");
        assert!(
            refusal(error).starts_with(code),
            "expected {code}, got something else"
        );
        assert_eq!(session.active_layer, LayerId(0), "the session is untouched");
        assert_untouched(&app, before);
    }
}

#[test]
fn laymcur_refuses_non_addressable_refs() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laymcur.not_addressable:"));
    assert_untouched(&app, before);
}
