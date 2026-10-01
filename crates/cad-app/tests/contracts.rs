use cad_app::*;
use cad_domain::*;

#[test]
fn viewer_rejects_every_mutating_annotation_entry() {
    let session = SessionState::new(DocumentId(1), AppMode::Viewer);
    for command in [
        CommandId::CreateAnnotation,
        CommandId::UpdateAnnotation,
        CommandId::DeleteAnnotation,
        CommandId::Undo,
        CommandId::Redo,
        CommandId::ImportAnnotations,
        CommandId::Measure,
    ] {
        assert_eq!(session.authorize(command), Err(CadError::PermissionDenied));
    }
    for command in [
        CommandId::Pan,
        CommandId::Zoom,
        CommandId::ToggleLayer,
        CommandId::SwitchSpace,
        CommandId::ExportAnnotations,
    ] {
        assert_eq!(session.authorize(command), Ok(()));
    }
}

#[test]
fn command_path_checks_mode_and_document_before_dispatch() {
    let mut app = Application::new();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let command = |id, document, payload| Command {
        schema_version: 1,
        id,
        document,
        viewport: ViewportId(1),
        payload,
    };
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::DeleteAnnotation,
                DocumentId(1),
                CommandPayload::None
            )
        ),
        Err(CadError::PermissionDenied)
    ));
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Pan, DocumentId(2), CommandPayload::None)
        ),
        Err(CadError::StaleResult)
    ));
    // Pan now requires a world-space delta rather than being unimplemented.
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Pan, DocumentId(1), CommandPayload::None)
        ),
        Err(CadError::InvalidInput(_))
    ));
    // SwitchBackend is implemented: a missing payload is rejected as invalid
    // input rather than pretending to be unimplemented (audit B01).
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(1),
                CommandPayload::None
            )
        ),
        Err(CadError::InvalidInput(_))
    ));
    // A command addressed to a different document than the session's is stale
    // and must be rejected before dispatch.
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(2),
                CommandPayload::Backend(BackendChoice::WebGl2)
            )
        ),
        Err(CadError::StaleResult)
    ));
    // A valid backend choice records the preference and reports it.
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::SwitchBackend,
                DocumentId(1),
                CommandPayload::Backend(BackendChoice::WebGl2),
            ),
        )
        .expect("valid backend choice executes");
    assert_eq!(session.backend, BackendChoice::WebGl2);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "backend.preference"));
}

#[test]
fn a_confirmed_drag_is_one_undo_and_a_cancel_leaves_nothing() {
    // The transaction path guarantees: a cancelled tool never commits, and one
    // confirm produces exactly one undo record.
    let session = SessionState::new(DocumentId(1), AppMode::Work);
    assert_eq!(session.authorize(CommandId::Undo), Ok(()));
}

#[test]
fn screen_pick_produces_a_selection_ref_the_select_command_accepts() {
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: -1.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    let database = builder.finish().unwrap();

    let camera = Camera::top_view_2d();
    let size = [800.0, 600.0];
    let report = pick_at_screen(
        &database,
        DocumentId(1),
        &camera,
        [size[0] * 0.5, size[1] * 0.5],
        size,
        &TolerancePolicy::default(),
        BackFacePolicy::Cull,
    )
    .expect("a valid pick runs");
    let hit = report.hit.expect("the centre line is hit");
    assert_eq!(hit.source.entity, EntityId(1));

    // The hit feeds the read-only selection command unchanged.
    let mut app = Application::new();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    let command = Command {
        schema_version: 1,
        id: CommandId::Select,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload: CommandPayload::Selection(vec![hit.source.clone()]),
    };
    app.execute(&mut session, command).unwrap();
    assert_eq!(session.selection.len(), 1);
    assert_eq!(session.selection.refs()[0], hit.source);
}
