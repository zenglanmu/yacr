//! `AUDIT` contract tests: detect-only drawing validation.
//!
//! Pinned here: a valid drawing reports `audit.clean` and changes nothing; a
//! stored invalid entity is reported as `audit.invalid_entity` behind an
//! `audit.summary` (the import builder is deliberately permissive, so this state
//! is constructible through the ordinary builder without bypassing a validated
//! write); the command refuses a payload and a stale document; and every path is
//! read-only — no revision bump, no `ChangeSet`, no undo entry, no repair.
//!
//! There is deliberately no "repaired" assertion: AUDIT does **not** repair, so
//! the invalid entity must still be there (same revision, same geometry) after
//! the report.

use super::*;

fn entity(id: u128, geometry: SemanticGeometry) -> cad_db::DbEntity {
    cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(id),
            type_key: "AcDbTest".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: id as i64,
    }
}

fn line(id: u128, start: Point3, end: Point3) -> cad_db::DbEntity {
    entity(id, SemanticGeometry::Line { start, end })
}

fn circle(id: u128, center: Point3, radius: f64) -> cad_db::DbEntity {
    entity(
        id,
        SemanticGeometry::Circle {
            center,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
    )
}

fn app_with(entities: Vec<cad_db::DbEntity>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for entity in entities {
        b.insert_entity(entity).unwrap();
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

fn audit(app: &mut Application, session: &mut SessionState) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::AuditDrawing, CommandPayload::None),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn assert_ascii(message: &str) {
    assert!(
        message.is_ascii(),
        "AUDIT must report ASCII only: {message:?}"
    );
}

#[test]
fn a_valid_drawing_reports_clean_and_changes_nothing() {
    let (mut app, mut session) = app_with(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0)),
        circle(2, point(5.0, 5.0), 2.0),
    ]);
    let before_revision = revision(&app);
    let before_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    let outcome = audit(&mut app, &mut session).expect("AUDIT runs on a valid drawing");

    assert_eq!(outcome.diagnostics.len(), 1, "{:?}", outcome.diagnostics);
    let clean = &outcome.diagnostics[0];
    assert_eq!(clean.code, "audit.clean");
    assert_eq!(clean.object, None);
    assert_eq!(
        clean.message,
        "AUDIT: 2 entities checked; no invalid entity found; no repair performed"
    );
    assert_ascii(&clean.message);
    // Nothing is emitted as an object/measurement, and no change set is published.
    assert!(outcome.objects.is_empty());
    assert!(outcome.changes.is_none());
    assert!(outcome.measurement.is_none());
    assert!(outcome.point_report.is_none());
    // Read-only: no revision bump, same entities, no undo entry.
    assert_eq!(revision(&app), before_revision);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before_count
    );
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        0
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn an_invalid_stored_entity_is_reported_and_not_repaired() {
    // The import builder is deliberately permissive, so a zero-length LINE can
    // be stored; `finish()` validates references, not geometry. That is exactly
    // the state AUDIT exists to report.
    let (mut app, mut session) = app_with(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0)),
        line(2, point(3.0, 3.0), point(3.0, 3.0)),
    ]);
    let before_revision = revision(&app);

    let outcome = audit(&mut app, &mut session).expect("AUDIT reports an invalid entity");

    assert_eq!(outcome.diagnostics.len(), 2, "{:?}", outcome.diagnostics);
    let summary = &outcome.diagnostics[0];
    assert_eq!(summary.code, "audit.summary");
    assert_eq!(summary.object, None);
    assert_eq!(
        summary.message,
        "AUDIT: 2 entities checked; 1 invalid; no repair performed"
    );
    assert_ascii(&summary.message);
    let invalid = &outcome.diagnostics[1];
    assert_eq!(invalid.code, "audit.invalid_entity");
    assert_eq!(invalid.object, Some(ObjectId(2)));
    assert!(
        invalid.message.starts_with("entity 2: "),
        "{:?}",
        invalid.message
    );
    assert_ascii(&invalid.message);
    // Detect only: the invalid entity is still stored exactly as it was.
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_payload_is_refused_explicitly() {
    let (mut app, mut session) = app_with(Vec::new());
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::AuditDrawing,
                CommandPayload::Selection(Vec::new()),
            ),
        )
        .expect_err("AUDIT takes no payload");
    match error {
        CadError::InvalidInput(message) => {
            assert_eq!(message, "AUDIT needs no payload");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn a_stale_document_is_refused() {
    let (mut app, mut session) = app_with(vec![line(1, point(0.0, 0.0), point(1.0, 0.0))]);
    // A document whose stored identity no longer matches the key it is filed
    // under is stale: the command must not audit the wrong drawing.
    app.workspace.documents.get_mut(&DocumentId(1)).unwrap().id = DocumentId(9);
    let error = audit(&mut app, &mut session).expect_err("a stale document is refused");
    assert!(
        matches!(error, CadError::StaleResult),
        "expected StaleResult, got {error:?}"
    );
}
