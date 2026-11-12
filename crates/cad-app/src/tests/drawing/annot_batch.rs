//! Contract tests for the 2026-11-07 annotation/group batch:
//! `GROUP`/`-GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`, `TEXTFIT`,
//! `HATCHSETBOUNDARY`, `HATCHSETORIGIN` and `XDLIST`.
//!
//! Pinned here:
//!
//! * the group store is session-only: no revision bump, no history and no entity
//!   row changes;
//! * create/list/delete/duplicate/unknown, UNGROUP and every GROUPEDIT arm;
//! * QLEADER/LEADER create exactly one MULTILEADER in one undo step;
//! * TEXTFIT changes only the horizontal width factor under the declared estimate and
//!   refuses an empty selection, a non-text and a bad width;
//! * HATCHSETBOUNDARY rebuilds the loops in one transaction or refuses by name;
//! * HATCHSETORIGIN names the missing origin field and writes nothing;
//! * XDLIST reports `xdlist.not_retained` and never invents a record.

use super::*;

const GROUP_LINE_A: u128 = 901;
const GROUP_LINE_B: u128 = 902;
const GROUP_TEXT: u128 = 903;
const GROUP_HATCH: u128 = 904;
const GROUP_CIRCLE: u128 = 905;
const GROUP_RING: u128 = 906;
const GROUP_OPEN: u128 = 907;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

fn text(id: u128, text: &str, height: f64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Text {
            text: text.to_string(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
            width_factor: 1.0,
        },
        0,
    )
}

fn square_ring(min: f64, max: f64) -> Vec<[f64; 2]> {
    vec![[min, min], [max, min], [max, max], [min, max]]
}

fn hatch(square: f64) -> SemanticGeometry {
    HatchGeometry::new(
        vec![HatchLoop::new(
            square_ring(0.0, square),
            vec![0.0; 4],
            HatchLoopFlags(HatchLoopFlags::POLYLINE),
        )],
        HatchPlane::new(p3(0.0, 0.0, 1.0), 0.0),
        HatchStyle::Normal,
        HatchFill::Solid,
        Vec::new(),
        None,
        HatchPatternType::Predefined,
    )
    .map(|hatch| SemanticGeometry::Hatch(Box::new(hatch)))
    .expect("fixture hatch")
}

fn annot_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        GROUP_LINE_A,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        GROUP_LINE_B,
        SemanticGeometry::Line {
            start: point(0.0, 10.0),
            end: point(10.0, 10.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(text(GROUP_TEXT, "ABCD", 2.0)).unwrap();
    b.insert_entity(ent(GROUP_HATCH, hatch(10.0), 3)).unwrap();
    b.insert_entity(ent(
        GROUP_CIRCLE,
        SemanticGeometry::Circle {
            center: point(2.0, 2.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 1.0,
        },
        4,
    ))
    .unwrap();
    b.insert_entity(ent(
        GROUP_RING,
        SemanticGeometry::Polyline {
            points: square_ring(6.0, 8.0)
                .into_iter()
                .map(|point| p3(point[0], point[1], 0.0))
                .collect(),
            bulges: Vec::new(),
            closed: true,
        },
        5,
    ))
    .unwrap();
    b.insert_entity(ent(
        GROUP_OPEN,
        SemanticGeometry::Polyline {
            points: vec![p3(20.0, 0.0, 0.0), p3(21.0, 0.0, 0.0), p3(21.0, 1.0, 0.0)],
            bulges: Vec::new(),
            closed: false,
        },
        6,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
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

fn group_names(session: &SessionState) -> Vec<String> {
    session
        .groups
        .groups()
        .map(|group| group.name.clone())
        .collect()
}

#[test]
fn group_create_list_delete_is_session_only() {
    let (mut app, mut session) = annot_app();
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
    assert!(out.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(group_names(&session), vec!["walls"]);
    assert_eq!(session.groups.resolve("walls").unwrap().members.len(), 2);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::Group,
        CommandPayload::None,
    )
    .unwrap();
    assert!(out.diagnostics.iter().any(|d| d.code == "group.entry"));
    assert!(out.diagnostics.iter().any(|d| d.code == "group.list"));
    assert_eq!(revision(&app), before);

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
    assert!(session.groups.is_empty());
    assert_eq!(revision(&app), before);
}

#[test]
fn group_duplicate_and_unknown_are_named_refusals() {
    let (mut app, mut session) = annot_app();
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
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn group_create_needs_a_selection() {
    let (mut app, mut session) = annot_app();
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
    assert!(session.groups.is_empty());
}

#[test]
fn ungroup_removes_only_the_name() {
    let (mut app, mut session) = annot_app();
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
    let entities_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .count();

    let out = execute(
        &mut app,
        &mut session,
        CommandId::Ungroup,
        annotation(AnnotationBatchPayload::Ungroup("walls".into())),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "ungroup.removed");
    assert!(session.groups.is_empty());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entities()
            .count(),
        entities_before
    );
    assert_eq!(undo_depth(&app), 0);

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
fn groupedit_add_remove_and_rename_mutate_only_membership() {
    let (mut app, mut session) = annot_app();
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

    // Empty selection: typed entity ids are the input.
    session.selection.clear();
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
    assert_eq!(session.groups.resolve("walls").unwrap().members.len(), 2);
    assert_eq!(revision(&app), before);

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
    assert_eq!(session.groups.resolve("walls").unwrap().members.len(), 1);

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
    assert_eq!(group_names(&session), vec!["WALLS"]);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn groupedit_unknown_and_stale_ids_are_refused_before_mutation() {
    let (mut app, mut session) = annot_app();
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
    assert_eq!(session.groups.resolve("walls").unwrap().members.len(), 1);
}

fn leader_payload(arrow: Point3, landing: Point3, text: &str) -> CommandPayload {
    annotation(AnnotationBatchPayload::Leader(LeaderCommand {
        arrow,
        landing,
        text: text.to_string(),
    }))
}

fn leader_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entities()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::MultiLeader(_)))
        .count()
}

#[test]
fn qleader_and_leader_create_one_multileader_in_one_undo_step() {
    let (mut app, mut session) = annot_app();
    let arrow = p3(0.0, 0.0, 0.0);
    let landing = p3(10.0, 2.0, 0.0);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::QLeader,
        leader_payload(arrow, landing, "Note"),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1);
    assert_eq!(leader_count(&app), 1);
    assert_eq!(undo_depth(&app), 1);
    let created = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(out.objects[0].0))
        .unwrap();
    assert!(
        matches!(created.geometry, SemanticGeometry::MultiLeader(_)),
        "QLEADER must create a MULTILEADER"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(leader_count(&app), 0);
    assert_eq!(undo_depth(&app), 0);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::Leader,
        leader_payload(arrow, landing, "Second"),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 1);
    assert_eq!(leader_count(&app), 1);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn textfit_changes_only_the_width_factor_under_the_estimate() {
    let (mut app, mut session) = annot_app();
    select(&mut session, &[GROUP_TEXT]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        annotation(AnnotationBatchPayload::TextFit(TextFitCommand {
            target_width: 12.0,
        })),
    )
    .unwrap();
    assert_eq!(out.objects, vec![ObjectId(GROUP_TEXT)]);
    assert_eq!(undo_depth(&app), 1);
    let stored = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(GROUP_TEXT))
        .unwrap();
    let SemanticGeometry::Text {
        height,
        width_factor,
        text,
        position,
        rotation,
        ..
    } = &stored.geometry
    else {
        panic!("expected text");
    };
    assert_eq!(*height, 2.0, "the glyph height is unchanged");
    assert_eq!(*width_factor, 1.5, "12 / (height 2 × 4 characters)");
    assert_eq!(text, "ABCD");
    assert_eq!(*position, point(0.0, 0.0));
    assert_eq!(*rotation, 0.0);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let restored = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(GROUP_TEXT))
        .unwrap();
    let SemanticGeometry::Text {
        height,
        width_factor,
        ..
    } = &restored.geometry
    else {
        panic!("expected text");
    };
    assert_eq!(*height, 2.0);
    assert_eq!(*width_factor, 1.0, "undo restores the stored width factor");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn textfit_refuses_bad_width_empty_selection_and_non_text() {
    let (mut app, mut session) = annot_app();
    select(&mut session, &[GROUP_TEXT]);
    for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::TextFit,
            annotation(AnnotationBatchPayload::TextFit(TextFitCommand {
                target_width: width,
            })),
        )
        .unwrap_err();
        assert!(format!("{error}").contains("textfit.bad_width"), "{error}");
    }
    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        annotation(AnnotationBatchPayload::TextFit(TextFitCommand {
            target_width: 12.0,
        })),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("textfit.selection"), "{error}");

    select(&mut session, &[GROUP_LINE_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::TextFit,
        annotation(AnnotationBatchPayload::TextFit(TextFitCommand {
            target_width: 12.0,
        })),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("textfit.not_text"), "{error}");
    assert_eq!(undo_depth(&app), 0);
}

fn hatch_loops(app: &Application) -> usize {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(GROUP_HATCH))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Hatch(hatch) => hatch.loops().len(),
        other => panic!("expected a hatch, got {other:?}"),
    }
}

#[test]
fn hatch_set_boundary_rebuilds_loops_in_one_transaction() {
    let (mut app, mut session) = annot_app();
    assert_eq!(hatch_loops(&app), 1);
    select(&mut session, &[GROUP_HATCH, GROUP_CIRCLE, GROUP_RING]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetBoundary,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects, vec![ObjectId(GROUP_HATCH)]);
    assert_eq!(hatch_loops(&app), 2, "circle + closed polyline");
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(hatch_loops(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_set_boundary_refuses_open_missing_and_non_planar_inputs() {
    let (mut app, mut session) = annot_app();
    select(&mut session, &[GROUP_HATCH, GROUP_OPEN]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetBoundary,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("hatchsetboundary.not_closed"),
        "{error}"
    );

    select(&mut session, &[GROUP_HATCH, GROUP_LINE_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetBoundary,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("hatchsetboundary.unsupported_boundary"),
        "{error}"
    );

    // A +Z rectangle at elevation 5 is not on the hatch plane (elevation 0).
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(GROUP_HATCH, hatch(10.0), 0)).unwrap();
    b.insert_entity(ent(
        950,
        SemanticGeometry::Polyline {
            points: square_ring(0.0, 4.0)
                .into_iter()
                .map(|point| p3(point[0], point[1], 5.0))
                .collect(),
            bulges: Vec::new(),
            closed: true,
        },
        1,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    select(&mut session, &[GROUP_HATCH, 950]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetBoundary,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("hatchsetboundary.non_planar"),
        "{error}"
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn hatch_set_origin_is_a_named_missing_field_refusal() {
    let (mut app, mut session) = annot_app();
    select(&mut session, &[GROUP_HATCH]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::HatchSetOrigin,
        annotation(AnnotationBatchPayload::HatchSetOrigin(HatchOriginCommand {
            x: 1.0,
            y: 2.0,
        })),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("hatchsetorigin.missing_field"), "{text}");
    assert!(text.contains("HatchGeometry"), "{text}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn xdlist_reports_not_retained_and_never_invents_data() {
    let (mut app, mut session) = annot_app();
    select(&mut session, &[GROUP_LINE_A, GROUP_TEXT]);
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::XdList,
        CommandPayload::None,
    )
    .unwrap();
    assert!(out.changes.is_none());
    assert!(out.objects.is_empty());
    assert_eq!(
        out.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "xdlist.not_retained")
            .count(),
        2
    );
    assert!(out.diagnostics.iter().any(|d| d.code == "xdlist.summary"));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    session.selection.clear();
    let error = execute(
        &mut app,
        &mut session,
        CommandId::XdList,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("xdlist.selection"), "{error}");
}
