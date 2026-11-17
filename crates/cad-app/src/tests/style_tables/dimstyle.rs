//! `DIMSTYLE` (and `-DIMSTYLE`) contract tests: the read-only reports, the
//! session-only `SET`, and the post-import `NEW`/`RENAME`/`DELETE`/`MODIFY`
//! table writes.
//!
//! Split out of `tests/style_tables.rs` for the 1000-line gate. Every write is
//! one revision / one undo step; every refusal is a stable named reason that
//! writes nothing; and a stored dimension keeps its resolved snapshot
//! (ADR 0017 non-retroactive).

use super::*;

use cad_import_acadrust::export::{
    export, AcisVersion, DxfTarget, ExportFormat, ExportLimits, ExportRequest, ExportSpace,
};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

// ---------------------------------------------------------------------------
// DIMSTYLE
// ---------------------------------------------------------------------------

#[test]
fn dimstyle_set_makes_the_row_current_and_survives_rename() {
    let (mut app, mut session) = style_app();
    let outcome = dimstyle_command(&mut app, &mut session, DimStyleCommand::Set("Fancy".into()))
        .expect("DIMSTYLE SET");
    assert!(outcome.diagnostics.iter().any(|d| d.code == "dimstyle.set"));
    assert_eq!(session.current_dim_style_name(), "Fancy");
    assert!(outcome.changes.is_none(), "SET is session-only");

    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Rename {
            old: "Fancy".into(),
            new: "Detail".into(),
        },
    )
    .expect("DIMSTYLE RENAME");
    assert_eq!(
        session.current_dim_style_name(),
        "Detail",
        "the session follows the rename"
    );
    assert!(drawing(&app).dim_style_by_name("Fancy").is_none());
    assert_eq!(
        drawing(&app).dim_style_by_name("detail").unwrap().id,
        DimStyleId(1)
    );
}

#[test]
fn dimstyle_new_modify_delete_round_trip_and_standard_is_undeletable() {
    let (mut app, mut session) = style_app();
    let start = revision(&app);

    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::New {
            name: "Detail".into(),
            from: Some("Fancy".into()),
        },
    )
    .expect("DIMSTYLE NEW");
    let id = drawing(&app).dim_style_by_name("Detail").unwrap().id;
    assert_eq!(revision(&app), Revision(start.0 + 1));

    let outcome = dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Modify {
            name: "Detail".into(),
            assignments: vec![
                ("DIMTXT".into(), "4.0".into()),
                ("DIMDEC".into(), "3".into()),
            ],
        },
    )
    .expect("DIMSTYLE MODIFY");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "dimstyle.modified"));
    let row = drawing(&app).dim_style(id).unwrap();
    assert_eq!(row.text_height, 4.0);
    assert_eq!(row.decimals, 3);

    let error = dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Modify {
            name: "Detail".into(),
            assignments: vec![("DIMTXT".into(), "0".into())],
        },
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimstyle.value_range"),
        "{error}"
    );

    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Delete("Detail".into()),
    )
    .expect("DIMSTYLE DELETE");
    assert!(drawing(&app).dim_style(id).is_none());

    let error = dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Delete("Standard".into()),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimstyle.standard_undeletable"),
        "{error}"
    );
}

/// Create a linear dimension through the real command path and return its
/// object id plus the stored entity as it was written.
fn create_dimension(app: &mut Application, session: &mut SessionState) -> (ObjectId, DbEntity) {
    let outcome = execute(
        app,
        session,
        CommandId::DimLinear,
        CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0), point(5.0, 12.0)]),
    )
    .expect("DIMLINEAR");
    let object = outcome.objects[0];
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .cloned()
        .expect("the created dimension entity");
    (object, entity)
}

/// The `(style name, DIMTXT)` snapshot a stored dimension carries.
fn dimension_snapshot(app: &Application, object: ObjectId) -> (String, f64) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the dimension entity");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (
            dimension.style_name().to_string(),
            dimension.style().text_height,
        ),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

fn stored_entity(app: &Application, object: ObjectId) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .cloned()
        .expect("the entity")
}

#[test]
fn dimstyle_set_only_changes_new_dimensions_and_edits_do_not_retroact() {
    let (mut app, mut session) = style_app();
    // Created under the default `Standard` (metric: DIMTXT 2.5).
    let (first, first_entity) = create_dimension(&mut app, &mut session);
    assert_eq!(
        dimension_snapshot(&app, first),
        ("Standard".to_string(), 2.5)
    );

    // A style edit is exactly one revision / one undo step, and the stored
    // dimension (its snapshot) is byte-identical afterwards.
    let before_edit = revision(&app);
    let depth_before = undo_depth(&app);
    let outcome = dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Modify {
            name: "Standard".into(),
            assignments: vec![("DIMTXT".into(), "9.0".into())],
        },
    )
    .expect("DIMSTYLE MODIFY Standard");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "dimstyle.modified"));
    assert_eq!(
        revision(&app),
        Revision(before_edit.0 + 1),
        "the style write is one revision"
    );
    assert_eq!(undo_depth(&app), depth_before + 1, "one undo step");
    assert_eq!(
        stored_entity(&app, first),
        first_entity,
        "the existing dimension is not rewritten"
    );
    assert_eq!(
        dimension_snapshot(&app, first),
        ("Standard".to_string(), 2.5),
        "the existing dimension keeps its resolved snapshot"
    );

    // A dimension created *after* the edit resolves the new value.
    let (second, _) = create_dimension(&mut app, &mut session);
    assert_eq!(
        dimension_snapshot(&app, second),
        ("Standard".to_string(), 9.0),
        "new dimensions resolve the edited row"
    );
    assert_eq!(
        dimension_snapshot(&app, first),
        ("Standard".to_string(), 2.5),
        "the first dimension is still unchanged"
    );

    // SET is session state only: no revision, no history, and only dimensions
    // created afterwards use it.
    let before_set = revision(&app);
    let depth_before = undo_depth(&app);
    let outcome = dimstyle_command(&mut app, &mut session, DimStyleCommand::Set("Fancy".into()))
        .expect("DIMSTYLE SET Fancy");
    assert!(outcome.changes.is_none(), "SET writes no drawing");
    assert_eq!(revision(&app), before_set, "SET bumps no revision");
    assert_eq!(undo_depth(&app), depth_before, "SET records no undo step");
    let message = &outcome.diagnostics[0].message;
    assert!(
        message.contains("not retroactive") && message.contains("existing dimensions"),
        "SET states the documented effect: {message}"
    );

    let (third, _) = create_dimension(&mut app, &mut session);
    assert_eq!(
        dimension_snapshot(&app, third),
        ("Fancy".to_string(), 2.5),
        "creation resolves the session's current row"
    );

    // `NEW` with no base copies the drawing's *stored* `Standard` (DIMTXT 9.0),
    // not a fixed constant template.
    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::New {
            name: "Copy".into(),
            from: None,
        },
    )
    .expect("DIMSTYLE NEW Copy");
    let copy = drawing(&app).dim_style_by_name("Copy").expect("Copy");
    assert_eq!(copy.text_height, 9.0);
    assert_eq!(copy.arrow_size, 2.5, "the rest of Standard is copied");
}

#[test]
fn dimstyle_refusals_are_named_and_write_nothing() {
    let (mut app, mut session) = style_app();
    // Make `Fancy` referenced by a stored dimension.
    dimstyle_command(&mut app, &mut session, DimStyleCommand::Set("Fancy".into()))
        .expect("DIMSTYLE SET Fancy");
    let (dimension, _) = create_dimension(&mut app, &mut session);
    let fancy_before = drawing(&app)
        .dim_style_by_name("Fancy")
        .cloned()
        .expect("Fancy");
    let snapshot_before = dimension_snapshot(&app, dimension);
    let start = revision(&app);
    let depth = undo_depth(&app);

    let cases = [
        (DimStyleCommand::Set("Missing".into()), "dimstyle.unknown"),
        (
            DimStyleCommand::Delete("Missing".into()),
            "dimstyle.unknown",
        ),
        (
            DimStyleCommand::Delete("Standard".into()),
            "dimstyle.standard_undeletable",
        ),
        (DimStyleCommand::Delete("Fancy".into()), "dimstyle.in_use"),
        (
            DimStyleCommand::Rename {
                old: "Fancy".into(),
                new: "Detail".into(),
            },
            "dimstyle.in_use",
        ),
        (
            DimStyleCommand::Modify {
                name: "Fancy".into(),
                assignments: vec![("DIMALT".into(), "2".into())],
            },
            "dimstyle.unknown_field",
        ),
        (
            DimStyleCommand::Modify {
                name: "Fancy".into(),
                assignments: vec![
                    ("DIMTXT".into(), "4.0".into()),
                    ("DIMBLK".into(), "arrow".into()),
                ],
            },
            "dimstyle.unknown_field",
        ),
        (
            DimStyleCommand::Modify {
                name: "Fancy".into(),
                assignments: vec![("DIMTXT".into(), "0".into())],
            },
            "dimstyle.value_range",
        ),
    ];
    for (command, code) in cases {
        let error =
            dimstyle_command(&mut app, &mut session, command.clone()).expect_err("named refusal");
        assert!(
            format!("{error}").contains(code),
            "DIMSTYLE {command:?} should refuse with {code}: {error}"
        );
        assert_eq!(revision(&app), start, "{code}: no revision");
        assert_eq!(undo_depth(&app), depth, "{code}: no undo step");
        assert_eq!(
            drawing(&app).dim_style_by_name("Fancy").cloned(),
            Some(fancy_before.clone()),
            "{code}: the row is unchanged (a partial MODIFY must not land)"
        );
        assert_eq!(
            dimension_snapshot(&app, dimension),
            snapshot_before,
            "{code}: the dimension snapshot is unchanged"
        );
    }

    // The unrepresentable-variable refusal names the field it could not carry.
    let error = dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Modify {
            name: "Fancy".into(),
            assignments: vec![("DIMALT".into(), "2".into())],
        },
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(
        text.contains("dimstyle.unknown_field") && text.contains("'dimalt'"),
        "the refusal names the missing field: {text}"
    );
}

#[test]
fn a_written_dimstyle_row_round_trips_through_save_and_open() {
    let (mut app, mut session) = style_app();
    // Write an unreferenced row through the commands (`NEW` + `MODIFY`), then
    // export and re-import it: the exporter half writes stored rows field for
    // field, so a SAVE -> OPEN must read the same values back.
    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::New {
            name: "Detail".into(),
            from: Some("Fancy".into()),
        },
    )
    .expect("DIMSTYLE NEW Detail");
    dimstyle_command(
        &mut app,
        &mut session,
        DimStyleCommand::Modify {
            name: "Detail".into(),
            assignments: vec![
                ("DIMTXT".into(), "7.5".into()),
                ("DIMDEC".into(), "3".into()),
                ("DIMTAD".into(), "2".into()),
            ],
        },
    )
    .expect("DIMSTYLE MODIFY Detail");
    let stored = drawing(&app).dim_style_by_name("Detail").cloned().unwrap();

    let source = app.workspace.documents[&DocumentId(1)].drawing.clone();
    let (bytes, _) = export(&ExportRequest {
        database: &source,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        acis_version: AcisVersion::default(),
        dxf_target: DxfTarget::default(),
        limits: ExportLimits::default(),
        units: Some(UnitContext::drawing_units()),
    })
    .expect("export succeeds");

    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("re-import succeeds");
    let reopened = imported
        .database
        .dim_style_by_name("Detail")
        .expect("the written row survives SAVE -> OPEN");
    assert_eq!(reopened.text_height, 7.5);
    assert_eq!(reopened.decimals, 3);
    assert_eq!(reopened.text_vertical, 2);
    assert_eq!(reopened.arrow_size, stored.arrow_size);
    assert_eq!(reopened.text_style_name, stored.text_style_name);
}
