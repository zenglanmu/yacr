//! The 2026-11-07 utility/named-object batch contracts: `PURGE`, `RENAME`,
//! `SETVAR` (report half), `TIME`, `DBLIST`, `TREESTAT`, `MREDO`, `MULTIPLE`.
//!
//! The write half of `SETVAR` is a typed command-line path and is pinned in
//! `cad-ui-core`; here the report path and the closed-set refusal are checked.
//! Every read-only command is also checked to leave the revision and undo history
//! untouched.

use super::*;
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

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

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
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
    // There is no block/style/linetype row in this fixture, so the table half is
    // a named no-op rather than a refusal.
    assert!(codes.contains(&"purge.tables_none"), "{codes:?}");
    let refused = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "purge.layer_refused")
        .unwrap();
    assert!(refused.message.contains("WALLS"), "{}", refused.message);
    // PURGE layer deletions are not undoable (out of scope for the layer lane).
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn purge_layer_narrows_to_the_layer_kind() {
    let (mut app, mut session) = app_with_purge_fixture();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Purge,
        CommandPayload::Purge(PurgeTarget::Layer),
    )
    .expect("PURGE LAYER is implemented");
    assert_eq!(layer_names(&app), vec!["0", "WALLS"]);
    assert!(!codes(&outcome).contains(&"purge.tables_none"));
    assert!(!codes(&outcome).contains(&"purge.block_removed"));
}

/// The three table kinds a `PURGE` now really removes: an unused block
/// definition (with its members), an unused text style and an unused linetype.
/// Each in-use row is a named per-object refusal that removes nothing, and the
/// whole table half is one undo step that restores rows *and* block members.
fn purge_table_fixture() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    builder.insert_entity(entity(1, LayerId(3))).unwrap();

    // Blocks: 7 is unreferenced (member 5), 8 is used by INSERT 7.
    builder
        .insert_block(BlockDefinition {
            id: BlockId(7),
            entities: vec![EntityId(5)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(7), "UNUSED", point(0.0, 0.0))
        .unwrap();
    let mut member = entity(5, LayerId(0));
    member.space = SpaceId::Block(BlockId(7));
    builder.insert_entity(member).unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(8),
            entities: vec![EntityId(6)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(8), "USED", point(0.0, 0.0))
        .unwrap();
    let mut used_member = entity(6, LayerId(0));
    used_member.space = SpaceId::Block(BlockId(8));
    builder.insert_entity(used_member).unwrap();
    let mut reference = entity(7, LayerId(0));
    reference.geometry = SemanticGeometry::Insert {
        block: BlockId(8),
        transform: Transform3::translation(point(0.0, 0.0)),
        attributes: Vec::new(),
    };
    builder.insert_entity(reference).unwrap();

    // Text styles: Standard, an unused NOTES and a used TITLE.
    for (id, name) in [(1u128, "Standard"), (2, "NOTES"), (3, "TITLE")] {
        builder
            .insert_style(cad_db::Style {
                id: StyleId(id),
                name: name.into(),
                resource_keys: Vec::new(),
            })
            .unwrap();
    }
    let mut title = entity(9, LayerId(0));
    title.geometry = SemanticGeometry::Text {
        position: point(0.0, 0.0),
        height: 1.0,
        text: "title".into(),
        style: StyleId(3),
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        line_spacing: TextLineSpacing::default(),
        width_factor: 1.0,
        mirrored: false,
    };
    builder.insert_entity(title).unwrap();

    // Linetypes: Continuous, an unused DASHED and a used PHANTOM (by layer 0).
    for (id, name) in [(1u128, "Continuous"), (2, "DASHED"), (3, "PHANTOM")] {
        builder
            .insert_linetype(cad_db::LineType {
                id: LinetypeId(id),
                name: name.into(),
                pattern: cad_db::LinetypePattern::default(),
                complex: false,
            })
            .unwrap();
    }
    let mut drawing = builder.finish().unwrap();
    drawing
        .set_layer_attributes(
            LayerId(0),
            cad_db::LayerAttributes {
                linetype: "PHANTOM".into(),
                ..cad_db::LayerAttributes::default()
            },
            TransactionId(1),
            "fixture layer attributes",
        )
        .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

#[test]
fn purge_removes_unused_block_style_and_linetype_rows_in_one_undo_step() {
    let (mut app, mut session) = purge_table_fixture();
    let depth_before = undo_depth(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Purge,
        CommandPayload::None,
    )
    .expect("bare PURGE purges every table");
    let codes = codes(&outcome);
    assert!(codes.contains(&"purge.block_removed"), "{codes:?}");
    assert!(codes.contains(&"purge.style_removed"), "{codes:?}");
    assert!(codes.contains(&"purge.linetype_removed"), "{codes:?}");
    assert!(codes.contains(&"purge.block_refused"), "{codes:?}");
    assert!(codes.contains(&"purge.style_refused"), "{codes:?}");
    assert!(codes.contains(&"purge.linetype_refused"), "{codes:?}");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.block(BlockId(7)).is_none(), "block 7 is unused");
    assert!(
        drawing.entity(EntityId(5)).is_none(),
        "its member went with it"
    );
    assert!(drawing.block(BlockId(8)).is_some(), "INSERT 7 uses block 8");
    assert!(drawing.entity(EntityId(6)).is_some());
    assert!(drawing.style(StyleId(2)).is_none(), "NOTES is unused");
    assert!(drawing.style(StyleId(1)).is_some(), "Standard stays");
    assert!(
        drawing.style(StyleId(3)).is_some(),
        "TITLE is used by text 9"
    );
    assert!(
        drawing.linetype(LinetypeId(2)).is_none(),
        "DASHED is unused"
    );
    assert!(
        drawing.linetype(LinetypeId(3)).is_some(),
        "PHANTOM is used by layer 0"
    );
    assert_eq!(undo_depth(&app), depth_before + 1);

    // One undo step restores the rows and the block member.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.block(BlockId(7)).is_some());
    assert_eq!(
        drawing.block(BlockId(7)).unwrap().entities,
        vec![EntityId(5)]
    );
    assert!(drawing.entity(EntityId(5)).is_some());
    assert!(drawing.style(StyleId(2)).is_some());
    assert!(drawing.linetype(LinetypeId(2)).is_some());
}

#[test]
fn purge_of_a_named_kind_touches_only_that_table() {
    let (mut app, mut session) = purge_table_fixture();
    execute(
        &mut app,
        &mut session,
        CommandId::Purge,
        CommandPayload::Purge(PurgeTarget::Style),
    )
    .expect("PURGE STYLE is implemented");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.style(StyleId(2)).is_none());
    assert!(
        drawing.block(BlockId(7)).is_some(),
        "PURGE STYLE must not touch blocks"
    );
    assert!(drawing.linetype(LinetypeId(2)).is_some());
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
    // RENAME LAYER reuses RenameLayer, which now records one layer-table step.
    assert_eq!(undo_depth(&app), 1, "RENAME records one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the rename");
    assert!(layer_names(&app).contains(&"DOORS".to_string()));
    assert!(!layer_names(&app).contains(&"ENTRY".to_string()));
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo the rename");
    assert!(layer_names(&app).contains(&"ENTRY".to_string()));
    assert_eq!(undo_depth(&app), 1, "undo and redo stay one step");

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
fn rename_of_an_unknown_name_is_a_named_refusal() {
    let (mut app, mut session) = app_with_purge_fixture();
    // Every RENAME kind now addresses a row; an unknown name is its own stable
    // refusal (the linetype write itself is pinned in `style_tables`).
    for (kind, code) in [
        (NamedObjectKind::Block, "block.unknown"),
        (NamedObjectKind::Style, "style.unknown"),
        (NamedObjectKind::DimStyle, "dimstyle.unknown"),
        (NamedObjectKind::MLeaderStyle, "mleaderstyle.unknown"),
        (NamedObjectKind::Linetype, "linetype.unknown"),
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
        .expect_err("unknown row");
        assert!(error.to_string().contains(code), "{kind:?}: {error}");
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

/// The four Wave-31 additions are modelled but a fresh drawing stores no value,
/// so the report names the absence instead of printing a default.
#[test]
fn setvar_names_the_modelled_variables_a_fresh_drawing_cannot_value() {
    let (mut app, mut session) = app_with_purge_fixture();
    let before = revision(&app);
    for name in ["INSBASE", "LIMMIN", "LIMMAX", "CVPORT"] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::SetVar,
            CommandPayload::SetVar(name.into()),
        )
        .expect_err("a modelled value with no stored data is refused by name, not defaulted");
        let text = error.to_string();
        assert!(
            text.contains("setvar.not_stored"),
            "{name}: {text} must name the absence"
        );
        assert!(
            text.contains(name),
            "{name}: the refusal must name the variable: {text}"
        );
    }
    assert_eq!(revision(&app), before, "a report writes nothing");
    assert!(!app.can_undo(&DocumentId(1)));
}

/// After `BASE`/`LIMITS` write the header, the same four names report the real
/// stored values through the one reader.
#[test]
fn setvar_reports_the_header_values_base_and_limits_stored() {
    let (mut app, mut session) = app_with_purge_fixture();
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::BasePoint),
        CommandPayload::Points(vec![point(4.0, 5.0)]),
    )
    .expect("BASE writes the insertion base point");
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::DrawingLimits),
        CommandPayload::Points(vec![point(0.0, 0.0), point(210.0, 297.0)]),
    )
    .expect("LIMITS writes both corners");
    for (name, expected) in [
        ("INSBASE", "4,5,0"),
        ("LIMMIN", "0,0"),
        ("LIMMAX", "210,297"),
    ] {
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::SetVar,
            CommandPayload::SetVar(name.into()),
        )
        .expect("the stored header value reports");
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
        assert!(
            report.message.contains("(point)"),
            "{name} must name its kind: {}",
            report.message
        );
    }
}

/// `CVPORT` reads the session's current-viewport marker of the **active paper
/// layout**, one-based, and has no value in model space.
#[test]
fn setvar_reports_cvport_only_inside_a_paper_layout_with_a_viewport() {
    let (mut app, mut session) = application_with_layouts();
    session.active_space = SpaceId::Paper(cad_domain::LayoutId(1));
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::SetVar,
        CommandPayload::SetVar("CVPORT".into()),
    )
    .expect("CVPORT reports inside a paper layout with a viewport");
    let report = &outcome.diagnostics[0];
    assert!(report.message.contains("CVPORT = 1"), "{}", report.message);
    assert!(report.message.contains("(integer)"), "{}", report.message);

    // Model space has no current viewport.
    session.active_space = SpaceId::Model;
    let error = execute(
        &mut app,
        &mut session,
        CommandId::SetVar,
        CommandPayload::SetVar("CVPORT".into()),
    )
    .expect_err("model space has no CVPORT");
    assert!(error.to_string().contains("setvar.not_stored"), "{error}");
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
    assert!(summary.message.contains("1 entity"), "{}", summary.message);
    assert!(
        summary.message.contains("1 model"),
        "the space split is reported: {}",
        summary.message
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "dblist.not_modelled"
                && diagnostic.message.contains("XDATA")),
        "unretained data is named"
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn dblist_lists_block_definition_members_as_their_own_rows() {
    let (mut app, mut session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let mut member = entity(10, LayerId(0));
    member.space = SpaceId::Block(BlockId(2));
    builder.insert_entity(member).unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(10)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(
            BlockId(2),
            "TABLE",
            cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    let before = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::DbList, CommandPayload::None),
        )
        .expect("DBLIST is read-only");
    let member_rows: Vec<&Diagnostic> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "dblist.block_object")
        .collect();
    assert_eq!(member_rows.len(), 1, "the block member is a listed row");
    assert!(
        member_rows[0].message.contains("space=Block(TABLE)"),
        "{}",
        member_rows[0].message
    );
    let summary = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "dblist.summary")
        .expect("summary");
    assert!(
        summary
            .message
            .contains("1 block member(s) in 1 definition"),
        "{}",
        summary.message
    );
    assert_eq!(revision(&app), before);
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
