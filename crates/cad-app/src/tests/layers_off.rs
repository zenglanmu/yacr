//! `LAYOFF` / `LAYON` contract tests (layers area).
//!
//! `LAYOFF` is the sibling of `LAYISO` (`tests/layers_isolate.rs`): it turns the
//! **selection's** layers off instead of hiding everything else. Pinned here: one
//! atomic override batch, the current layer is never turned off, a
//! current-layer-only selection changes nothing and names
//! `layers.off.current_layer`, no revision bump and no history entry, an admitted
//! Viewer-mode path, and the `LAYON` restore (the existing `RESTORELAYERS` command,
//! also aliased in `cad-ui-core`) turning everything back on.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

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
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        draw_order: order,
    }
}

/// A work-mode app with layers `0`/`WALLS`(3)/`DOORS`(5), one entity on each of
/// WALLS (1) and DOORS (2), plus entity 3 on the current layer 0.
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

fn layoff(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::LayOff, CommandPayload::None)
}

/// The `LAYON` path: it is an **alias** of the existing restore command, so the
/// application-level contract is the `RESTORELAYERS` command itself.
fn layon(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::RestoreLayers, CommandPayload::None)
}

#[test]
fn turning_the_selection_off_hides_exactly_its_layers() {
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[1, 2]); // WALLS (3) and DOORS (5); current is 0

    let outcome = layoff(&mut app, &mut session).expect("LAYOFF runs");

    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(session.layer_overrides.get(LayerId(5)), Some(false));
    assert!(
        session.layer_overrides.get(LayerId(0)).is_none(),
        "the current layer is never turned off"
    );
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .filter(|d| d.code == "layers.off")
            .count(),
        1,
        "{:?}",
        outcome.diagnostics
    );
    // Session view state only: no database write and no undo entry.
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
    assert!(outcome.changes.is_none());
}

#[test]
fn the_current_layer_is_never_turned_off_even_when_selected() {
    let (mut app, mut session) = app_three_layers();
    // Entity 1 is on WALLS, entity 3 on the current layer 0.
    select(&mut app, &mut session, &[1, 3]);

    layoff(&mut app, &mut session).expect("LAYOFF runs");

    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert!(
        session.layer_overrides.get(LayerId(0)).is_none(),
        "the current layer must stay on (this build refuses to hide it on every other path)"
    );
}

#[test]
fn a_selection_only_on_the_current_layer_changes_nothing() {
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);
    select(&mut app, &mut session, &[3]); // layer 0 == current layer

    let outcome = layoff(&mut app, &mut session).expect("LAYOFF runs");

    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "layers.off.current_layer"),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        session.layer_overrides.is_empty(),
        "nothing may be turned off: {:?}",
        session.layer_overrides
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn turning_layers_off_needs_a_selection() {
    let (mut app, mut session) = app_three_layers();
    let error = layoff(&mut app, &mut session).expect_err("an empty selection is a refusal");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("layers.off.needs_selection"));
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn a_payload_is_refused_rather_than_ignored() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayOff,
        CommandPayload::LayerVisibilities(vec![(LayerId(3), false)]),
    )
    .expect_err("LAYOFF takes no payload");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn the_command_is_admitted_in_viewer_mode() {
    assert!(
        !CommandId::LayOff.requires_work_mode(),
        "LAYOFF is a session view operation, not an edit"
    );
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);
    session.switch_mode(AppMode::Viewer).expect("switch mode");
    select(&mut app, &mut session, &[1]);
    layoff(&mut app, &mut session).expect("Viewer may turn layers off");
    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(revision(&app), before_revision, "still no database write");
}

#[test]
fn layon_restores_every_layer() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, &[1, 2]);
    layoff(&mut app, &mut session).expect("LAYOFF runs");
    assert_eq!(session.layer_overrides.len(), 2);

    // `LAYON` is an alias of exactly this command (cad-ui-core), so the restore
    // half is the existing single path.
    layon(&mut app, &mut session).expect("LAYON clears the overrides");
    assert!(
        session.layer_overrides.is_empty(),
        "every layer is on again: {:?}",
        session.layer_overrides
    );
    assert_eq!(
        undo_depth(&app),
        0,
        "restoring view state records no history"
    );
}
