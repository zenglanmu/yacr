//! Wave 27 read-only report contracts: `RECOVER`, `RECOVERALL`, `PURGEAECDATA`
//! and `SHOWURLS`.
//!
//! Pinned here: each selector is admitted from Viewer mode, returns `Ok` with
//! diagnostics (never an error for a merely unsupported capability), and writes
//! nothing — no `ChangeSet`, no revision bump, no undo entry, no entity change.
//! The documented diagnostic codes are asserted verbatim, `RECOVER` on a clean
//! drawing reports the clean summary with zero invalid entities, and a non-`None`
//! payload is refused before any scan runs.

use super::*;

/// The four Wave 27 selectors with their canonical names.
const REPORT_IDS: [(CommandId, &str); 4] = [
    (CommandId::RecoverDrawing, "RECOVER"),
    (CommandId::RecoverAllDrawings, "RECOVERALL"),
    (CommandId::PurgeAecData, "PURGEAECDATA"),
    (CommandId::ShowUrls, "SHOWURLS"),
];

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

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn assert_report_wrote_nothing(app: &Application, outcome: &CommandOutcome, before: Revision) {
    assert!(outcome.changes.is_none(), "{:?}", outcome.diagnostics);
    assert!(outcome.objects.is_empty());
    assert!(outcome.measurement.is_none());
    assert!(outcome.point_report.is_none());
    assert_eq!(revision(app), before, "the revision must not move");
    assert_eq!(undo_depth(app), 0, "no undo entry may be recorded");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn every_report_selector_is_viewer_legal_and_writes_nothing() {
    for (id, selector) in REPORT_IDS {
        assert!(
            !id.requires_work_mode(),
            "{selector} is a read-only report, so Viewer mode must admit it"
        );
        let (mut app, _) = app_with(vec![
            line(1, point(0.0, 0.0), point(10.0, 0.0)),
            line(2, point(3.0, 3.0), point(3.0, 3.0)),
        ]);
        let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
        let before = revision(&app);
        let count_before = entity_count(&app);

        let outcome = app
            .execute(&mut session, command(id, CommandPayload::None))
            .unwrap_or_else(|error| panic!("{selector} must report, not fail: {error:?}"));

        assert!(
            !outcome.diagnostics.is_empty(),
            "{selector} must always report at least one diagnostic"
        );
        assert_eq!(entity_count(&app), count_before);
        assert_report_wrote_nothing(&app, &outcome, before);
    }
}

#[test]
fn recover_on_a_clean_drawing_reports_zero_invalid_and_the_clean_summary() {
    let (mut app, mut session) = app_with(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0)),
        line(2, point(0.0, 5.0), point(10.0, 5.0)),
    ]);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::RecoverDrawing, CommandPayload::None),
        )
        .expect("RECOVER reports on a clean drawing");

    assert_eq!(
        codes(&outcome),
        vec!["recover.summary", "recover.file_level_not_modeled"],
        "{:?}",
        outcome.diagnostics
    );
    let summary = &outcome.diagnostics[0];
    assert_eq!(summary.code, "recover.summary");
    assert_eq!(summary.object, None);
    assert_eq!(
        summary.message,
        "RECOVER: 2 entities checked; no invalid entity found; no repair performed"
    );
    // The disk-level salvage half is named, never silently claimed.
    let boundary = &outcome.diagnostics[1];
    assert_eq!(boundary.code, "recover.file_level_not_modeled");
    assert_ascii(&boundary.message);
    assert_report_wrote_nothing(&app, &outcome, before);
}

#[test]
fn recover_reports_each_invalid_entity_and_repairs_nothing() {
    // The import builder is permissive: a zero-length LINE is stored, which is
    // exactly what the scan exists to report.
    let (mut app, mut session) = app_with(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0)),
        line(2, point(3.0, 3.0), point(3.0, 3.0)),
    ]);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::RecoverDrawing, CommandPayload::None),
        )
        .expect("RECOVER reports an invalid entity");

    assert_eq!(
        codes(&outcome),
        vec![
            "recover.summary",
            "recover.invalid_entity",
            "recover.file_level_not_modeled",
        ],
        "{:?}",
        outcome.diagnostics
    );
    let summary = &outcome.diagnostics[0];
    assert_eq!(summary.code, "recover.summary");
    assert_eq!(
        summary.message,
        "RECOVER: 2 entities checked; 1 invalid; no repair performed"
    );
    let invalid = &outcome.diagnostics[1];
    assert_eq!(invalid.code, "recover.invalid_entity");
    assert_eq!(invalid.object, Some(ObjectId(2)));
    assert!(
        invalid.message.starts_with("entity 2: "),
        "{:?}",
        invalid.message
    );
    assert_ascii(&invalid.message);

    // Detect only: the stored row is untouched, and the report wrote nothing.
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(2))
        .is_some());
    assert_report_wrote_nothing(&app, &outcome, before);
}

#[test]
fn recoverall_scans_the_single_open_document_and_names_the_batch_boundary() {
    let (mut app, mut session) = app_with(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0)),
        line(2, point(4.0, 4.0), point(4.0, 4.0)),
    ]);
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::RecoverAllDrawings, CommandPayload::None),
        )
        .expect("RECOVERALL reports");

    assert_eq!(
        codes(&outcome),
        vec![
            "recover.summary",
            "recover.invalid_entity",
            "recover.file_level_not_modeled",
            "recoverall.single_document",
        ],
        "{:?}",
        outcome.diagnostics
    );
    let summary = &outcome.diagnostics[0];
    assert_eq!(summary.code, "recover.summary");
    assert_eq!(
        summary.message,
        "RECOVERALL: 2 entities checked; 1 invalid; no repair performed"
    );
    let boundary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "recoverall.single_document")
        .expect("the single-document boundary is named");
    assert_ascii(&boundary.message);
    assert_report_wrote_nothing(&app, &outcome, before);
}

#[test]
fn the_data_less_reports_name_their_boundary_instead_of_an_empty_success() {
    for (id, selector, code) in [
        (
            CommandId::PurgeAecData,
            "PURGEAECDATA",
            "purgeaecdata.no_aec_model",
        ),
        (CommandId::ShowUrls, "SHOWURLS", "showurls.not_retained"),
    ] {
        let (mut app, mut session) = app_with(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
        let before = revision(&app);
        let outcome = app
            .execute(&mut session, command(id, CommandPayload::None))
            .unwrap_or_else(|error| panic!("{selector} must report, not fail: {error:?}"));

        assert_eq!(codes(&outcome), vec![code], "{:?}", outcome.diagnostics);
        assert_eq!(outcome.diagnostics[0].object, None);
        assert_ascii(&outcome.diagnostics[0].message);
        assert_report_wrote_nothing(&app, &outcome, before);
    }
}

#[test]
fn a_non_none_payload_is_refused_before_any_scan() {
    for (id, selector) in REPORT_IDS {
        let (mut app, mut session) = app_with(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
        let before = revision(&app);
        let error = app
            .execute(
                &mut session,
                command(id, CommandPayload::Selection(Vec::new())),
            )
            .expect_err("a report command takes no payload");
        match error {
            CadError::InvalidInput(message) => {
                assert_eq!(message, format!("{selector} needs no payload"));
                assert!(
                    message.contains(selector),
                    "the refusal must name {selector}: {message:?}"
                );
            }
            other => panic!("expected InvalidInput for {selector}, got {other:?}"),
        }
        assert_eq!(revision(&app), before, "the refused payload wrote nothing");
        assert_eq!(undo_depth(&app), 0);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

fn assert_ascii(message: &str) {
    assert!(
        message.is_ascii(),
        "the Wave 27 reports must be ASCII only: {message:?}"
    );
}
