//! `LAYISO` / `LAYUNISO` contract tests (layers area).
//!
//! Pins that isolation is a **session view operation**: one atomic override batch,
//! no database revision bump, no history/undo entry, the current layer always kept
//! visible, a no-op that writes nothing, and an admitted Viewer-mode path. The
//! restore half is the existing `RESTORELAYERS` command (`LAYUNISO`/`LAYON` are
//! its aliases, pinned in `cad-ui-core`; `LAYERP` is its **own** previous-state
//! command and is pinned in `tests::layers_previous`).

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(x0, y0),
        end: point(x1, y1),
    }
}

fn entity(id: u128, layer: LayerId, order: i64) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry: line(0.0, 0.0, 10.0, 0.0),
        draw_order: order,
    }
}

/// A work-mode app with layers `0`/`WALLS`(3)/`DOORS`(5) and one entity on each of
/// WALLS (1) and DOORS (2), plus entity 3 on layer 0.
fn app_three_layers() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    for (id, layer, order) in [
        (1u128, LayerId(3), 0i64),
        (2, LayerId(5), 1),
        (3, LayerId(0), 2),
    ] {
        b.insert_entity(entity(id, layer, order)).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([41u8; 32]),
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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
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

fn isolate(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::IsolateLayers, CommandPayload::None)
}

#[test]
fn isolating_the_selection_hides_every_other_layer_atomically() {
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1]); // WALLS (3); current layer is 0

    let outcome = isolate(&mut app, &mut session).expect("LAYISO runs");

    // keep = {WALLS (selected), 0 (current)}; everything else is hidden.
    assert_eq!(session.layer_overrides.get(LayerId(5)), Some(false));
    assert_eq!(
        session.layer_overrides.get(LayerId(0)),
        None,
        "the current layer is never overridden invisible"
    );
    assert_eq!(session.layer_overrides.get(LayerId(3)), None);
    assert_eq!(session.layer_overrides.len(), 1);

    // Session state only: no database revision, no entity change, no undo entry.
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), 0, "no history entry");
    assert!(outcome.changes.is_none(), "no ChangeSet");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        3,
        "nothing written to the database"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "layers.isolate.current_layer"));
}

#[test]
fn the_current_layer_is_kept_visible_even_with_nothing_selected_on_it() {
    let (mut app, mut session) = app_three_layers();
    session.active_layer = LayerId(5); // DOORS, no selected entity on it
    select(&mut app, &mut session, &[1]); // WALLS
    let outcome = isolate(&mut app, &mut session).expect("LAYISO runs");

    // keep = {DOORS (current), WALLS}; layer 0 is the one now hidden.
    assert_eq!(session.layer_overrides.get(LayerId(0)), Some(false));
    assert_eq!(
        session.layer_overrides.get(LayerId(5)),
        None,
        "the current layer stays visible (our rule)"
    );
    assert_eq!(session.layer_overrides.get(LayerId(3)), None);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "layers.isolate.current_layer"),
        "the kept current layer is explained: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn every_layer_already_kept_is_a_noop_that_writes_no_override() {
    // Only layers 0 and WALLS exist; selecting the WALLS entity with 0 current
    // puts every layer in the keep-set, so there is nothing to hide.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_entity(entity(1, LayerId(3), 0)).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([42u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);

    select(&mut app, &mut session, &[1]);
    let outcome = isolate(&mut app, &mut session).expect("LAYISO runs");

    assert!(
        session.layer_overrides.is_empty(),
        "no churn: no override was written"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "layers.isolate.noop"));
}

#[test]
fn an_empty_selection_is_refused_and_writes_nothing() {
    let (mut app, mut session) = app_three_layers();
    let error = isolate(&mut app, &mut session).expect_err("isolating nothing is refused");
    assert!(
        matches!(error, CadError::InvalidInput(ref reason)
            if reason.contains("layers.isolate.needs_selection")),
        "{error:?}"
    );
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn restore_layers_clears_the_isolation() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, &[1]);
    isolate(&mut app, &mut session).expect("LAYISO runs");
    assert_eq!(session.layer_overrides.len(), 1);

    // `LAYUNISO` is an alias of exactly this command (cad-ui-core); `LAYERP` is
    // the separate previous-state command (`tests::layers_previous`).
    execute(
        &mut app,
        &mut session,
        CommandId::RestoreLayers,
        CommandPayload::None,
    )
    .expect("the restore path clears the overrides");
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn the_command_is_admitted_in_viewer_mode() {
    assert!(
        !CommandId::IsolateLayers.requires_work_mode(),
        "LAYISO is a session view operation, not an edit"
    );
    let (mut app, mut session) = app_three_layers();
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    select(&mut app, &mut session, &[1]);
    isolate(&mut app, &mut session).expect("Viewer may isolate layers");
    assert_eq!(session.layer_overrides.get(LayerId(5)), Some(false));
    assert_eq!(revision(&app), Revision(0), "still no database write");
}

#[test]
fn a_payload_is_refused_rather_than_ignored() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::IsolateLayers,
        CommandPayload::LayerVisibilities(vec![(LayerId(5), false)]),
    )
    .expect_err("LAYISO takes no payload");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(session.layer_overrides.is_empty());
}

/// Layers `0`(0)/WALLS(3)/DOORS(5); block 7 holds entity 10 on layer `0` and
/// entity 11 on DOORS; model INSERT 20 (on WALLS) places that block.
fn app_with_a_layer_zero_block_member() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_block(cad_db::BlockDefinition {
        id: BlockId(7),
        entities: vec![EntityId(10), EntityId(11)],
        dynamic_visibility: None,
    })
    .unwrap();
    for (id, layer) in [(10u128, LayerId(0)), (11, LayerId(5))] {
        let mut child = entity(id, layer, 0);
        child.space = SpaceId::Block(BlockId(7));
        b.insert_entity(child).unwrap();
    }
    let mut insert = entity(20, LayerId(3), 1);
    insert.geometry = SemanticGeometry::Insert {
        block: BlockId(7),
        transform: Transform3::identity(),
    };
    b.insert_entity(insert).unwrap();

    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([43u8; 32]),
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

/// The selection ref of block member `member` through INSERT `insert`.
fn block_member_ref(insert: u128, member: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(member),
        instance: InstancePath(vec![EntityId(insert)]),
        sub_element: None,
    }
}

#[test]
fn layiso_keeps_the_insert_layer_of_a_layer_zero_block_member() {
    // A member stored on layer `0` belongs to its INSERT's layer (WALLS), not to
    // layer `0`: isolating it must keep WALLS and hide layer 0.
    let (mut app, mut session) = app_with_a_layer_zero_block_member();
    session.active_layer = LayerId(5); // DOORS, current, not selected

    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![block_member_ref(20, 10)]),
    )
    .unwrap();
    let outcome = isolate(&mut app, &mut session).expect("LAYISO runs");

    assert_eq!(
        session.layer_overrides.get(LayerId(0)),
        Some(false),
        "layer 0 is hidden: the member belongs to WALLS, not to layer 0"
    );
    assert_eq!(
        session.layer_overrides.get(LayerId(3)),
        None,
        "the INSERT's layer is kept visible"
    );
    assert_eq!(session.layer_overrides.get(LayerId(5)), None);
    assert_eq!(session.layer_overrides.len(), 1);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "layers.isolate.current_layer"));
    assert_eq!(undo_depth(&app), 0, "still no history entry");
}

#[test]
fn layoff_turns_off_the_insert_layer_of_a_layer_zero_block_member() {
    // `LAYOFF` shares `selection_layers`, so the same effective-layer rule picks
    // WALLS to turn off — never layer `0`.
    let (mut app, mut session) = app_with_a_layer_zero_block_member();
    session.active_layer = LayerId(5); // DOORS is current, so it is never turned off

    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![block_member_ref(20, 10)]),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::LayOff,
        CommandPayload::None,
    )
    .expect("LAYOFF runs");

    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(
        session.layer_overrides.get(LayerId(0)),
        None,
        "layer 0 was never the selection's layer"
    );
}
