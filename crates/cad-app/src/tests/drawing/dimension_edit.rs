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
const DIM_RADIAL: u128 = 809;
const DIM_ORDINATE: u128 = 810;

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
    b.insert_entity(ent(
        DIM_RADIAL,
        synthesized(
            DimensionKind::Radius,
            DimensionPoints::Radial {
                center: point(500.0, 0.0),
                point_on_arc: point(505.0, 0.0),
                leader_length: 0.0,
            },
        ),
        8,
    ))
    .unwrap();
    b.insert_entity(ent(
        DIM_ORDINATE,
        synthesized(
            DimensionKind::Ordinate { x_ordinate: true },
            DimensionPoints::Ordinate {
                feature: point(600.0, 0.0),
                leader_end: point(600.0, 5.0),
                base: point(0.0, 0.0),
            },
        ),
        9,
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
fn dimtedit_moves_justifies_homes_and_angles_text_in_one_undo_each() {
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

    // ANGLE keeps the text centre where MOVE put it and stores radians about the
    // UCS X axis; 0 is the "derive" sentinel.
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Angle { degrees: 30.0 }),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.definition().text_middle, point(3.0, 4.0));
    assert!(approx(
        dimension.definition().text_rotation,
        std::f64::consts::PI / 6.0
    ));
    assert_eq!(undo_depth(&app), 2);

    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Angle { degrees: 0.0 }),
    )
    .unwrap();
    assert!(approx(stored(&app, DIM_A).definition().text_rotation, 0.0));
    assert_eq!(undo_depth(&app), 3);

    // LEFT/RIGHT/CENTER place the text anchor at the low/high/mid projection of
    // the two extension-line origins on the dimension line (y = -5 here).
    for (option, expected_x) in [
        (DimTextEditCommand::Left, 0.0),
        (DimTextEditCommand::Right, 10.0),
        (DimTextEditCommand::Center, 5.0),
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::DimTextEdit,
            text_edit_payload(option),
        )
        .unwrap();
        let middle = stored(&app, DIM_A).definition().text_middle;
        assert!(approx(middle.x, expected_x), "{option:?}: {middle:?}");
        assert!(approx(middle.y, -5.0), "{option:?}: {middle:?}");
    }
    let after_justify = undo_depth(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Center),
    )
    .unwrap();
    assert_eq!(out.diagnostics[0].code, "dimtedit.unchanged");
    assert_eq!(undo_depth(&app), after_justify);

    // HOME restores the derived position only; the rotation stays as set.
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Angle { degrees: 15.0 }),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Home),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.definition().text_middle, point(0.0, 0.0));
    assert!(approx(
        dimension.definition().text_rotation,
        15.0f64.to_radians()
    ));
}

#[test]
fn dimtedit_justifies_glyph_alignment_and_undo_redo_restore_it() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    let text_align = |app: &Application, id: u128| -> TextAlignH {
        stored(app, id)
            .display()
            .iter()
            .find_map(|child| match child {
                SemanticGeometry::Text { h_align, .. } => Some(*h_align),
                _ => None,
            })
            .expect("the dimension has a text child")
    };
    // Synthesis centres the glyphs and the fixture stores the default anchor.
    assert_eq!(stored(&app, DIM_A).text_h_align(), TextAlignH::Center);
    assert_eq!(text_align(&app, DIM_A), TextAlignH::Center);
    assert_eq!(undo_depth(&app), 0);

    // LEFT/RIGHT/CENTER set both the anchor and the glyph alignment.
    for (option, expected_align, expected_x) in [
        (DimTextEditCommand::Left, TextAlignH::Left, 0.0),
        (DimTextEditCommand::Right, TextAlignH::Right, 10.0),
        (DimTextEditCommand::Center, TextAlignH::Center, 5.0),
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::DimTextEdit,
            text_edit_payload(option),
        )
        .unwrap();
        let dimension = stored(&app, DIM_A);
        assert_eq!(dimension.text_h_align(), expected_align, "{option:?}");
        assert_eq!(text_align(&app, DIM_A), expected_align, "{option:?}");
        let middle = dimension.definition().text_middle;
        assert!(approx(middle.x, expected_x), "{option:?}: {middle:?}");
        assert!(approx(middle.y, -5.0), "{option:?}: {middle:?}");
    }

    // Move/Home/Angle preserve the current alignment (here LEFT).
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Left),
    )
    .unwrap();
    for (option, expected_middle) in [
        (DimTextEditCommand::Move { x: 3.0, y: 4.0 }, point(3.0, 4.0)),
        (DimTextEditCommand::Home, point(0.0, 0.0)),
        (DimTextEditCommand::Angle { degrees: 15.0 }, point(0.0, 0.0)),
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::DimTextEdit,
            text_edit_payload(option),
        )
        .unwrap();
        let dimension = stored(&app, DIM_A);
        assert_eq!(dimension.text_h_align(), TextAlignH::Left, "{option:?}");
        assert_eq!(text_align(&app, DIM_A), TextAlignH::Left, "{option:?}");
        assert_eq!(
            dimension.definition().text_middle,
            expected_middle,
            "{option:?}"
        );
    }

    // One undo step per justify; UNDO restores the previous anchor *and*
    // alignment, REDO reapplies both.
    let before_undo = undo_depth(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Center),
    )
    .unwrap();
    assert_eq!(undo_depth(&app), before_undo + 1);
    assert_eq!(stored(&app, DIM_A).text_h_align(), TextAlignH::Center);
    assert!(approx(stored(&app, DIM_A).definition().text_middle.x, 5.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let undone = stored(&app, DIM_A);
    assert_eq!(undone.text_h_align(), TextAlignH::Left);
    assert_eq!(text_align(&app, DIM_A), TextAlignH::Left);
    assert_eq!(undone.definition().text_middle, point(0.0, 0.0));
    assert_eq!(undo_depth(&app), before_undo);

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    let redone = stored(&app, DIM_A);
    assert_eq!(redone.text_h_align(), TextAlignH::Center);
    assert_eq!(text_align(&app, DIM_A), TextAlignH::Center);
    assert!(approx(redone.definition().text_middle.x, 5.0));
    assert_eq!(undo_depth(&app), before_undo + 1);
}

#[test]
fn dimtedit_justifies_radius_and_aligned_but_refuses_ordinate_by_name() {
    let (mut app, mut session) = dimension_edit_app();
    // Radius: LEFT is the centre-side end of the leader, RIGHT the arc-side.
    select(&mut session, &[DIM_RADIAL]);
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Left),
    )
    .unwrap();
    assert_eq!(
        stored(&app, DIM_RADIAL).definition().text_middle,
        point(500.0, 0.0)
    );
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Right),
    )
    .unwrap();
    assert_eq!(
        stored(&app, DIM_RADIAL).definition().text_middle,
        point(505.0, 0.0)
    );

    // Aligned is part of the linear family and is admitted.
    select(&mut session, &[DIMROTATED_ID]);
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Right),
    )
    .unwrap();
    let middle = stored(&app, DIMROTATED_ID).definition().text_middle;
    assert!(
        approx(middle.x, 42.5) && approx(middle.y, 17.5),
        "{middle:?}"
    );

    // Ordinate has no single dimension line in this model: named refusal.
    select(&mut session, &[DIM_ORDINATE]);
    let revision_before = revision(&app);
    let undo_before = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Left),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimtedit.justify_unsupported"),
        "{error}"
    );
    assert!(format!("{error}").contains("ordinate dimension"), "{error}");
    assert_eq!(revision(&app), revision_before);
    assert_eq!(undo_depth(&app), undo_before);
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
            text_edit_payload(DimTextEditCommand::Home),
        )
        .unwrap_err();
        assert!(format!("{error}").contains("dimtedit.selection"), "{error}");
    }

    select(&mut session, &[LINE_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Home),
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
            text_edit_payload(DimTextEditCommand::Home),
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
