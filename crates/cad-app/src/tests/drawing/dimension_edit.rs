//! Contract tests for the 2026-11-07 dimension-editing batch:
//! `DIMSTYLE`/`-DIMSTYLE`, `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`, `DIMREGEN`,
//! `DIMDISASSOCIATE`, `QDIM`, `DIMJOGLINE` and `DIMBREAK`.
//!
//! Pinned here:
//!
//! * the read-only reports and no-op diagnostics write nothing;
//! * `DIMTEDIT`/`DIMEDIT`/`DIMOVERRIDE` change the entity, not the DIMSTYLE
//!   table, and record exactly one transaction / one undo step per run;
//! * every refusal is named, changes nothing and records no history;
//! * `QDIM` creates aligned dimensions from LINEs in one transaction and refuses
//!   arcs/circles/points by name;
//! * `DIMJOGLINE`/`DIMBREAK` are capability refusals that never touch the
//!   dimension;
//! * Viewer mode refuses the four work-only edits.

use super::*;

const DIM_A: u128 = 801;
const DIM_B: u128 = 802;
const LINE_A: u128 = 803;
const LINE_B: u128 = 804;
const CIRCLE: u128 = 805;
const ARC: u128 = 806;
const POINT: u128 = 807;
const DIMROTATED_ID: u128 = 808;

fn standard_style() -> DimensionStyleValues {
    cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    )
}

fn synthesized(kind: DimensionKind, points: DimensionPoints) -> SemanticGeometry {
    SemanticGeometry::Dimension(Box::new(
        DimensionGeometry::synthesized(
            DimensionDefinition {
                kind,
                points,
                text_middle: point(0.0, 0.0),
                text_rotation: 0.0,
                normal: p3(0.0, 0.0, 1.0),
            },
            "Standard".to_string(),
            standard_style(),
            None,
        )
        .expect("the fixture dimension synthesizes"),
    ))
}

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

fn dimension_edit_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_dim_style(cad_db::DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    b.insert_entity(ent(
        DIM_A,
        synthesized(
            DimensionKind::Linear { rotation: 0.0 },
            DimensionPoints::Linear {
                first: point(0.0, 0.0),
                second: point(10.0, 0.0),
                dimension_line: point(5.0, -5.0),
            },
        ),
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        DIM_B,
        synthesized(
            DimensionKind::Linear { rotation: 0.0 },
            DimensionPoints::Linear {
                first: point(20.0, 0.0),
                second: point(30.0, 0.0),
                dimension_line: point(25.0, -5.0),
            },
        ),
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        DIMROTATED_ID,
        synthesized(
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first: point(40.0, 0.0),
                second: point(50.0, 10.0),
                dimension_line: point(45.0, 20.0),
            },
        ),
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        LINE_A,
        SemanticGeometry::Line {
            start: point(0.0, 100.0),
            end: point(10.0, 100.0),
        },
        3,
    ))
    .unwrap();
    b.insert_entity(ent(
        LINE_B,
        SemanticGeometry::Line {
            start: point(0.0, 200.0),
            end: point(0.0, 210.0),
        },
        4,
    ))
    .unwrap();
    b.insert_entity(ent(
        CIRCLE,
        SemanticGeometry::Circle {
            center: point(100.0, 100.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        5,
    ))
    .unwrap();
    b.insert_entity(ent(
        ARC,
        SemanticGeometry::Arc {
            center: point(200.0, 100.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        6,
    ))
    .unwrap();
    b.insert_entity(ent(POINT, SemanticGeometry::Point(point(300.0, 100.0)), 7))
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

fn stored(app: &Application, id: u128) -> DimensionGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the dimension entity");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (**dimension).clone(),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn style_payload(command: DimStyleCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Style(command)))
}

fn text_edit_payload(command: DimTextEditCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::TextEdit(command)))
}

fn edit_payload(command: DimEditCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Edit(command)))
}

fn override_payload(variable: &str, value: &str) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Override(
        DimOverrideCommand {
            variable: variable.to_string(),
            value: value.to_string(),
        },
    )))
}

fn jog_payload(command: DimJogLineCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::JogLine(command)))
}

#[test]
fn dimstyle_reports_are_read_only_and_set_is_a_named_capability_refusal() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        style_payload(DimStyleCommand::List),
    )
    .unwrap();
    assert!(out.changes.is_none());
    assert!(out.objects.is_empty());
    assert!(
        out.diagnostics.iter().any(|d| d.code == "dimstyle.entry"),
        "LIST reports every stored style"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        style_payload(DimStyleCommand::Current),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimstyle.current");
    assert!(out.diagnostics[0].message.contains("Standard"));
    assert!(out.changes.is_none());

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        style_payload(DimStyleCommand::Set("Standard".into())),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimstyle.set_unavailable"),
        "{error}"
    );
    assert_eq!(revision(&app), before);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        style_payload(DimStyleCommand::Set("Missing".into())),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("dimstyle.unknown"), "{error}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimstyle.needs_option"),
        "{error}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn dimtedit_moves_rotates_and_resets_text_in_one_undo_each() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Move { x: 3.0, y: 4.0 }),
    )
    .unwrap();
    assert_eq!(out.objects, vec![ObjectId(DIM_A)]);
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.definition().text_middle, point(3.0, 4.0));
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Rotate { degrees: 30.0 }),
    );
    assert!(error.is_ok());
    let dimension = stored(&app, DIM_A);
    assert!(approx(
        dimension.definition().text_rotation,
        std::f64::consts::PI / 6.0
    ));
    assert_eq!(undo_depth(&app), 2);

    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Reset),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.definition().text_middle, point(0.0, 0.0));
    assert!(approx(dimension.definition().text_rotation, 0.0));
    assert_eq!(undo_depth(&app), 3);

    // Resetting an already-reset dimension is a named no-op: no transaction.
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Reset),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimtedit.unchanged");
    assert!(out.changes.is_none());
    assert_eq!(undo_depth(&app), 3);
}

#[test]
fn dimtedit_refuses_the_wrong_selection_count_and_non_dimensions() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);

    for ids in [&[][..], &[DIM_A, DIM_B][..]] {
        select(&mut session, ids);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::DimTextEdit,
            text_edit_payload(DimTextEditCommand::Reset),
        )
        .unwrap_err();
        assert!(format!("{error}").contains("dimtedit.selection"), "{error}");
    }

    select(&mut session, &[LINE_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Reset),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimtedit.not_dimension"),
        "{error}"
    );

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn dimedit_sets_and_drops_the_text_override_in_one_undo_each() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A, DIM_B]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Text("<> mm".into())),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert_eq!(
        stored(&app, DIM_A).text_override(),
        Some("<> mm"),
        "group 1 stores the template verbatim"
    );
    assert_eq!(
        stored(&app, DIM_A).measurement(),
        10.0,
        "the measurement is unchanged"
    );
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Reset),
    )
    .unwrap();
    assert_eq!(stored(&app, DIM_A).text_override(), None);
    assert_eq!(undo_depth(&app), 2);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Reset),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimedit.unchanged");
    assert!(out.changes.is_none());
    assert_eq!(undo_depth(&app), 2);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Text("   ".into())),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("dimedit.bad_text"), "{error}");
    assert_eq!(revision(&app).0, before.0 + 2);
}

#[test]
fn dimoverride_applies_the_closed_snapshot_set_and_refuses_the_rest() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A, DIM_B]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXT", "5"),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert!(approx(stored(&app, DIM_A).style().text_height, 5.0));
    assert!(approx(stored(&app, DIM_B).style().text_height, 5.0));
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMDEC", "3"),
    )
    .unwrap();
    assert_eq!(stored(&app, DIM_A).style().decimals, 3);
    assert_eq!(undo_depth(&app), 2);

    // The DIMSTYLE table itself is untouched: the change is the entity snapshot.
    let table_height = app.workspace.documents[&DocumentId(1)]
        .drawing
        .dim_style_by_name("Standard")
        .expect("Standard")
        .text_height;
    assert!(approx(table_height, 2.5));

    let checks = [
        ("DIMFOO", "1", "dimoverride.unknown_variable"),
        ("DIMCLRD", "1", "dimoverride.unsupported_variable"),
        ("DIMTXT", "0", "dimoverride.bad_value"),
        ("DIMLUNIT", "4", "dimoverride.unsupported_value"),
        ("DIMZIN", "3", "dimoverride.unsupported_value"),
    ];
    for (variable, value, code) in checks {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::DimOverride,
            override_payload(variable, value),
        )
        .unwrap_err();
        assert!(format!("{error}").contains(code), "{variable}: {error}");
    }
    assert_eq!(revision(&app).0, before.0 + 2);
    assert_eq!(undo_depth(&app), 2);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimOverride,
        override_payload("DIMTXT", "5"),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimoverride.unchanged");
    assert!(out.changes.is_none());
    assert_eq!(undo_depth(&app), 2);
}

#[test]
fn dimregen_and_dimdisassociate_are_named_noops() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimRegen,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimregen.non_associative");
    assert!(out.changes.is_none());
    assert!(out.objects.is_empty());

    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimDisassociate,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        out.diagnostics[0].code,
        "dimdisassociate.already_non_associative"
    );
    assert!(out.changes.is_none());

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimRegen,
        CommandPayload::Points(Vec::new()),
    )
    .unwrap_err();
    assert!(format!("{error}").contains("dimregen.payload"), "{error}");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimDisassociate,
        CommandPayload::Points(Vec::new()),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimdisassociate.payload"),
        "{error}"
    );
}

#[test]
fn qdim_creates_aligned_dimensions_from_lines_in_one_transaction() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[LINE_A, LINE_B]);
    let before = revision(&app);

    let out = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);

    let first = stored(&app, out.objects[0].0);
    match first.kind() {
        DimensionKind::Aligned => {}
        other => panic!("QDIM must create aligned dimensions, got {other:?}"),
    }
    assert!(approx(first.measurement(), 10.0));
    let (first_point, second_point, dimension_line) = match &first.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => (*first, *second, *dimension_line),
        other => panic!("expected Linear points, got {other:?}"),
    };
    assert_eq!(first_point, point(0.0, 100.0));
    assert_eq!(second_point, point(10.0, 100.0));
    // Left normal of +X is +Y; the Standard DIMDLI places the line 3.75 above.
    assert_eq!(dimension_line, point(5.0, 103.75));

    let second = stored(&app, out.objects[1].0);
    assert!(approx(second.measurement(), 10.0));
}

#[test]
fn qdim_refuses_non_line_kinds_by_name() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);
    let checks = [
        (CIRCLE, "qdim.circle_unsupported"),
        (ARC, "qdim.arc_unsupported"),
        (POINT, "qdim.point_unsupported"),
        (DIM_A, "qdim.unsupported_kind"),
    ];
    for (id, code) in checks {
        select(&mut session, &[id]);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::QDim,
            CommandPayload::None,
        )
        .unwrap_err();
        assert!(format!("{error}").contains(code), "{id}: {error}");
    }
    select(&mut session, &[]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(format!("{error}").contains("qdim.selection"), "{error}");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn dimjogline_and_dimbreak_name_the_missing_domain_field() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    let before = revision(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimJogLine,
        jog_payload(DimJogLineCommand::Add { x: 1.0, y: 2.0 }),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimjogline.unsupported"), "{text}");
    assert!(text.contains("no jog field"), "{text}");

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimJogLine,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimjogline.needs_option"),
        "{error}"
    );

    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimBreak,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimbreak.unsupported"),
        "{error}"
    );

    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    // The dimension is still exactly where it was: DIMBREAK never trims it.
    let dimension = stored(&app, DIM_A);
    match &dimension.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => {
            assert_eq!(*first, point(0.0, 0.0));
            assert_eq!(*second, point(10.0, 0.0));
            assert_eq!(*dimension_line, point(5.0, -5.0));
        }
        other => panic!("expected Linear points, got {other:?}"),
    }
}

#[test]
fn viewer_mode_refuses_the_work_only_dimension_edits() {
    let (mut app, mut session) = dimension_edit_app();
    session.switch_mode(AppMode::Viewer).expect("Viewer mode");
    select(&mut session, &[DIM_A]);

    for (id, payload) in [
        (
            CommandId::DimTextEdit,
            text_edit_payload(DimTextEditCommand::Reset),
        ),
        (CommandId::DimEdit, edit_payload(DimEditCommand::Reset)),
        (CommandId::DimOverride, override_payload("DIMTXT", "5")),
    ] {
        let error = execute(&mut app, &mut session, id, payload).unwrap_err();
        assert!(
            matches!(error, CadError::PermissionDenied),
            "{id:?}: {error:?}"
        );
    }
    let error = execute(
        &mut app,
        &mut session,
        CommandId::QDim,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
}
