//! Layer-lock contracts through the **real command funnel**.
//!
//! `cad-db` owns the rule (`drawing::lock`, `layer.locked`); these tests prove a
//! user command surfaces that refusal unchanged: no document mutation, no
//! revision bump and **no undo entry**. They also pin the two documented
//! allowances (display-only property writes, and layer-table writes including
//! "make the layer current") plus the history-replay rule, which is refused once
//! the layer is locked and keeps the step on the stack.
//!
//! The entity ids and layers match the shared fixture shape: entity 1 is on
//! `WALLS` (id 3, **locked**), entity 2 is on layer 0.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, EntityColor, Layer, LayerAttributes};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128, layer: LayerId, start: Point3, end: Point3, order: i64) -> DbEntity {
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
        geometry: SemanticGeometry::Line { start, end },
        draw_order: order,
    }
}

/// A work-mode application whose `WALLS` layer is locked.
fn app_with_locked_walls() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let walls = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    b.set_layer_attributes(LayerId(3), walls).unwrap();
    b.insert_entity(line(1, LayerId(3), point(0.0, 0.0), point(10.0, 0.0), 0))
        .unwrap();
    b.insert_entity(line(2, LayerId(0), point(0.0, 10.0), point(10.0, 10.0), 1))
        .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

fn drawing(app: &Application) -> Arc<DrawingDatabase> {
    app.workspace.documents[&DocumentId(1)].drawing.clone()
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn entity(app: &Application, id: u128) -> DbEntity {
    drawing(app)
        .entity(EntityId(id))
        .unwrap_or_else(|| panic!("entity {id} is missing"))
        .clone()
}

fn line_end(app: &Application, id: u128) -> Point3 {
    match entity(app, id).geometry {
        SemanticGeometry::Line { end, .. } => end,
        other => panic!("entity {id} is not a line: {other:?}"),
    }
}

/// Rewrite `WALLS`' lock flag through the layer-table write (the only sanctioned
/// way to unlock), used to set up the "legal when it was made" history state.
fn set_walls_locked(app: &mut Application, locked: bool) {
    let mut database = drawing(app).as_ref().clone();
    let mut attributes = database.layer_attributes(LayerId(3));
    attributes.locked = locked;
    database
        .set_layer_attributes(
            LayerId(3),
            attributes,
            TransactionId(900),
            if locked { "lock" } else { "unlock" },
        )
        .expect("a layer-table attribute write is always allowed");
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(database);
}

fn assert_locked_refusal(error: &CadError, layer: &str) {
    let text = error.to_string();
    assert!(text.contains("layer.locked"), "stable code missing: {text}");
    assert!(text.contains(layer), "the refusal names the layer: {text}");
    assert!(
        text.contains("unlock the layer first"),
        "the refusal says how to proceed: {text}"
    );
}

#[test]
fn move_of_a_locked_selection_is_refused_with_no_undo_entry_and_no_partial_write() {
    let (mut app, mut session) = app_with_locked_walls();
    let before = revision(&app);
    let locked_before = entity(&app, 1);
    let free_before = entity(&app, 2);

    // The mixed selection is the important case: entity 2 (unlocked) is staged
    // before entity 1 (locked) fails, so the whole command must still leave no
    // trace — no half-applied MOVE, no revision bump, no undo step.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(2), ref_of(1)],
            delta: p3(5.0, 0.0, 0.0),
        },
    )
    .expect_err("entity 1 is on the locked WALLS layer");
    assert_locked_refusal(&error, "WALLS");
    assert!(
        error.to_string().contains("entity 1"),
        "the refusal names the entity: {error}"
    );

    assert_eq!(revision(&app), before);
    assert_eq!(entity(&app, 1), locked_before);
    assert_eq!(entity(&app, 2), free_before, "the free entity did not move");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry was recorded");
}

#[test]
fn erase_of_a_locked_entity_is_refused_with_no_undo_entry() {
    let (mut app, mut session) = app_with_locked_walls();
    let before = revision(&app);
    let stored = entity(&app, 1);
    session.selection.replace([ref_of(1)]);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .expect_err("the entity is on the locked WALLS layer");
    assert_locked_refusal(&error, "WALLS");
    assert_eq!(revision(&app), before);
    assert_eq!(entity(&app, 1), stored);
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry was recorded");
    assert_eq!(
        session.selection.len(),
        1,
        "a refused erase does not reconcile the selection away"
    );
}

#[test]
fn moving_an_entity_onto_a_locked_layer_is_refused() {
    let (mut app, mut session) = app_with_locked_walls();
    let before = revision(&app);
    session.selection.replace([ref_of(2)]);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: "layer".into(),
            value: "WALLS".into(),
        },
    )
    .expect_err("the target layer is locked");
    assert_locked_refusal(&error, "WALLS");
    assert_eq!(revision(&app), before);
    assert_eq!(
        entity(&app, 2).layer,
        LayerId(0),
        "the entity was not retargeted"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_display_only_property_change_is_allowed_on_a_locked_layer() {
    // Colour is a display attribute (`update_entity_display`), not an entity-row
    // edit, so a locked layer still regenerates.
    let (mut app, mut session) = app_with_locked_walls();
    let before = revision(&app);
    let stored = entity(&app, 1);
    session.selection.replace([ref_of(1)]);

    execute(
        &mut app,
        &mut session,
        CommandId::SetEntityProperty,
        CommandPayload::EntityProperty {
            key: "color".into(),
            value: "aci:1".into(),
        },
    )
    .expect("a display-only write is allowed on a locked layer");

    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(
        entity(&app, 1),
        stored,
        "the entity row (geometry/layer) is untouched"
    );
    assert_eq!(
        drawing(&app).entity_render_attributes(EntityId(1)).color,
        EntityColor::Explicit([255, 0, 0]),
        "the display attribute did change"
    );
    assert!(
        app.can_undo(&DocumentId(1)),
        "the display edit is a real, undoable step"
    );
}

#[test]
fn creating_a_line_on_a_locked_current_layer_is_refused() {
    let (mut app, mut session) = app_with_locked_walls();
    // Making a locked layer current is session state and is **not** refused by
    // this lane (documented gap); the creation it enables is refused.
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(3)),
    )
    .expect("the active layer is session state");
    assert_eq!(session.active_layer, LayerId(3));

    let before = revision(&app);
    let count = drawing(&app).entity_count();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 20.0), point(10.0, 20.0)]),
    )
    .expect_err("a new entity must not appear on a locked layer");
    assert_locked_refusal(&error, "WALLS");

    assert_eq!(revision(&app), before);
    assert_eq!(drawing(&app).entity_count(), count, "nothing was inserted");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry was recorded");
}

#[test]
fn undo_of_a_step_now_on_a_locked_layer_is_refused_and_the_step_is_kept() {
    let (mut app, mut session) = app_with_locked_walls();
    // Record the MOVE while the layer is still writable …
    set_walls_locked(&mut app, false);
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: p3(5.0, 0.0, 0.0),
        },
    )
    .expect("the layer was unlocked when the move was made");
    let moved_end = line_end(&app, 1);
    assert_eq!(moved_end, point(15.0, 0.0));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // … then lock the layer. Replaying the `before` side is now refused: the
    // current lock state wins (documented rule), nothing is muted and the step
    // stays on the stack instead of being silently dropped.
    set_walls_locked(&mut app, true);
    let locked_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect_err("undo would write to a now-locked layer");
    assert_locked_refusal(&error, "WALLS");

    assert_eq!(revision(&app), locked_revision);
    assert_eq!(
        line_end(&app, 1),
        moved_end,
        "the drawing is exactly as it was before the refused undo"
    );
    assert!(
        app.can_undo(&DocumentId(1)),
        "the refused step was put back, not dropped"
    );
    assert!(!app.can_redo(&DocumentId(1)));

    // Unlocking makes the very same undo work.
    set_walls_locked(&mut app, false);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the layer is writable again");
    assert_eq!(line_end(&app, 1), point(10.0, 0.0));
    assert!(!app.can_undo(&DocumentId(1)));
}
