//! Contract tests for the post-import style-table commands (`STYLE`, `-STYLE`,
//! `DIMSTYLE`, `MLEADERSTYLE`) and `RENAME` over the writable tables.
//!
//! Every write is one revision and **one** undo step that restores the exact
//! before row; every refusal is a stable named reason that leaves the revision,
//! the tables and the history untouched.

use super::*;
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DimStyle, DrawingDatabaseBuilder, Layer, MLeaderStyle,
    Style,
};

fn text_entity(id: u128, style: StyleId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "note".into(),
            position: point(0.0, 0.0),
            style,
            height: 2.5,
            rotation: 0.0,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
        },
        draw_order: id as i64,
    }
}

/// Layers `0`/`WALLS`, text styles `Standard`(0)/`Note`(1), dim styles
/// `Standard`(0)/`Fancy`(1), multileader `Standard`(0), block `PART`(7), and one
/// text entity using `Note`.
fn style_app() -> (Application, SessionState) {
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
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: vec!["txt.shx".into()],
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(1),
            name: "Note".into(),
            resource_keys: vec!["arial.ttf".into()],
        })
        .unwrap();
    builder
        .insert_dim_style(DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    let mut fancy = DimStyle::standard_metric(DimStyleId(1));
    fancy.name = "Fancy".into();
    builder.insert_dim_style(fancy).unwrap();
    builder
        .insert_mleader_style(MLeaderStyle::standard_metric(MLeaderStyleId(0)))
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(7),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .set_block_metadata(BlockId(7), "PART", point(0.0, 0.0))
        .unwrap();
    builder.insert_entity(text_entity(1, StyleId(1))).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());
    (app, session)
}

fn drawing(app: &Application) -> &cad_db::DrawingDatabase {
    &app.workspace.documents[&DocumentId(1)].drawing
}

fn revision(app: &Application) -> Revision {
    drawing(app).revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn style_table(kind: StyleTableKind, op: StyleTableOp) -> CommandPayload {
    CommandPayload::StyleTable(StyleTableCommand { kind, op })
}

fn style_command(
    app: &mut Application,
    session: &mut SessionState,
    op: StyleTableOp,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::TextStyle),
        style_table(StyleTableKind::TextStyle, op),
    )
}

fn mleader_command(
    app: &mut Application,
    session: &mut SessionState,
    op: StyleTableOp,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(ExtendedCommandId::MLeaderStyleList),
        style_table(StyleTableKind::MLeaderStyle, op),
    )
}

fn dimstyle_command(
    app: &mut Application,
    session: &mut SessionState,
    option: DimStyleCommand,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::DimStyle,
        CommandPayload::Dimension(Box::new(DimensionBatchPayload::Style(option))),
    )
}

// ---------------------------------------------------------------------------
// STYLE
// ---------------------------------------------------------------------------

#[test]
fn style_new_rename_delete_are_one_revision_and_one_undo_each() {
    let (mut app, mut session) = style_app();
    let start = revision(&app);

    let outcome = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Create {
            name: "Header".into(),
            from: Some("Note".into()),
        },
    )
    .expect("STYLE NEW");
    assert!(outcome.changes.is_some());
    let created = drawing(&app)
        .style_by_name("Header")
        .expect("created row")
        .id;
    assert_eq!(
        drawing(&app).style(created).unwrap().resource_keys,
        vec!["arial.ttf".to_string()],
        "the base row's font keys are copied"
    );
    assert_eq!(revision(&app), Revision(start.0 + 1));
    assert_eq!(undo_depth(&app), 1);

    style_command(
        &mut app,
        &mut session,
        StyleTableOp::Rename {
            old: "Header".into(),
            new: "Title".into(),
        },
    )
    .expect("STYLE RENAME");
    assert!(drawing(&app).style_by_name("Header").is_none());
    assert_eq!(drawing(&app).style_by_name("Title").unwrap().id, created);
    assert_eq!(revision(&app), Revision(start.0 + 2));
    assert_eq!(undo_depth(&app), 2);

    style_command(
        &mut app,
        &mut session,
        StyleTableOp::Delete {
            name: "Title".into(),
        },
    )
    .expect("STYLE DELETE");
    assert!(drawing(&app).style_by_name("Title").is_none());
    assert_eq!(revision(&app), Revision(start.0 + 3));
    assert_eq!(undo_depth(&app), 3);

    // Undo the delete: the row returns with its id and values.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    let restored = drawing(&app).style_by_name("Title").expect("restored row");
    assert_eq!(restored.id, created);
    assert_eq!(restored.resource_keys, vec!["arial.ttf".to_string()]);
    assert_eq!(undo_depth(&app), 2);
}

#[test]
fn style_set_font_and_keys_write_one_row_at_a_time() {
    let (mut app, mut session) = style_app();
    style_command(
        &mut app,
        &mut session,
        StyleTableOp::Modify {
            name: "Note".into(),
            assignments: vec![("FONT".into(), "news.ttf".into())],
        },
    )
    .expect("STYLE SET FONT");
    assert_eq!(
        drawing(&app).style_by_name("Note").unwrap().resource_keys,
        vec!["news.ttf".to_string()]
    );
    style_command(
        &mut app,
        &mut session,
        StyleTableOp::Modify {
            name: "Note".into(),
            assignments: vec![("KEYS".into(), "a.ttf,b.ttf".into())],
        },
    )
    .expect("STYLE SET KEYS");
    assert_eq!(
        drawing(&app).style_by_name("Note").unwrap().resource_keys,
        vec!["a.ttf".to_string(), "b.ttf".to_string()]
    );
    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Modify {
            name: "Note".into(),
            assignments: vec![("nope".into(), "x".into())],
        },
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("style.unknown_field"),
        "{error}"
    );
}

#[test]
fn style_refusals_name_the_reason_and_change_nothing() {
    let (mut app, mut session) = style_app();
    let start = revision(&app);
    let depth = undo_depth(&app);

    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Create {
            name: "note".into(),
            from: None,
        },
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("style.duplicate_name"),
        "{error}"
    );

    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Create {
            name: "a/b".into(),
            from: None,
        },
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("style.name_illegal_char"),
        "{error}"
    );

    // `Note` is used by a text entity, so it cannot be deleted.
    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Delete {
            name: "Note".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{error}").contains("style.in_use"), "{error}");

    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Rename {
            old: "Missing".into(),
            new: "Other".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{error}").contains("style.unknown"), "{error}");

    assert_eq!(revision(&app), start);
    assert_eq!(undo_depth(&app), depth);
}

#[test]
fn style_rename_cascades_into_dim_and_leader_text_style_names() {
    let (mut app, mut session) = style_app();
    // Point the dim/leader rows at `Standard` and rename it; the cascade is part
    // of the same transaction (one revision).
    let start = revision(&app);
    style_command(
        &mut app,
        &mut session,
        StyleTableOp::Rename {
            old: "Standard".into(),
            new: "Body".into(),
        },
    )
    .expect("cascade rename");
    assert_eq!(revision(&app), Revision(start.0 + 1));
    assert_eq!(
        drawing(&app)
            .dim_style_by_name("Standard")
            .unwrap()
            .text_style_name,
        "Body"
    );
    assert_eq!(
        drawing(&app)
            .mleader_style_by_name("Standard")
            .unwrap()
            .text_style_name,
        "Body"
    );
}

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

// ---------------------------------------------------------------------------
// MLEADERSTYLE
// ---------------------------------------------------------------------------

#[test]
fn mleaderstyle_set_new_modify_delete_write_one_undo_each() {
    let (mut app, mut session) = style_app();
    let start = revision(&app);

    let outcome = mleader_command(
        &mut app,
        &mut session,
        StyleTableOp::Create {
            name: "Callout".into(),
            from: None,
        },
    )
    .expect("MLEADERSTYLE NEW");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|d| d.code == "mleaderstyle.created"));
    assert_eq!(revision(&app), Revision(start.0 + 1));
    assert_eq!(session.current_mleader_style_name(), "Callout");
    assert_eq!(undo_depth(&app), 1);

    mleader_command(
        &mut app,
        &mut session,
        StyleTableOp::Modify {
            name: "Callout".into(),
            assignments: vec![
                ("TEXT_HEIGHT".into(), "3.5".into()),
                ("PATH_TYPE".into(), "SPLINE".into()),
            ],
        },
    )
    .expect("MLEADERSTYLE MODIFY");
    let row = drawing(&app).mleader_style_by_name("Callout").unwrap();
    assert_eq!(row.text_height, 3.5);
    assert_eq!(row.path_type, cad_domain::MLeaderPathType::Spline);

    mleader_command(
        &mut app,
        &mut session,
        StyleTableOp::Set {
            name: "Standard".into(),
        },
    )
    .expect("MLEADERSTYLE SET");
    assert_eq!(session.current_mleader_style_name(), "Standard");

    mleader_command(
        &mut app,
        &mut session,
        StyleTableOp::Delete {
            name: "Callout".into(),
        },
    )
    .expect("MLEADERSTYLE DELETE");
    assert!(drawing(&app).mleader_style_by_name("Callout").is_none());

    let error = mleader_command(
        &mut app,
        &mut session,
        StyleTableOp::Delete {
            name: "Standard".into(),
        },
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("mleaderstyle.standard_undeletable"),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// RENAME
// ---------------------------------------------------------------------------

fn rename(
    app: &mut Application,
    session: &mut SessionState,
    kind: NamedObjectKind,
    old: &str,
    new: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Rename,
        CommandPayload::Rename(RenameCommand {
            kind,
            old: old.into(),
            new: new.into(),
        }),
    )
}

#[test]
fn rename_covers_every_writable_kind_and_refuses_unknown_names() {
    let (mut app, mut session) = style_app();
    rename(
        &mut app,
        &mut session,
        NamedObjectKind::Style,
        "Note",
        "Callout",
    )
    .expect("RENAME STYLE");
    assert!(drawing(&app).style_by_name("Callout").is_some());

    rename(
        &mut app,
        &mut session,
        NamedObjectKind::DimStyle,
        "Fancy",
        "Detail",
    )
    .expect("RENAME DIMSTYLE");
    assert!(drawing(&app).dim_style_by_name("Detail").is_some());

    rename(
        &mut app,
        &mut session,
        NamedObjectKind::MLeaderStyle,
        "Standard",
        "Basic",
    )
    .expect("RENAME MLEADERSTYLE");
    assert_eq!(
        drawing(&app).mleader_style_by_name("Basic").unwrap().id,
        MLeaderStyleId(0)
    );

    rename(
        &mut app,
        &mut session,
        NamedObjectKind::Block,
        "PART",
        "COMPONENT",
    )
    .expect("RENAME BLOCK");
    assert!(drawing(&app).block_by_name("COMPONENT").is_some());

    let error = rename(&mut app, &mut session, NamedObjectKind::Style, "Nope", "X").unwrap_err();
    assert!(format!("{error}").contains("style.unknown"), "{error}");

    // LINETYPE still has no write API: the refusal is named, not silent.
    let error = rename(
        &mut app,
        &mut session,
        NamedObjectKind::Linetype,
        "Continuous",
        "Dashed",
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("rename.linetype.unsupported"),
        "{error}"
    );
}

#[test]
fn rename_block_is_one_undo_step_and_undo_restores_the_name() {
    let (mut app, mut session) = style_app();
    let depth = undo_depth(&app);
    rename(
        &mut app,
        &mut session,
        NamedObjectKind::Block,
        "PART",
        "COMPONENT",
    )
    .expect("RENAME BLOCK");
    assert_eq!(undo_depth(&app), depth + 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo");
    assert!(drawing(&app).block_by_name("PART").is_some());
    assert!(drawing(&app).block_by_name("COMPONENT").is_none());
}

#[test]
fn a_written_table_command_is_refused_in_viewer_mode() {
    let (mut app, mut session) = style_app();
    let _ = session.switch_mode(AppMode::Viewer).unwrap();
    let start = revision(&app);
    let error = style_command(
        &mut app,
        &mut session,
        StyleTableOp::Create {
            name: "Header".into(),
            from: None,
        },
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), start);

    // The read-only list stays Viewer-legal.
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TextStyle),
        CommandPayload::None,
    )
    .expect("STYLE report is Viewer-legal");
}
