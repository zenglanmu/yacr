//! Synthetic command contracts only; no UI, renderer or vendor validation.
use std::sync::Arc;

use cad_app::{
    host::HostController, AppMode, Application, Command, CommandId, CommandPayload, Document,
    SessionState,
};
use cad_db::{AnnotationDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    CadError, DatabaseId, DocumentId, DocumentIdentity, LayerId, UnitContext, ViewportId,
};

const LOW: LayerId = LayerId(7);
const HIGH: LayerId = LayerId((1_u128 << 96) + 7);
const MAX: LayerId = LayerId(u128::MAX);

fn fixture(mode: AppMode) -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name, visible) in [
        (LayerId(0), "0", true),
        (LOW, "LOW", true),
        (HIGH, "HIGH", false),
        (MAX, "MAX", true),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible,
            })
            .unwrap();
    }
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(builder.finish().unwrap()),
            annotations: AnnotationDatabase::new(DatabaseId(2)),
            identity: DocumentIdentity::Temporary(1),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    let mut session = SessionState::new(DocumentId(1), mode);
    session.layer_overrides.set(LOW, true);
    session.layer_overrides.set(MAX, false);
    (app, session)
}

fn command(payload: CommandPayload) -> Command {
    Command {
        schema_version: 1,
        id: CommandId::SetLayerVisibilities,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload,
    }
}

fn assert_invalid_unchanged(payload: CommandPayload) {
    let (mut app, mut session) = fixture(AppMode::Work);
    let before = session.layer_overrides.clone();
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let revision = drawing.revision();
    assert!(matches!(
        app.execute(&mut session, command(payload)),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(session.layer_overrides, before);
    assert!(Arc::ptr_eq(
        &drawing,
        &app.workspace.documents[&DocumentId(1)].drawing
    ));
    assert_eq!(drawing.revision(), revision);
    assert!(app.history.is_empty());
}

#[test]
fn mixed_batch_preserves_full_ids_and_unrelated_overrides_without_database_edits() {
    let (mut app, mut session) = fixture(AppMode::Work);
    let drawing = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let revision = drawing.revision();
    let flags: Vec<_> = drawing
        .layers()
        .map(|layer| (layer.id, layer.visible))
        .collect();
    let generation = session.generation;
    let outcome = app
        .execute(
            &mut session,
            command(CommandPayload::LayerVisibilities(vec![
                (HIGH, true),
                (LOW, false),
            ])),
        )
        .unwrap();
    assert_eq!(session.layer_overrides.get(LOW), Some(false));
    assert_eq!(session.layer_overrides.get(HIGH), Some(true));
    assert_eq!(session.layer_overrides.get(MAX), Some(false));
    assert_eq!(session.layer_overrides.get(LayerId(0)), None);
    assert_eq!(session.layer_overrides.len(), 3);
    assert_eq!(session.generation, generation);
    assert!(outcome.changes.is_none());
    assert!(outcome.objects.is_empty());
    assert!(Arc::ptr_eq(
        &drawing,
        &app.workspace.documents[&DocumentId(1)].drawing
    ));
    assert_eq!(drawing.revision(), revision);
    assert_eq!(
        drawing
            .layers()
            .map(|layer| (layer.id, layer.visible))
            .collect::<Vec<_>>(),
        flags
    );
    assert!(app.history.is_empty());
}

#[test]
fn full_batch_all_on_and_all_off_are_allowed_in_viewer_and_work() {
    assert!(!CommandId::SetLayerVisibilities.requires_work_mode());
    for mode in [AppMode::Viewer, AppMode::Work] {
        let (mut app, mut session) = fixture(mode);
        for visible in [true, false] {
            let changes = [LayerId(0), LOW, HIGH, MAX]
                .into_iter()
                .map(|id| (id, visible))
                .collect();
            app.execute(
                &mut session,
                command(CommandPayload::LayerVisibilities(changes)),
            )
            .unwrap();
            assert_eq!(session.layer_overrides.len(), 4);
            for id in [LayerId(0), LOW, HIGH, MAX] {
                assert_eq!(session.layer_overrides.get(id), Some(visible));
            }
        }
        assert!(app.history.is_empty());
    }
}

#[test]
fn unknown_layer_after_valid_changes_rejects_entire_batch() {
    assert_invalid_unchanged(CommandPayload::LayerVisibilities(vec![
        (LOW, false),
        (HIGH, true),
        (LayerId(123), false),
    ]));
}

#[test]
fn duplicate_with_equal_values_rejects_entire_batch() {
    assert_invalid_unchanged(CommandPayload::LayerVisibilities(vec![
        (LOW, false),
        (HIGH, true),
        (LOW, false),
    ]));
}

#[test]
fn duplicate_with_conflicting_values_rejects_entire_batch() {
    assert_invalid_unchanged(CommandPayload::LayerVisibilities(vec![
        (LOW, false),
        (HIGH, true),
        (LOW, true),
    ]));
}

#[test]
fn empty_batch_is_invalid_not_empty_success() {
    assert_invalid_unchanged(CommandPayload::LayerVisibilities(Vec::new()));
}

#[test]
fn wrong_payload_rejects_without_publication() {
    assert_invalid_unchanged(CommandPayload::None);
    assert_invalid_unchanged(CommandPayload::Layer(LOW, false));
}

#[test]
fn stale_document_rejects_before_publication_even_when_target_is_open() {
    let (mut app, mut session) = fixture(AppMode::Viewer);
    let other = app.workspace.documents.remove(&DocumentId(1)).unwrap();
    app.workspace.documents.insert(
        DocumentId(2),
        Document {
            id: DocumentId(2),
            ..other
        },
    );
    let before = session.layer_overrides.clone();
    let mut stale = command(CommandPayload::LayerVisibilities(vec![(LOW, false)]));
    stale.document = DocumentId(2);
    assert!(matches!(
        app.execute(&mut session, stale),
        Err(CadError::StaleResult)
    ));
    assert_eq!(session.layer_overrides, before);
}

#[test]
fn unopened_current_document_rejects_entire_batch() {
    let (mut app, mut session) = fixture(AppMode::Viewer);
    app.workspace.documents.remove(&DocumentId(1));
    let before = session.layer_overrides.clone();
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandPayload::LayerVisibilities(vec![(LOW, false)]))
        ),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(session.layer_overrides, before);
}

#[test]
fn single_toggle_rejects_unknown_layer_and_unopened_document() {
    let (mut app, mut session) = fixture(AppMode::Viewer);
    let before = session.layer_overrides.clone();
    let mut toggle = command(CommandPayload::Layer(LayerId(123), false));
    toggle.id = CommandId::ToggleLayer;
    assert!(matches!(
        app.execute(&mut session, toggle),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(session.layer_overrides, before);
    app.workspace.documents.remove(&DocumentId(1));
    let mut toggle = command(CommandPayload::Layer(LOW, false));
    toggle.id = CommandId::ToggleLayer;
    assert!(matches!(
        app.execute(&mut session, toggle),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(session.layer_overrides, before);
}

#[test]
fn host_batch_routes_through_atomic_validation() {
    let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
    host.session.switch_mode(AppMode::Viewer).unwrap();
    let ids = host.layer_ids().unwrap();
    assert!(!ids.is_empty());
    host.set_layer_visibilities(ids.iter().map(|id| (*id, false)).collect())
        .unwrap();
    let before = host.session.layer_overrides.clone();
    assert!(matches!(
        host.set_layer_visibilities(vec![(ids[0], true), (ids[0], true)]),
        Err(CadError::InvalidInput(_))
    ));
    assert_eq!(host.session.layer_overrides, before);
    host.set_layer_visibilities(ids.iter().map(|id| (*id, true)).collect())
        .unwrap();
    assert!(host
        .layer_rows()
        .unwrap()
        .iter()
        .all(|row| row.effective_visible));
    assert!(host.application.history.is_empty());
}
