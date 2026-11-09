//! The 2026-11-07 utility/named-object batch contracts: `PURGE`, `RENAME`,
//! `SETVAR` (report half), `TIME`, `DBLIST`, `TREESTAT`, `MREDO`, `MULTIPLE`.
//!
//! The write half of `SETVAR` is a typed command-line path and is pinned in
//! `cad-ui-core`; here the report path and the closed-set refusal are checked.
//! Every read-only command is also checked to leave the revision and undo history
//! untouched.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn entity(id: u128, layer: LayerId) -> DbEntity {
    DbEntity {
        object: DbObject {
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
            end: point(4.0, 0.0),
        },
        draw_order: id as i64,
    }
}

/// Layers `0` (id 0), `WALLS` (id 3, referenced) and `DOORS` (id 5, unused).
fn app_with_purge_fixture() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(3), "WALLS"),
        (LayerId(5), "DOORS"),
    ] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder.insert_entity(entity(1, LayerId(3))).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    (app, session)
}

fn layer_names(app: &Application) -> Vec<String> {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    let mut names: Vec<String> = drawing.layers().map(|row| row.name.clone()).collect();
    names.sort();
    names
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn codes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

// ---------------------------------------------------------------------------
// PURGE / -PURGE
// ---------------------------------------------------------------------------

#[test]
fn purge_removes_only_unused_layers_and_refuses_referenced_ones() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Purge,
        CommandPayload::None,
    )
    .expect("bare PURGE purges what it can");
    assert_eq!(layer_names(&app), vec!["0", "WALLS"]);
    assert!(revision(&app) > before);
    let codes = codes(&outcome);
    assert!(codes.contains(&"purge.layer_removed"), "{codes:?}");
    assert!(codes.contains(&"purge.layer_refused"), "{codes:?}");
    assert!(codes.contains(&"purge.kind_unsupported"), "{codes:?}");
    let refused = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "purge.layer_refused")
        .unwrap();
    assert!(refused.message.contains("WALLS"), "{}", refused.message);
    // Layer-table writes are not undoable.
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn purge_layer_narrows_to_the_layer_kind_and_refuses_unsupported_kinds_by_name() {
    let (mut app, mut session) = app_with_purge_fixture();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Purge,
        CommandPayload::Purge(PurgeTarget::Layer),
    )
    .expect("PURGE LAYER is implemented");
    assert_eq!(layer_names(&app), vec!["0", "WALLS"]);
    assert!(!codes(&outcome).contains(&"purge.kind_unsupported"));

    for target in [
        PurgeTarget::Block,
        PurgeTarget::Style,
        PurgeTarget::Linetype,
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Purge,
            CommandPayload::Purge(target),
        )
        .expect_err("no delete API for this kind");
        assert!(error.to_string().contains("unsupported"), "{error}");
    }
}

// ---------------------------------------------------------------------------
// RENAME / -RENAME
// ---------------------------------------------------------------------------

#[test]
fn rename_layer_succeeds_and_refuses_duplicate_unknown_and_illegal_names() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::Rename,
        CommandPayload::Rename(RenameCommand {
            kind: NamedObjectKind::Layer,
            old: "DOORS".into(),
            new: "ENTRY".into(),
        }),
    )
    .expect("a legal rename succeeds");
    assert!(layer_names(&app).contains(&"ENTRY".to_string()));
    assert!(revision(&app) > before);
    assert!(!app.can_undo(&DocumentId(1)));

    let before_failure = revision(&app);
    for (old, new, code) in [
        ("ENTRY", "walls", "layer.duplicate_name"),
        ("NOPE", "X", "layer.unknown"),
        ("ENTRY", "A/B", "layer.name_illegal_char"),
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Rename,
            CommandPayload::Rename(RenameCommand {
                kind: NamedObjectKind::Layer,
                old: old.into(),
                new: new.into(),
            }),
        )
        .expect_err("rename refusal");
        assert!(error.to_string().contains(code), "{error}");
        assert_eq!(revision(&app), before_failure);
    }
}

#[test]
fn rename_of_a_kind_without_an_api_is_a_named_refusal() {
    let (mut app, mut session) = app_with_purge_fixture();
    for kind in [
        NamedObjectKind::Block,
        NamedObjectKind::Style,
        NamedObjectKind::Linetype,
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Rename,
            CommandPayload::Rename(RenameCommand {
                kind,
                old: "A".into(),
                new: "B".into(),
            }),
        )
        .expect_err("no rename API");
        assert!(error.to_string().contains("unsupported"), "{error}");
    }
}

// ---------------------------------------------------------------------------
// SETVAR (report half)
// ---------------------------------------------------------------------------

#[test]
fn setvar_reports_the_closed_set_and_refuses_unknown_or_missing_names() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);
    for (name, expected) in [
        ("PDMODE", "0"),
        ("PDSIZE", "0"),
        ("SNAPUNIT", "10"),
        ("LTSCALE", "1"),
        ("GRID", "false"),
        ("SNAP", "false"),
        ("ORTHO", "false"),
        ("TEXTSIZE", "2.5"),
    ] {
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::SetVar,
            CommandPayload::SetVar(name.into()),
        )
        .expect("a modelled variable reports");
        let report = outcome
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "setvar.value")
            .expect("setvar.value diagnostic");
        assert!(
            report.message.contains(expected),
            "{name}: {}",
            report.message
        );
    }
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetVar,
        CommandPayload::SetVar("NOT_A_VAR".into()),
    )
    .expect_err("unknown variable");
    assert!(
        error.to_string().contains("setvar.unknown_variable"),
        "{error}"
    );

    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetVar,
        CommandPayload::None,
    )
    .expect_err("missing name");
    assert!(error.to_string().contains("setvar.needs_name"), "{error}");
}

// ---------------------------------------------------------------------------
// TIME / DBLIST / TREESTAT (read-only)
// ---------------------------------------------------------------------------

#[test]
fn time_reports_the_document_and_clock_without_writing() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Time,
        CommandPayload::None,
    )
    .expect("TIME is read-only");
    let codes = codes(&outcome);
    assert!(codes.contains(&"time.document"), "{codes:?}");
    assert!(codes.contains(&"time.utc"), "{codes:?}");
    assert!(codes.contains(&"time.not_tracked"), "{codes:?}");
    assert!(codes.contains(&"time.uptime_not_tracked"), "{codes:?}");
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dblist_lists_top_level_entities_without_writing() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::DbList,
        CommandPayload::None,
    )
    .expect("DBLIST is read-only");
    let objects = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "dblist.object")
        .count();
    assert_eq!(objects, 1);
    let summary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "dblist.summary")
        .expect("summary");
    assert!(
        summary.message.contains("1 top-level"),
        "{}",
        summary.message
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn treestat_reports_not_tracked_without_a_fake_count() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TreeStat,
        CommandPayload::None,
    )
    .expect("TREESTAT is read-only");
    let report = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "treestat.not_tracked")
        .expect("not-tracked report");
    assert!(report.message.contains("not tracked"), "{}", report.message);
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

// ---------------------------------------------------------------------------
// MREDO
// ---------------------------------------------------------------------------

#[test]
fn mredo_applies_each_requested_step_and_refuses_an_empty_stack() {
    let (mut app, mut session) = application_with_document();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::MRedo,
        CommandPayload::None,
    )
    .expect_err("no redo stack");
    assert!(error.to_string().contains("mredo.empty"), "{error}");

    for index in 0..2 {
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(index as f64, 0.0), point(index as f64, 1.0)]),
        )
        .expect("create");
    }
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert_eq!(entity_count(&app), 0);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MRedo,
        CommandPayload::MRedo(2),
    )
    .expect("MREDO 2");
    assert_eq!(entity_count(&app), 2);
    assert!(
        codes(&outcome).contains(&"mredo.applied"),
        "{:?}",
        codes(&outcome)
    );

    let error = execute(
        &mut app,
        &mut session,
        CommandId::MRedo,
        CommandPayload::MRedo(0),
    )
    .expect_err("zero is refused");
    assert!(error.to_string().contains("mredo.count"), "{error}");
}

// ---------------------------------------------------------------------------
// MULTIPLE
// ---------------------------------------------------------------------------

#[test]
fn multiple_arms_the_session_flag_and_reports_the_missing_repeat_loop() {
    let (mut app, mut session) = app_with_purge_fixture();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Multiple,
        CommandPayload::None,
    )
    .expect("MULTIPLE arms session state");
    assert_eq!(
        session.multiple_target(),
        Some(&MultipleRepeat::NextCommand)
    );
    let codes = codes(&outcome);
    assert!(codes.contains(&"multiple.armed"), "{codes:?}");
    assert!(
        codes.contains(&"multiple.repeat_not_implemented"),
        "{codes:?}"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Multiple,
        CommandPayload::Multiple(Some("LINE".into())),
    )
    .expect("MULTIPLE LINE arms");
    assert_eq!(
        session.multiple_target(),
        Some(&MultipleRepeat::Command("LINE".into()))
    );
    session.clear_multiple();
    assert!(session.multiple_target().is_none());
}
