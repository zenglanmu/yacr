//! Layer table command contracts (`CREATELAYER` / `RENAMELAYER` / `DELETELAYER`).
//!
//! These exercise the real command funnel (`Application::execute`) on a document
//! shaped like the shared fixture (layer `0` plus `WALLS`) **with one line on
//! `WALLS`**, so the delete-in-use and rename-reflects-on-the-panel rules are
//! pinned against a real referencing entity.
//!
//! The database's own rules (name grammar, id allocation, case-insensitive
//! uniqueness) are covered by `cad-db`; here the contract is the command layer:
//! explicit refusals with stable reasons, one revision bump, no undo entry, the
//! entity property label following a rename, and the active-layer fallback.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer, LayerAttributes};

/// Layers `0` (id 0) and `WALLS` (id 3), one model-space line on `WALLS`, and
/// `WALLS` carrying the given stored attributes.
///
/// `Layer::visible` is derived from the attributes exactly as the database does
/// (`!(off || frozen)`), because the builder's attribute write is storage only —
/// the importer is the authority for the flag.
pub(super) fn app_with_walls_attributes(
    attributes: LayerAttributes,
) -> (Application, SessionState) {
    app_with_layers(attributes, false)
}

/// [`app_with_walls_attributes`] plus a third, plain layer `DOORS` (id 5) as an
/// eligible target for the success path.
fn app_with_walls_and_doors(attributes: LayerAttributes) -> (Application, SessionState) {
    app_with_layers(attributes, true)
}

fn app_with_layers(attributes: LayerAttributes, doors: bool) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    let mut layers = vec![(LayerId(0), "0"), (LayerId(3), "WALLS")];
    if doors {
        layers.push((LayerId(5), "DOORS"));
    }
    for (id, name) in layers {
        let visible = if id == LayerId(3) {
            attributes.is_visible()
        } else {
            true
        };
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible,
        })
        .unwrap();
    }
    b.set_layer_attributes(LayerId(3), attributes).unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(3),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    (app, session)
}

/// The same fixture with `WALLS` on, thawed and unlocked.
fn app_with_entity_on_walls() -> (Application, SessionState) {
    app_with_walls_attributes(LayerAttributes::default())
}

pub(super) fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

pub(super) fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

pub(super) fn drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

fn layer_names(app: &Application) -> Vec<String> {
    let mut names: Vec<String> = drawing(app).layers().map(|row| row.name.clone()).collect();
    names.sort();
    names
}

fn layer_id(app: &Application, name: &str) -> LayerId {
    drawing(app)
        .layers()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no layer named {name}"))
        .id
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

#[test]
fn create_layer_adds_a_row_in_one_revision_bump_and_no_undo_entry() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLayer,
        CommandPayload::CreateLayer("DOORS".into()),
    )
    .expect("a fresh legal name is accepted");

    assert_eq!(layer_names(&app), vec!["0", "DOORS", "WALLS"]);
    assert_eq!(revision(&app), Revision(before.0 + 1));
    let changes = outcome.changes.expect("a revision-tagged ChangeSet");
    assert_eq!(changes.after, revision(&app));
    assert!(
        changes.changes.is_empty(),
        "layer edits are revision-tagged, not entity-addressed"
    );
    assert_eq!(outcome.diagnostics[0].code, "drawing.layer_created");
    // The session's active layer is untouched by a structural create.
    assert_eq!(session.active_layer, LayerId(0));
    // Not undoable: layer edits record no history entry.
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_layer_refuses_a_duplicate_name_and_an_empty_name_without_writing() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);
    let rows = layer_names(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateLayer,
        CommandPayload::CreateLayer("walls".into()),
    )
    .expect_err("WALLS exists; names are case-insensitive");
    assert!(
        error.to_string().contains("layer.duplicate_name"),
        "{error}"
    );

    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateLayer,
        CommandPayload::CreateLayer("   ".into()),
    )
    .expect_err("a whitespace-only name is refused");
    assert!(error.to_string().contains("layer.name_empty"), "{error}");

    assert_eq!(revision(&app), before);
    assert_eq!(layer_names(&app), rows);
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

#[test]
fn rename_layer_makes_the_entity_property_label_follow_and_retires_the_old_name() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::RenameLayer,
        CommandPayload::RenameLayer {
            layer: "WALLS".into(),
            name: " PARTITIONS ".into(),
        },
    )
    .expect("a fresh legal name is accepted");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    // The panel's layer row is derived from the database, so the entity's
    // property label follows the rename with no second source of truth.
    let db = drawing(&app);
    let entity = db.entity(EntityId(1)).expect("the line").clone();
    let rows = crate::selection::entity_property_rows(
        &db,
        &SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        },
        &entity,
    );
    let layer_row = rows
        .iter()
        .find(|row| row.key == "layer")
        .expect("layer row");
    assert_eq!(layer_row.value.as_deref(), Some("PARTITIONS (3)"));

    // The old name no longer resolves; the new one resolves
    // case-insensitively (and is then refused as in-use, proving resolution).
    let error = execute(
        &mut app,
        &mut session,
        CommandId::RenameLayer,
        CommandPayload::RenameLayer {
            layer: "WALLS".into(),
            name: "OTHER".into(),
        },
    )
    .expect_err("WALLS no longer exists");
    assert!(error.to_string().contains("layer.unknown"), "{error}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DeleteLayer,
        CommandPayload::DeleteLayer {
            layer: "partitions".into(),
        },
    )
    .expect_err("entity 1 is still on the renamed layer");
    assert!(error.to_string().contains("layer.in_use"), "{error}");
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[test]
fn delete_layer_in_use_is_refused_and_the_revision_is_unchanged() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);
    let rows = layer_names(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DeleteLayer,
        CommandPayload::DeleteLayer {
            layer: "WALLS".into(),
        },
    )
    .expect_err("entity 1 references WALLS");
    let text = error.to_string();
    assert!(text.contains("layer.in_use"), "{text}");
    assert!(text.contains('1'), "the refusal names the count: {text}");

    assert_eq!(revision(&app), before);
    assert_eq!(layer_names(&app), rows);
    assert_eq!(
        drawing(&app).entity(EntityId(1)).expect("the line").layer,
        LayerId(3),
        "no entity was silently retargeted"
    );
}

#[test]
fn deleting_the_active_layer_falls_back_to_layer_zero_and_prunes_its_override() {
    let (mut app, mut session) = app_with_entity_on_walls();
    execute(
        &mut app,
        &mut session,
        CommandId::CreateLayer,
        CommandPayload::CreateLayer("DOORS".into()),
    )
    .expect("DOORS is free");
    let doors = layer_id(&app, "DOORS");
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(doors),
    )
    .expect("the session may make the new layer current");
    // A temporary override naming the same row must not outlive the row.
    session.layer_overrides.set(doors, false);
    let before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::DeleteLayer,
        CommandPayload::DeleteLayer {
            layer: "DOORS".into(),
        },
    )
    .expect("nothing references DOORS");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert!(!layer_names(&app).contains(&"DOORS".to_string()));
    assert_eq!(
        session.active_layer,
        LayerId(0),
        "the session never keeps a dangling active layer"
    );
    assert!(
        session.layer_overrides.get(doors).is_none(),
        "the override for the deleted row is pruned"
    );
    let fallback = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.active_layer_fallback")
        .expect("the fallback is reported, never silent");
    assert!(fallback.message.contains("'0'"), "{}", fallback.message);
}

#[test]
fn deleting_layer_zero_is_refused() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DeleteLayer,
        CommandPayload::DeleteLayer { layer: "0".into() },
    )
    .expect_err("the default layer cannot be deleted");
    assert!(
        error.to_string().contains("layer.default_undeletable"),
        "{error}"
    );

    assert_eq!(revision(&app), before);
    assert!(layer_names(&app).contains(&"0".to_string()));
}

#[test]
fn an_unknown_layer_name_is_refused_by_name_and_changes_nothing() {
    let (mut app, mut session) = app_with_entity_on_walls();
    let before = revision(&app);
    let rows = layer_names(&app);

    for payload in [
        CommandPayload::RenameLayer {
            layer: "ROOF".into(),
            name: "TOP".into(),
        },
        CommandPayload::DeleteLayer {
            layer: "ROOF".into(),
        },
    ] {
        let id = match &payload {
            CommandPayload::RenameLayer { .. } => CommandId::RenameLayer,
            _ => CommandId::DeleteLayer,
        };
        let error = execute(&mut app, &mut session, id, payload)
            .expect_err("ROOF does not exist in this drawing");
        let text = error.to_string();
        assert!(text.contains("layer.unknown"), "{text}");
        assert!(text.contains("ROOF"), "the refusal names the layer: {text}");
    }

    assert_eq!(revision(&app), before);
    assert_eq!(layer_names(&app), rows);
}

// ---------------------------------------------------------------------------
// Active (current) layer eligibility
// ---------------------------------------------------------------------------

/// Every eligibility refusal is: a stable code, a sentence naming the layer and an
/// English remedy — and **no** state change at all (no session write, no revision
/// bump, no history entry).
fn assert_not_made_current(
    app: &mut Application,
    session: &mut SessionState,
    code: &str,
    remedy: &str,
) {
    let before = revision(app);
    let error = execute(
        app,
        session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .expect_err("this layer cannot be the current layer");
    let text = error.to_string();
    assert!(text.contains(code), "stable code missing: {text}");
    assert!(
        text.contains("WALLS"),
        "the refusal names the layer: {text}"
    );
    assert!(
        text.contains(remedy),
        "the refusal says how to proceed: {text}"
    );
    assert_eq!(
        session.active_layer,
        LayerId(0),
        "the active layer is unchanged"
    );
    assert_eq!(revision(app), before, "a refusal is not a database write");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a refusal records no history entry"
    );
}

#[test]
fn making_a_locked_layer_current_is_refused() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    });
    assert_not_made_current(
        &mut app,
        &mut session,
        "layer.current_locked",
        "unlock it before making it the current layer",
    );
}

#[test]
fn making_a_frozen_layer_current_is_refused() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        frozen: true,
        ..LayerAttributes::default()
    });
    assert_not_made_current(
        &mut app,
        &mut session,
        "layer.current_frozen",
        "thaw it before making it the current layer",
    );
}

#[test]
fn making_an_off_layer_current_is_refused() {
    // Decision (this build's rule, deliberately not AutoCAD's): an off layer is
    // **invisible** here (`Layer::visible == !(off || frozen)`) and the shared
    // create funnel does not look at visibility, so an off current layer would
    // accept geometry the user can neither see nor pick. AutoCAD lets an off layer
    // be current (it does not turn it on either); copying that would silently
    // hide what the user just drew, so the off case is refused instead.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        off: true,
        ..LayerAttributes::default()
    });
    assert_not_made_current(
        &mut app,
        &mut session,
        "layer.current_off",
        "turn it on before making it the current layer",
    );
    assert!(
        !drawing(&app).layer(LayerId(3)).unwrap().visible,
        "the fixture really models an invisible off layer"
    );
}

#[test]
fn the_first_stored_flag_wins_when_a_layer_carries_several() {
    // locked → frozen → off: one deterministic precedence, so the reported reason
    // never depends on attribute iteration order.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        locked: true,
        frozen: true,
        off: true,
        ..LayerAttributes::default()
    });
    assert_not_made_current(
        &mut app,
        &mut session,
        "layer.current_locked",
        "unlock it before making it the current layer",
    );
}

#[test]
fn an_on_thawed_unlocked_layer_still_becomes_current_with_no_database_write() {
    let (mut app, mut session) = app_with_walls_and_doors(LayerAttributes::default());
    let before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(5)),
    )
    .expect("DOORS is on, thawed and unlocked");

    assert_eq!(session.active_layer, LayerId(5));
    assert_eq!(revision(&app), before, "session state only, no DB write");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "session state only, no history entry"
    );
    assert_eq!(outcome.diagnostics[0].code, "drawing.active_layer");
    assert!(outcome.changes.is_none(), "no ChangeSet is published");
}

#[test]
fn a_refused_current_layer_leaves_the_previous_layer_fully_usable() {
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    });
    let before = revision(&app);
    let count = drawing(&app).entity_count();

    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .expect_err("WALLS is locked");
    assert_eq!(session.active_layer, LayerId(0));

    // The refused transition has no collateral effect: drawing on the layer that
    // is still current works normally (one entity, one revision bump, one undo
    // step) and the locked layer is untouched.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 20.0), point(10.0, 20.0)]),
    )
    .expect("layer 0 is still the current layer and is writable");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(drawing(&app).entity_count(), count + 1);
    let db = drawing(&app);
    let created = db
        .model_space()
        .iter()
        .max_by_key(|entity| entity.draw_order)
        .cloned()
        .expect("the new line");
    assert_eq!(
        created.layer,
        LayerId(0),
        "it landed on the unchanged layer"
    );
    assert_eq!(outcome.objects, vec![ObjectId(created.id.0)]);
    assert!(app.can_undo(&DocumentId(1)), "one undo step was recorded");
    assert_eq!(
        drawing(&app)
            .entity(EntityId(1))
            .expect("the WALLS line")
            .layer,
        LayerId(3),
        "the locked layer's entity is untouched"
    );
}

#[test]
fn a_layer_can_still_be_locked_or_frozen_while_it_is_not_current() {
    // The panel's chip toggles stay available: only *becoming current* is refused,
    // so the layer-table write that freezes/locks a non-current layer still works.
    let (mut app, mut session) = app_with_walls_attributes(LayerAttributes::default());
    let before = revision(&app);

    for (key, value) in [("locked", "true"), ("frozen", "true")] {
        execute(
            &mut app,
            &mut session,
            CommandId::SetLayerProperty,
            CommandPayload::SetLayerProperty {
                layer: "WALLS".into(),
                key: key.into(),
                value: value.into(),
            },
        )
        .unwrap_or_else(|error| {
            panic!("'{key}' must stay writable on a non-current layer: {error}")
        });
    }

    let attributes = drawing(&app).layer_attributes(LayerId(3));
    assert!(attributes.locked && attributes.frozen);
    assert!(
        !drawing(&app).layer(LayerId(3)).unwrap().visible,
        "the frozen layer is hidden through the existing visibility path"
    );
    assert_eq!(
        session.active_layer,
        LayerId(0),
        "a layer-table write never moves the current layer"
    );
    assert!(revision(&app).0 > before.0, "the writes are real changes");
}

// ---------------------------------------------------------------------------
// Command layer
// ---------------------------------------------------------------------------

#[test]
fn the_layer_table_commands_are_work_only() {
    for id in [
        CommandId::CreateLayer,
        CommandId::RenameLayer,
        CommandId::DeleteLayer,
    ] {
        assert!(id.requires_work_mode(), "{id:?} must be Work-only");
    }

    let (mut app, mut session) = app_with_entity_on_walls();
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateLayer,
        CommandPayload::CreateLayer("DOORS".into()),
    )
    .expect_err("Viewer mode refuses a database write");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}
