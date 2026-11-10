//! `TABLE` / `-TABLE` creation and `TABLEDIT` cell-text editing contracts.
//!
//! Creation builds one real empty table through the shared create funnel (active
//! layer, model space, one transaction / one undo step) and exports/re-imports as
//! an `ACAD_TABLE`. `TABLEDIT` is the pre-selection 1-based cell-text edit: one
//! transaction / one undo step, an identical string is a no-op, and every named
//! refusal writes nothing.

use super::*;
use cad_history::History;

use crate::command_payload::{TableCreateCommand, TableEditCommand};
use cad_import_acadrust::export::{export, ExportFormat, ExportLimits, ExportRequest, ExportSpace};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(History::undo_depth)
        .unwrap_or(0)
}

fn create_payload(
    rows: u32,
    cols: u32,
    col_width: f64,
    row_height: f64,
    origin: Point3,
) -> CommandPayload {
    CommandPayload::TableCreate(Box::new(TableCreateCommand {
        rows,
        cols,
        col_width,
        row_height,
        origin,
    }))
}

fn edit_payload(row: u32, col: u32, text: &str) -> CommandPayload {
    CommandPayload::TableEdit(Box::new(TableEditCommand {
        row,
        col,
        text: text.into(),
    }))
}

fn create_table(
    app: &mut Application,
    session: &mut SessionState,
    rows: u32,
    cols: u32,
    col_width: f64,
    row_height: f64,
    origin: Point3,
) -> EntityId {
    let outcome = execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::TableCreate),
        create_payload(rows, cols, col_width, row_height, origin),
    )
    .expect("TABLE creates one table");
    assert_eq!(outcome.objects.len(), 1);
    EntityId(outcome.objects[0].0)
}

fn stored_table_of(app: &Application, id: EntityId) -> TableGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .expect("the table entity");
    match &entity.geometry {
        SemanticGeometry::Table(table) => table.as_ref().clone(),
        other => panic!("expected a table, got {other:?}"),
    }
}

/// Everything a refusal must leave untouched.
fn assert_no_write(app: &Application, before_revision: Revision, before_undo: usize) {
    assert_eq!(
        revision(app),
        before_revision,
        "a refusal bumps no revision"
    );
    assert_eq!(undo_depth(app), before_undo, "a refusal records no undo");
}

#[test]
fn table_create_builds_the_exact_grid_in_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    session.active_layer = LayerId(3);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableCreate),
        create_payload(3, 2, 5.0, 2.0, p3(10.0, 20.0, 0.0)),
    )
    .expect("TABLE creates one table");
    assert_eq!(outcome.objects.len(), 1, "one table entity");
    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    let id = EntityId(outcome.objects[0].0);
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .cloned()
        .expect("the stored table");
    assert_eq!(entity.layer, LayerId(3), "created on the active layer");
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbTable");
    let table = stored_table_of(&app, id);
    assert_eq!(table.rows().len(), 3);
    assert_eq!(table.columns().len(), 2);
    for column in table.columns() {
        assert!((column.width - 5.0).abs() < 1e-9, "{column:?}");
    }
    for row in table.rows() {
        assert!((row.height - 2.0).abs() < 1e-9, "{row:?}");
    }
    let frame = table.frame();
    assert_eq!(frame.origin, p3(10.0, 20.0, 0.0));
    assert_eq!(frame.horizontal, p3(1.0, 0.0, 0.0));
    assert_eq!(frame.normal, p3(0.0, 0.0, 1.0));
    assert_eq!(table.style_name(), "Standard");
    assert!(table.texts().is_empty(), "every cell starts empty");
    for row in 0..3 {
        for col in 0..2 {
            let cell = table.cell(row, col).expect("the cell");
            assert_eq!(cell.text, "");
            assert_eq!(cell.text_height, cad_domain::DEFAULT_CELL_TEXT_HEIGHT);
            assert_eq!(cell.style_name, "Standard");
            assert_eq!(cell.merge_width, 1);
            assert_eq!(cell.merge_height, 1);
            assert_eq!(cell.borders.top, cad_domain::TableBorder::VISIBLE);
            assert_eq!(cell.borders.right, cad_domain::TableBorder::VISIBLE);
            assert_eq!(cell.borders.bottom, cad_domain::TableBorder::VISIBLE);
            assert_eq!(cell.borders.left, cad_domain::TableBorder::VISIBLE);
        }
    }
    assert!(
        !table.display().is_empty(),
        "the grid has a derived display"
    );

    let created = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "table.created")
        .expect("table.created");
    for needle in ["3", "2", "5", "2", "10", "20"] {
        assert!(created.message.contains(needle), "{}", created.message);
    }

    // UNDO removes the whole table; REDO restores the byte-identical row.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(id)
            .is_none(),
        "undo removes the table"
    );
    assert_eq!(undo_depth(&app), 0);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(id)
            .unwrap(),
        &entity,
        "redo restores the exact entity"
    );
}

#[test]
fn dash_table_uses_the_dash_diagnostic_code() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::DashTable),
        create_payload(1, 1, 2.5, 2.5, point(0.0, 0.0)),
    )
    .expect("-TABLE creates the same table");
    assert_eq!(outcome.objects.len(), 1);
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "-table.created"),
        "{outcome:?}"
    );
}

#[test]
fn table_create_refusals_write_nothing() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let cases: [(CommandPayload, &str); 14] = [
        (
            create_payload(0, 2, 5.0, 2.0, point(0.0, 0.0)),
            "table.invalid_dimensions",
        ),
        (
            create_payload(3, 0, 5.0, 2.0, point(0.0, 0.0)),
            "table.invalid_dimensions",
        ),
        (
            create_payload(1001, 2, 5.0, 2.0, point(0.0, 0.0)),
            "table.create_too_large",
        ),
        (
            create_payload(2, 1001, 5.0, 2.0, point(0.0, 0.0)),
            "table.create_too_large",
        ),
        (
            create_payload(1000, 200, 5.0, 2.0, point(0.0, 0.0)),
            "table.create_too_large",
        ),
        (
            create_payload(3, 2, f64::NAN, 2.0, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 0.0, 2.0, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, -5.0, 2.0, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 5.0, f64::INFINITY, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 5.0, f64::NAN, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 5.0, -2.0, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 5.0, 0.0, point(0.0, 0.0)),
            "table.invalid_size",
        ),
        (
            create_payload(3, 2, 5.0, 2.0, p3(f64::NAN, 0.0, 0.0)),
            "table.invalid_origin",
        ),
        (
            create_payload(3, 2, 5.0, 2.0, p3(0.0, f64::INFINITY, 0.0)),
            "table.invalid_origin",
        ),
    ];
    for (payload, code) in cases {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableCreate),
            payload,
        )
        .expect_err("the table must be refused");
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with(code)),
            "expected {code}, got {error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .len(),
        0,
        "no table was inserted"
    );
}

#[test]
fn a_created_table_exports_and_reimports_with_the_same_sizes() {
    let (mut app, mut session) = application_with_document();
    create_table(&mut app, &mut session, 3, 2, 5.0, 2.0, p3(10.0, 20.0, 0.0));
    let (bytes, report) = export(&ExportRequest {
        database: &app.workspace.documents[&DocumentId(1)].drawing,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .expect("the drawing exports");
    let text = std::str::from_utf8(&bytes).expect("ASCII DXF");
    assert!(
        text.lines().any(|line| line.trim() == "TABLE"),
        "a real TABLE record is written, not a flattened display: {report:?}"
    );
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.code == "export.converted.table"),
        "the table exporter owns the record (converted, with its named losses): {report:?}"
    );

    let request = ImportRequest {
        document: DocumentId(2),
        database: DatabaseId(2),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let imported = AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("the exported drawing re-imports");
    let reimported = imported
        .database
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("the created table re-imports as a TABLE variant");
    assert_eq!(reimported.rows().len(), 3);
    assert_eq!(reimported.columns().len(), 2);
    for column in reimported.columns() {
        assert!((column.width - 5.0).abs() < 1e-9, "{column:?}");
    }
    for row in reimported.rows() {
        assert!((row.height - 2.0).abs() < 1e-9, "{row:?}");
    }
    let frame = reimported.frame();
    assert!((frame.origin.x - 10.0).abs() < 1e-9, "{frame:?}");
    assert!((frame.origin.y - 20.0).abs() < 1e-9, "{frame:?}");
}

#[test]
fn tabledit_sets_one_cell_with_spaces_in_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 3, 2, 5.0, 2.0, p3(10.0, 20.0, 0.0));
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    session.selection.replace([ref_of(id.0)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(2, 1, "hello world"),
    )
    .expect("TABLEDIT edits the cell");
    assert_eq!(outcome.objects, vec![ObjectId(id.0)]);
    let changes = outcome.changes.as_ref().expect("change set");
    assert_eq!(changes.after.0, before_revision.0 + 1, "one revision bump");
    assert_eq!(changes.changes.len(), 1, "one transaction");
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");

    let table = stored_table_of(&app, id);
    assert_eq!(table.cell(1, 0).unwrap().text, "hello world");
    assert_eq!(table.texts(), vec!["hello world".to_string()]);
    assert!(
        table.display().iter().any(|child| matches!(
            child,
            SemanticGeometry::Text { text, .. } if text == "hello world"
        )),
        "the derived display carries the new text: {table:?}"
    );
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "tabledit.cell")
        .expect("tabledit.cell");
    assert!(diagnostic.message.contains("2"), "{}", diagnostic.message);
    assert!(diagnostic.message.contains("1"), "{}", diagnostic.message);
    assert!(
        diagnostic.message.contains(&id.0.to_string()),
        "{}",
        diagnostic.message
    );

    // UNDO restores the empty cell; REDO re-applies the same text.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_table_of(&app, id).cell(1, 0).unwrap().text, "");
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        stored_table_of(&app, id).cell(1, 0).unwrap().text,
        "hello world"
    );

    // FIND sees the edited text in the table.
    let report = execute(
        &mut app,
        &mut session,
        CommandId::FindReport,
        CommandPayload::TextFind {
            needle: "hello world".into(),
            replacement: None,
        },
    )
    .unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "find.result"
                && diagnostic.message.contains("1 match(es)")),
        "{report:?}"
    );
}

#[test]
fn tabledit_without_text_clears_the_cell() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 1, 1, 5.0, 2.0, point(0.0, 0.0));
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "draft"),
    )
    .unwrap();
    assert_eq!(stored_table_of(&app, id).cell(0, 0).unwrap().text, "draft");
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, ""),
    )
    .expect("an empty text clears the cell");
    assert!(outcome.changes.is_some());
    assert_eq!(stored_table_of(&app, id).cell(0, 0).unwrap().text, "");
    assert!(stored_table_of(&app, id).texts().is_empty());
}

#[test]
fn tabledit_with_the_same_text_is_a_no_op() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 1, 1, 5.0, 2.0, point(0.0, 0.0));
    session.selection.replace([ref_of(id.0)]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "same"),
    )
    .unwrap();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "same"),
    )
    .expect("an identical text is a no-op");
    assert!(outcome.changes.is_none(), "{outcome:?}");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "tabledit.unchanged"),
        "{outcome:?}"
    );
    assert_no_write(&app, before_revision, before_undo);
}

#[test]
fn tabledit_refusals_write_nothing() {
    let (mut app, mut session) = application_with_document();
    let id = create_table(&mut app, &mut session, 3, 2, 5.0, 2.0, point(0.0, 0.0));
    let line = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        })),
    )
    .expect("a line for the not-table case");
    let line_id = EntityId(line.objects[0].0);
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    // No selection.
    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "x"),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.no_selection")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);

    // Multiple selection.
    session.selection.replace([ref_of(id.0), ref_of(line_id.0)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "x"),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.multiple")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);

    // A selected non-table.
    session.selection.replace([ref_of(line_id.0)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TableEdit),
        edit_payload(1, 1, "x"),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.not_table")),
        "{error:?}"
    );
    assert_no_write(&app, before_revision, before_undo);

    // Out-of-range addresses: zero (outside the 1-based space), row 4 and col 3.
    session.selection.replace([ref_of(id.0)]);
    for (row, col) in [(0, 1), (1, 0), (4, 1), (1, 3)] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::TableEdit),
            edit_payload(row, col, "x"),
        )
        .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(message) if message.starts_with("tabledit.out_of_range")),
            "({row}, {col}): {error:?}"
        );
        assert_no_write(&app, before_revision, before_undo);
    }
    assert!(stored_table_of(&app, id).texts().is_empty());
}
