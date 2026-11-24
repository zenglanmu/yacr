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
use crate::command_payload::DashLayerOps;
use crate::layers::LayerOverrideSet;
use crate::{LayerOverrideHistory, LayerPreviousState, LAYER_OVERRIDE_HISTORY_CAP};
use cad_db::{DrawingDatabaseBuilder, Layer, LayerAttributes, LineType, LinetypePattern};

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
        (LayerId(6), "DIMS"),
    ] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_linetype(LineType {
        id: LinetypeId(1),
        name: "DASHED".into(),
        pattern: LinetypePattern::from_elements([0.5, -0.25]),
        complex: false,
    })
    .unwrap();
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
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
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

fn select(app: &mut Application, session: &mut SessionState, entity: u128) {
    execute(
        app,
        session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(entity)]),
    )
    .expect("select the entity");
}

fn layerp(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::LayerPrevious, CommandPayload::None)
}

fn dash(
    app: &mut Application,
    session: &mut SessionState,
    op: DashLayerOps,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetLayerProperty,
        CommandPayload::DashLayer(op),
    )
}

fn set_property(
    app: &mut Application,
    session: &mut SessionState,
    layer: &str,
    key: &str,
    value: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::SetLayerProperty,
        CommandPayload::SetLayerProperty {
            layer: layer.into(),
            key: key.into(),
            value: value.into(),
        },
    )
}

fn undo(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::Undo, CommandPayload::None)
}

fn redo(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    execute(app, session, CommandId::Redo, CommandPayload::None)
}

fn attributes(app: &Application, layer: LayerId) -> LayerAttributes {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layer_attributes(layer)
}

fn digest_codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn refusal(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    }
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
    for index in 0..20usize {
        let visible = index.is_multiple_of(2);
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
fn layerp_reports_a_consumed_identical_snapshot_as_a_named_noop() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, 1);
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("LAYFRZ");
    // Undo the table write without touching the LAYERP stack: the recorded
    // pre-change state now equals the current state.
    undo(&mut app, &mut session).expect("undo LAYFRZ");
    assert!(!attributes(&app, LayerId(3)).frozen);

    let before = (revision(&app), undo_depth(&app));
    let outcome = layerp(&mut app, &mut session).expect("LAYERP consumes the step");
    assert_eq!(
        digest_codes(&outcome),
        vec!["layers.previous.unchanged"],
        "nothing to change is a named no-op, not a fake restore"
    );
    assert_eq!(revision(&app), before.0);
    assert_eq!(undo_depth(&app), before.1);
    assert_eq!(session.layer_history_len(), 0, "the step is consumed");
}

#[test]
fn the_layer_previous_history_type_is_a_bounded_ring_buffer() {
    // Unit-level contract for the container itself: the cap rule and the eviction
    // order are the type's, so they hold for every push site.
    let mut history = LayerOverrideHistory::new();
    assert!(history.is_empty());
    assert_eq!(history.cap(), LAYER_OVERRIDE_HISTORY_CAP);
    assert_eq!(history.pop(), None);

    for index in 0..LAYER_OVERRIDE_HISTORY_CAP + 4 {
        let mut snapshot = LayerOverrideSet::new();
        snapshot.set(LayerId(index as u128), false);
        history.push(LayerPreviousState::new(snapshot));
    }
    assert_eq!(history.len(), LAYER_OVERRIDE_HISTORY_CAP);
    // Newest first on pop; the oldest four were dropped.
    let newest = history.pop().expect("a retained snapshot");
    assert_eq!(newest.overrides.get(LayerId(19)), Some(false));
    let mut remaining = 0;
    while let Some(snapshot) = history.pop() {
        assert!(
            snapshot.overrides.get(LayerId(0)).is_none(),
            "the oldest were evicted"
        );
        remaining += 1;
    }
    assert_eq!(remaining, LAYER_OVERRIDE_HISTORY_CAP - 1);
}

#[test]
fn layerp_restores_a_frozen_layer_in_one_undo_step() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, 1);

    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("LAYFRZ freezes WALLS");
    assert!(attributes(&app, LayerId(3)).frozen, "LAYFRZ writes frozen");
    let frozenrevision = revision(&app);
    assert_eq!(undo_depth(&app), 1, "LAYFRZ is one undo step");
    assert_eq!(session.layer_history_len(), 1, "LAYFRZ is one LAYERP step");

    layerp(&mut app, &mut session).expect("LAYERP thaws it again");
    assert!(!attributes(&app, LayerId(3)).frozen);
    assert_eq!(
        undo_depth(&app),
        2,
        "the attribute restore is one undo step, not one per row"
    );
    assert!(
        revision(&app) > frozenrevision,
        "the restore bumped the revision"
    );

    undo(&mut app, &mut session).expect("undo the LAYERP");
    assert!(
        attributes(&app, LayerId(3)).frozen,
        "undo restores the pre-LAYERP frozen state"
    );
    redo(&mut app, &mut session).expect("redo the LAYERP");
    assert!(!attributes(&app, LayerId(3)).frozen);
}

#[test]
fn layerp_restores_a_locked_layer() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, 1);

    execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("LAYLCK locks WALLS");
    assert!(attributes(&app, LayerId(3)).locked);

    layerp(&mut app, &mut session).expect("LAYERP unlocks it again");
    assert!(!attributes(&app, LayerId(3)).locked);
    assert_eq!(session.layer_history_len(), 0);
}

#[test]
fn layerp_round_trips_every_recorded_layer_property() {
    let (mut app, mut session) = app_three_layers();
    for (key, value) in [
        ("color", "aci:5"),
        ("lineweight", "50"),
        ("transparency", "40"),
        ("plottable", "false"),
        ("locked", "true"),
        ("off", "true"),
        ("frozen", "true"),
        ("linetype", "DASHED"),
    ] {
        let before = attributes(&app, LayerId(3));
        set_property(&mut app, &mut session, "WALLS", key, value)
            .unwrap_or_else(|error| panic!("{key} must write: {error:?}"));
        assert_ne!(
            attributes(&app, LayerId(3)),
            before,
            "the {key} write must really change the row"
        );
        assert_eq!(
            session.layer_history_len(),
            1,
            "one write is one LAYERP step"
        );
        layerp(&mut app, &mut session).expect("LAYERP");
        assert_eq!(
            attributes(&app, LayerId(3)),
            before,
            "LAYERP restores the pre-{key} attribute set"
        );
        assert_eq!(session.layer_history_len(), 0);
    }
}

#[test]
fn layerp_round_trips_the_typed_dash_layer_attribute_forms() {
    let (mut app, mut session) = app_three_layers();
    let names = || vec!["WALLS".to_string()];
    for op in [
        DashLayerOps::Off(names()),
        DashLayerOps::Freeze(names()),
        DashLayerOps::Lock(names()),
        DashLayerOps::Color {
            value: "aci:4".into(),
            layers: names(),
        },
        DashLayerOps::Linetype {
            name: "DASHED".into(),
            layers: names(),
        },
        DashLayerOps::Lineweight {
            value: "25".into(),
            layers: names(),
        },
        DashLayerOps::Plot {
            plottable: false,
            layers: names(),
        },
    ] {
        let before = attributes(&app, LayerId(3));
        dash(&mut app, &mut session, op).expect("-LAYER writes");
        assert_ne!(
            attributes(&app, LayerId(3)),
            before,
            "the -LAYER form must really change the row"
        );
        assert_eq!(session.layer_history_len(), 1);
        layerp(&mut app, &mut session).expect("LAYERP");
        assert_eq!(attributes(&app, LayerId(3)), before);
    }
}

#[test]
fn layerp_restores_the_pre_laythw_frozen_rows() {
    let (mut app, mut session) = app_three_layers();
    select(&mut app, &mut session, 1);
    execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("LAYFRZ");
    assert!(attributes(&app, LayerId(3)).frozen);

    execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("LAYTHW");
    assert!(!attributes(&app, LayerId(3)).frozen);
    assert_eq!(session.layer_history_len(), 2, "LAYTHW is one LAYERP step");

    layerp(&mut app, &mut session).expect("LAYERP re-freezes the row");
    assert!(attributes(&app, LayerId(3)).frozen);
}

#[test]
fn layerp_restores_the_pre_layerstate_restore_attributes() {
    let (mut app, mut session) = app_three_layers();
    execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(crate::command_payload::LayerStateCommand::Save("S1".into())),
    )
    .expect("LAYERSTATE SAVE");
    set_property(&mut app, &mut session, "WALLS", "locked", "true").expect("lock WALLS");
    assert!(attributes(&app, LayerId(3)).locked);

    execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(crate::command_payload::LayerStateCommand::Restore(
            "S1".into(),
        )),
    )
    .expect("LAYERSTATE RESTORE");
    assert!(!attributes(&app, LayerId(3)).locked);
    assert_eq!(
        session.layer_history_len(),
        2,
        "the restore records one LAYERP step on top of the lock write"
    );

    layerp(&mut app, &mut session).expect("LAYERP");
    assert!(
        attributes(&app, LayerId(3)).locked,
        "LAYERP restores the pre-restore locked row"
    );
}

#[test]
fn layerp_refuses_a_snapshot_whose_layer_row_is_gone_without_writing() {
    let (mut app, mut session) = app_three_layers();
    // DIMS has no entity, so it can be deleted after the snapshot.
    set_property(&mut app, &mut session, "DIMS", "locked", "true").expect("lock DIMS");
    execute(
        &mut app,
        &mut session,
        CommandId::DeleteLayer,
        CommandPayload::DeleteLayer {
            layer: "DIMS".into(),
        },
    )
    .expect("delete the empty layer");

    let before = (revision(&app), undo_depth(&app));
    let error = layerp(&mut app, &mut session).expect_err("dangling layer");
    assert!(refusal(error).starts_with("layers.previous.dangling_layer:"));
    assert_eq!(revision(&app), before.0, "the refusal writes nothing");
    assert_eq!(
        undo_depth(&app),
        before.1,
        "the refusal records no undo step"
    );
    assert_eq!(
        session.layer_history_len(),
        1,
        "the refused snapshot stays on the stack"
    );
}
