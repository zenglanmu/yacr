//! Persisted group contract tests: `GROUP` / `-GROUP`, `UNGROUP`, `GROUPEDIT`.
//!
//! A group is document data (`tables/groups.json`): every real edit is one
//! revision, one `ChangeSet` and one undo step; read-only reports and refusals
//! write nothing. DXF/DWG round trips live with the connector; this file owns the
//! command/database/history contract.

use super::*;

const GROUP_LINE_A: u128 = 901;
const GROUP_LINE_B: u128 = 902;

fn line(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line { start, end },
        draw_order: order,
    }
}

/// Two model lines on layer 0, in one open document.
fn group_app() -> (Application, SessionState) {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(line(GROUP_LINE_A, point(0.0, 0.0), point(10.0, 0.0), 0))
        .unwrap();
    builder
        .insert_entity(line(GROUP_LINE_B, point(0.0, 10.0), point(10.0, 10.0), 1))
        .unwrap();
    let drawing = builder.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().copied().map(ref_of));
}

fn annotation(payload: AnnotationBatchPayload) -> CommandPayload {
    CommandPayload::Annotation(Box::new(payload))
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn drawing_of(app: &Application) -> &DrawingDatabase {
    app.workspace.documents[&DocumentId(1)].drawing.as_ref()
}

fn group_rows(app: &Application) -> Vec<(String, Vec<u128>)> {
    drawing_of(app)
        .groups()
        .map(|group| {
            (
                group.name.clone(),
                group.members.iter().map(|member| member.0).collect(),
            )
        })
        .collect()
}

#[test]
fn group_create_persists_one_revision_and_one_undo_step() {
    let (mut app, mut session) = group_app();
    let before = revision(&app);
    select(&mut session, &[GROUP_LINE_A, GROUP_LINE_B]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "group.created");
    let changes = out.changes.expect("a real write publishes a ChangeSet");
    assert_eq!(
        changes.scope.tables.iter().copied().collect::<Vec<_>>(),
        vec![cad_db::TableKind::Groups]
    );
    assert_eq!(changes.after.0, before.0 + 1, "exactly one revision");
    assert_eq!(revision(&app), changes.after);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        group_rows(&app),
        vec![("walls".to_string(), vec![GROUP_LINE_A, GROUP_LINE_B])]
    );

    // LIST is read-only: no revision, no history, no rows.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        CommandPayload::None,
    )
    .unwrap();
    assert!(out.changes.is_none());
    assert!(out.diagnostics.iter().any(|d| d.code == "group.entry"));
    assert!(out.diagnostics.iter().any(|d| d.code == "group.list"));
    assert_eq!(revision(&app), changes.after);
    assert_eq!(undo_depth(&app), 1);

    let before_delete = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Delete(
            "WALLS".into(),
        ))),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "group.deleted");
    assert_eq!(revision(&app).0, before_delete.0 + 1);
    assert!(group_rows(&app).is_empty());
    assert_eq!(undo_depth(&app), 2);
    assert_eq!(
        drawing_of(&app).entities().count(),
        2,
        "deleting a group never deletes an entity"
    );
}

#[test]
fn group_duplicate_and_unknown_are_named_refusals_without_writing() {
    let (mut app, mut session) = group_app();
    select(&mut session, &[GROUP_LINE_A]);
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();
    let before = revision(&app);
    let depth = undo_depth(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "WALLS".into(),
        ))),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("group.duplicate"), "{error}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Delete(
            "missing".into(),
        ))),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("group.unknown"), "{error}");
    assert_eq!(revision(&app), before, "a refusal writes nothing");
    assert_eq!(undo_depth(&app), depth, "a refusal records no history");
    assert_eq!(group_rows(&app).len(), 1);
}

#[test]
fn group_create_needs_a_selection() {
    let (mut app, mut session) = group_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "empty".into(),
        ))),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("group.selection_required"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert!(group_rows(&app).is_empty());
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn ungroup_removes_only_the_name_as_one_undo_step() {
    let (mut app, mut session) = group_app();
    select(&mut session, &[GROUP_LINE_A]);
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();
    let entities_before = drawing_of(&app).entities().count();
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::Ungroup,
        annotation(AnnotationBatchPayload::Ungroup("walls".into())),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "ungroup.removed");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert!(group_rows(&app).is_empty());
    assert_eq!(drawing_of(&app).entities().count(), entities_before);
    assert_eq!(undo_depth(&app), 2);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::Ungroup,
        annotation(AnnotationBatchPayload::Ungroup("walls".into())),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("ungroup.unknown"), "{error}");
}

#[test]
fn groupedit_add_remove_and_rename_are_persisted_writes() {
    let (mut app, mut session) = group_app();
    select(&mut session, &[GROUP_LINE_A]);
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();

    // Empty selection: typed entity ids are the input.
    session.selection.clear();
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(GroupEditCommand::Add {
            name: "walls".into(),
            entities: vec![GROUP_LINE_B],
        })),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "groupedit.added");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(
        group_rows(&app),
        vec![("walls".to_string(), vec![GROUP_LINE_A, GROUP_LINE_B])]
    );

    let out = execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(
            GroupEditCommand::Remove {
                name: "walls".into(),
                entities: vec![GROUP_LINE_A],
            },
        )),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "groupedit.removed");
    assert_eq!(
        group_rows(&app),
        vec![("walls".to_string(), vec![GROUP_LINE_B])]
    );

    let before_rename = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(
            GroupEditCommand::Rename {
                name: "walls".into(),
                new_name: "WALLS".into(),
            },
        )),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "groupedit.renamed");
    assert_eq!(revision(&app).0, before_rename.0 + 1);
    assert_eq!(
        group_rows(&app),
        vec![("WALLS".to_string(), vec![GROUP_LINE_B])]
    );
    assert_eq!(undo_depth(&app), 4, "create + add + remove + rename");
}

#[test]
fn groupedit_unknown_and_stale_ids_are_refused_before_mutation() {
    let (mut app, mut session) = group_app();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(GroupEditCommand::Add {
            name: "missing".into(),
            entities: vec![GROUP_LINE_A],
        })),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("groupedit.unknown"), "{error}");

    select(&mut session, &[GROUP_LINE_A]);
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();
    let before = revision(&app);
    let depth = undo_depth(&app);
    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(GroupEditCommand::Add {
            name: "walls".into(),
            entities: vec![9999],
        })),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("groupedit.stale_ref"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert_eq!(
        group_rows(&app),
        vec![("walls".to_string(), vec![GROUP_LINE_A])]
    );
}

#[test]
fn group_create_delete_rename_undo_and_redo_round_trip() {
    let (mut app, mut session) = group_app();
    select(&mut session, &[GROUP_LINE_A, GROUP_LINE_B]);
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Create(
            "walls".into(),
        ))),
    )
    .unwrap();
    assert_eq!(group_rows(&app).len(), 1);

    // Undo the create: the group table is empty again, one revision back.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(group_rows(&app).is_empty());
    // Redo restores the same row (same sequence and members).
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        group_rows(&app),
        vec![("walls".to_string(), vec![GROUP_LINE_A, GROUP_LINE_B])]
    );

    // Rename, undo, redo: the rename is one reversible step.
    session.selection.clear();
    execute(
        &mut app,
        &mut session,
        CommandId::GroupEdit,
        annotation(AnnotationBatchPayload::GroupEdit(
            GroupEditCommand::Rename {
                name: "walls".into(),
                new_name: "exterior".into(),
            },
        )),
    )
    .unwrap();
    assert_eq!(group_rows(&app)[0].0, "exterior");
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(group_rows(&app)[0].0, "walls");
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(group_rows(&app)[0].0, "exterior");

    // Delete, undo: members and name come back; entities were never touched.
    let entities_before = drawing_of(&app).entities().count();
    execute(
        &mut app,
        &mut session,
        CommandId::Group,
        annotation(AnnotationBatchPayload::Group(GroupCommand::Delete(
            "exterior".into(),
        ))),
    )
    .unwrap();
    assert!(group_rows(&app).is_empty());
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        group_rows(&app),
        vec![("exterior".to_string(), vec![GROUP_LINE_A, GROUP_LINE_B])]
    );
    assert_eq!(drawing_of(&app).entities().count(), entities_before);
    assert_eq!(undo_depth(&app), 2, "create + rename (delete undone)");
}
