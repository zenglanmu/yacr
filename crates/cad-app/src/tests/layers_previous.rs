//! `LAYERP` (previous layer state) contract tests.
//!
//! `LAYERP` used to be an alias of `RESTORELAYERS`, i.e. it cleared every override
//! instead of restoring the previous state. It is now a real command
//! ([`CommandId::LayerPrevious`]) backed by a bounded, session-scoped stack of
//! layer-override snapshots ([`cad_app::LayerOverrideHistory`]).
//!
//! Pinned here: the push/pop round trip, the cap and its eviction order, the
//! empty-history diagnostic (a no-op, not an error), that a mutation which changes
//! nothing pushes **no** phantom step, that the formerly-isolated state comes back,
//! and that none of it writes the drawing, bumps the revision or records an undo
//! entry.

use super::*;
use crate::layers::LayerOverrideSet;
use crate::{LayerOverrideHistory, LAYER_OVERRIDE_HISTORY_CAP};
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

/// A Work-mode app with layers `0`/`WALLS`(3)/`DOORS`(5) and one entity on each of
/// WALLS (1), DOORS (2) and the current layer 0 (3).
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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn set_visible(
    app: &mut Application,
    session: &mut SessionState,
    layer: LayerId,
    visible: bool,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::ToggleLayer,
        CommandPayload::Layer(layer, visible),
    )
}

fn layerp(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::LayerPrevious, CommandPayload::None)
}

fn digest_codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

#[test]
fn a_single_toggle_and_layerp_are_a_round_trip() {
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);

    set_visible(&mut app, &mut session, LayerId(3), false).expect("hide WALLS");
    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(session.layer_history_len(), 1);

    layerp(&mut app, &mut session).expect("LAYERP runs");

    assert!(
        session.layer_overrides.is_empty(),
        "the previous state had no override on WALLS"
    );
    assert_eq!(session.layer_history_len(), 0, "LAYERP pops, never pushes");
    // Session view state only: no database write, no revision bump, no undo entry.
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn the_batch_path_records_one_step_per_real_change() {
    let (mut app, mut session) = app_three_layers();

    execute(
        &mut app,
        &mut session,
        CommandId::SetLayerVisibilities,
        CommandPayload::LayerVisibilities(vec![(LayerId(3), false), (LayerId(5), false)]),
    )
    .expect("one atomic batch");
    assert_eq!(session.layer_history_len(), 1, "one batch is one step");

    layerp(&mut app, &mut session).expect("LAYERP runs");
    assert!(
        session.layer_overrides.is_empty(),
        "both hides undone at once"
    );

    // A second press has nothing left.
    let outcome = layerp(&mut app, &mut session).expect("an empty history is a no-op");
    assert_eq!(digest_codes(&outcome), vec!["layers.previous.empty"]);
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn a_mutation_that_changes_nothing_pushes_no_phantom_step() {
    let (mut app, mut session) = app_three_layers();

    // Hiding an already-hidden layer changes nothing -> no step.
    set_visible(&mut app, &mut session, LayerId(3), false).expect("hide WALLS");
    set_visible(&mut app, &mut session, LayerId(3), false).expect("hide it again");
    assert_eq!(session.layer_history_len(), 1, "the repeat pushed nothing");

    // Re-publishing the identical batch changes nothing either.
    execute(
        &mut app,
        &mut session,
        CommandId::SetLayerVisibilities,
        CommandPayload::LayerVisibilities(vec![(LayerId(3), false)]),
    )
    .expect("the identical batch is accepted");
    assert_eq!(session.layer_history_len(), 1);

    // A restore with no overrides is not a step either.
    execute(
        &mut app,
        &mut session,
        CommandId::RestoreLayers,
        CommandPayload::None,
    )
    .expect("restore");
    assert_eq!(
        session.layer_history_len(),
        2,
        "a restore that really cleared one override is one step, and only one"
    );
    execute(
        &mut app,
        &mut session,
        CommandId::RestoreLayers,
        CommandPayload::None,
    )
    .expect("restore with nothing to restore");
    assert_eq!(
        session.layer_history_len(),
        2,
        "an empty restore pushed nothing"
    );

    // Two steps exist: the cleared state and the pre-hide state.
    layerp(&mut app, &mut session).expect("LAYERP");
    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    layerp(&mut app, &mut session).expect("LAYERP");
    assert!(session.layer_overrides.is_empty());
}

#[test]
fn an_empty_history_reports_the_named_diagnostic_and_changes_nothing() {
    let (mut app, mut session) = app_three_layers();
    let before_revision = revision(&app);

    let outcome = layerp(&mut app, &mut session).expect("a no-op, not an error");

    assert_eq!(digest_codes(&outcome), vec!["layers.previous.empty"]);
    assert_eq!(
        outcome.diagnostics[0].message,
        "LAYERP: there is no previous layer state to restore in this session; nothing changed"
    );
    assert!(outcome.measurement.is_none());
    assert!(outcome.changes.is_none());
    assert!(session.layer_overrides.is_empty());
    assert_eq!(session.layer_history_len(), 0);
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn the_history_is_bounded_and_drops_the_oldest_state() {
    let (mut app, mut session) = app_three_layers();
    assert_eq!(LAYER_OVERRIDE_HISTORY_CAP, 16);

    // 20 real changes (alternating WALLS visible/hidden), recording the state the
    // session was in *after* each change.
    let mut states: Vec<Vec<(LayerId, bool)>> = Vec::new();
    for index in 0..20 {
        let visible = index % 2 == 0;
        set_visible(&mut app, &mut session, LayerId(3), visible).expect("toggle WALLS");
        states.push(session.layer_overrides.iter().collect());
    }
    assert_eq!(session.layer_history_len(), LAYER_OVERRIDE_HISTORY_CAP);

    // The 16 retained snapshots are the pre-change states of changes 4..=19, so
    // walking all the way back lands on the state after change 3 — the oldest
    // state still reachable. Anything older was evicted, never silently restored.
    for _ in 0..LAYER_OVERRIDE_HISTORY_CAP {
        layerp(&mut app, &mut session).expect("a retained step");
    }
    assert_eq!(
        session.layer_overrides.iter().collect::<Vec<_>>(),
        states[3]
    );
    assert_eq!(session.layer_history_len(), 0);

    // Evicted states are gone for good.
    let outcome = layerp(&mut app, &mut session).expect("the history is exhausted");
    assert_eq!(digest_codes(&outcome), vec!["layers.previous.empty"]);
    assert_eq!(
        session.layer_overrides.iter().collect::<Vec<_>>(),
        states[3],
        "an exhausted LAYERP changes nothing"
    );
}

#[test]
fn layerp_restores_the_formerly_isolated_state() {
    let (mut app, mut session) = app_three_layers();

    // LAYISO with the WALLS entity selected: the current layer (0) and WALLS stay
    // visible, DOORS is hidden — the "formerly isolated" state.
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .expect("select the WALLS entity");
    execute(
        &mut app,
        &mut session,
        CommandId::IsolateLayers,
        CommandPayload::None,
    )
    .expect("LAYISO runs");
    assert_eq!(
        session.layer_overrides.get(LayerId(5)),
        Some(false),
        "DOORS is isolated away"
    );

    // LAYUNISO clears it (one recorded step)...
    execute(
        &mut app,
        &mut session,
        CommandId::RestoreLayers,
        CommandPayload::None,
    )
    .expect("the restore");
    assert!(session.layer_overrides.is_empty());

    // ...and LAYERP brings the isolation back: that is exactly "the previous layer
    // state", which is why LAYERP must not be an alias of the clear-all restore.
    layerp(&mut app, &mut session).expect("LAYERP runs");
    assert_eq!(
        session.layer_overrides.get(LayerId(5)),
        Some(false),
        "the formerly isolated state is restored"
    );
    assert_eq!(session.layer_overrides.get(LayerId(3)), None);
    assert_eq!(session.layer_overrides.get(LayerId(0)), None);
}

#[test]
fn layerp_is_a_session_view_operation_and_is_admitted_in_viewer() {
    assert!(
        !CommandId::LayerPrevious.requires_work_mode(),
        "LAYERP is session view state, so Viewer must admit it"
    );
    let (mut app, mut session) = app_three_layers();
    session.switch_mode(AppMode::Viewer).expect("Viewer mode");
    let before_revision = revision(&app);

    set_visible(&mut app, &mut session, LayerId(3), false).expect("a Viewer may hide a layer");
    layerp(&mut app, &mut session).expect("a Viewer may run LAYERP");

    assert!(session.layer_overrides.is_empty());
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn layerp_takes_no_payload() {
    // A payload is refused rather than silently ignored: LAYERP's only input is the
    // session's own layer-override history.
    let (mut app, mut session) = app_three_layers();
    set_visible(&mut app, &mut session, LayerId(3), false).expect("hide WALLS");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayerPrevious,
        CommandPayload::Layer(LayerId(3), true),
    )
    .expect_err("a payload is refused");
    assert!(
        error
            .to_string()
            .contains("layers.previous.invalid_payload"),
        "{error}"
    );
    // The refusal changed nothing, including the history.
    assert_eq!(session.layer_overrides.get(LayerId(3)), Some(false));
    assert_eq!(session.layer_history_len(), 1);
}

#[test]
fn the_history_type_is_a_bounded_ring_buffer() {
    // Unit-level contract for the container itself: the cap rule and the eviction
    // order are the type's, so they hold for every push site.
    let mut history = LayerOverrideHistory::new();
    assert!(history.is_empty());
    assert_eq!(history.cap(), LAYER_OVERRIDE_HISTORY_CAP);
    assert_eq!(history.pop(), None);

    for index in 0..LAYER_OVERRIDE_HISTORY_CAP + 4 {
        let mut snapshot = LayerOverrideSet::new();
        snapshot.set(LayerId(index as u128), false);
        history.push(snapshot);
    }
    assert_eq!(history.len(), LAYER_OVERRIDE_HISTORY_CAP);
    // Newest first on pop; the oldest four were dropped.
    let newest = history.pop().expect("a retained snapshot");
    assert_eq!(newest.get(LayerId(19)), Some(false));
    let mut remaining = 0;
    while let Some(snapshot) = history.pop() {
        assert!(
            snapshot.get(LayerId(0)).is_none(),
            "the oldest were evicted"
        );
        remaining += 1;
    }
    assert_eq!(remaining, LAYER_OVERRIDE_HISTORY_CAP - 1);
}
