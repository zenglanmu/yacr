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

pub(super) const DIM_A: u128 = 801;
pub(super) const DIM_B: u128 = 802;
const LINE_A: u128 = 803;
const LINE_B: u128 = 804;
const CIRCLE: u128 = 805;
const ARC: u128 = 806;
const POINT: u128 = 807;
const DIMROTATED_ID: u128 = 808;
const DIM_RADIAL: u128 = 809;
const DIM_ORDINATE: u128 = 810;
const DIM_ANGULAR: u128 = 811;
const DIM_ANGULAR_PICTURE: u128 = 812;

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

/// An angular dimension whose display is an imported **picture** (no arc child),
/// the one kind `DIMTEDIT` justification cannot anchor on.
fn picture_angular() -> SemanticGeometry {
    SemanticGeometry::Dimension(Box::new(
        DimensionGeometry::from_picture(
            DimensionDefinition {
                kind: DimensionKind::Angular3Pt,
                points: DimensionPoints::Angular3Pt {
                    vertex: point(800.0, 0.0),
                    first: point(810.0, 0.0),
                    second: point(800.0, 10.0),
                    arc_point: point(805.0, 5.0),
                },
                text_middle: point(0.0, 0.0),
                text_rotation: 0.0,
                normal: p3(0.0, 0.0, 1.0),
            },
            "Standard".to_string(),
            standard_style(),
            None,
            vec![SemanticGeometry::Line {
                start: point(800.0, 0.0),
                end: point(810.0, 0.0),
            }],
        )
        .expect("the picture dimension measures"),
    ))
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

pub(super) fn dimension_edit_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_dim_style(cad_db::DimStyle::standard_metric(DimStyleId(0)))
        .unwrap();
    // A second TEXTSTYLE row, so `DIMOVERRIDE DIMTXSTY` can be pinned against a
    // real table entry (the text child's style/font follow it).
    b.insert_style(cad_db::Style {
        id: StyleId(4),
        name: "Note".into(),
        resource_keys: vec!["arial.ttf".into()],
    })
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
    b.insert_entity(ent(
        DIM_ANGULAR,
        synthesized(
            DimensionKind::Angular3Pt,
            DimensionPoints::Angular3Pt {
                vertex: point(700.0, 0.0),
                first: point(710.0, 0.0),
                second: point(700.0, 10.0),
                arc_point: point(705.0, 5.0),
            },
        ),
        10,
    ))
    .unwrap();
    b.insert_entity(ent(DIM_ANGULAR_PICTURE, picture_angular(), 11))
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

pub(super) fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().copied().map(ref_of));
}

pub(super) fn stored(app: &Application, id: u128) -> DimensionGeometry {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .expect("the dimension entity");
    match &entity.geometry {
        SemanticGeometry::Dimension(dimension) => (**dimension).clone(),
        other => panic!("expected a dimension, got {other:?}"),
    }
}

pub(super) fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

pub(super) fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn style_payload(command: DimStyleCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Style(command)))
}

pub(super) fn text_edit_payload(command: DimTextEditCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::TextEdit(command)))
}

pub(super) fn edit_payload(command: DimEditCommand) -> CommandPayload {
    CommandPayload::Dimension(Box::new(DimensionBatchPayload::Edit(command)))
}

#[test]
fn dimstyle_reports_are_read_only_and_set_is_a_session_write() {
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

    // `SET` is now a real session write (creation resolves the row); it still
    // changes no revision and records no undo step.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::DimStyle,
        style_payload(DimStyleCommand::Set("Standard".into())),
    )
    .expect("DIMSTYLE SET resolves the stored row");
    assert!(outcome.diagnostics.iter().any(|d| d.code == "dimstyle.set"));
    assert_eq!(session.current_dim_style_name(), "Standard");
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
fn dimtedit_justifies_radius_aligned_angular_and_ordinate_along_their_dimension_lines() {
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

    // Ordinate: the leader is the dimension line, so LEFT is the feature end,
    // RIGHT the leader end and CENTER the midpoint between them.
    select(&mut session, &[DIM_ORDINATE]);
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Left),
    )
    .unwrap();
    assert_eq!(
        stored(&app, DIM_ORDINATE).definition().text_middle,
        point(600.0, 0.0)
    );
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Right),
    )
    .unwrap();
    assert_eq!(
        stored(&app, DIM_ORDINATE).definition().text_middle,
        point(600.0, 5.0)
    );
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Center),
    )
    .unwrap();
    assert_eq!(
        stored(&app, DIM_ORDINATE).definition().text_middle,
        point(600.0, 2.5)
    );

    // Angular: the dimension arc is the dimension line. The fixture arc runs
    // from 0 to 90 degrees about (700, 0) at radius sqrt(50).
    select(&mut session, &[DIM_ANGULAR]);
    let radius = 50.0_f64.sqrt();
    for (option, expected) in [
        (DimTextEditCommand::Left, point(700.0 + radius, 0.0)),
        (DimTextEditCommand::Right, point(700.0, radius)),
        (
            DimTextEditCommand::Center,
            point(
                700.0 + radius * std::f64::consts::FRAC_1_SQRT_2,
                radius * std::f64::consts::FRAC_1_SQRT_2,
            ),
        ),
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::DimTextEdit,
            text_edit_payload(option),
        )
        .unwrap();
        let middle = stored(&app, DIM_ANGULAR).definition().text_middle;
        assert!(
            approx(middle.x, expected.x) && approx(middle.y, expected.y),
            "{option:?}: {middle:?} != {expected:?}"
        );
    }
}

#[test]
fn dimtedit_refuses_oblique_and_a_picture_arc_by_name() {
    let (mut app, mut session) = dimension_edit_app();
    let before = revision(&app);
    let undo_before = undo_depth(&app);

    // OBLIQUE needs a glyph-level slant angle this build's text model has no
    // field for: a named capability refusal that writes nothing.
    select(&mut session, &[DIM_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Oblique { degrees: 15.0 }),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimtedit.oblique_unsupported"), "{text}");
    assert!(text.contains("oblique"), "{text}");

    // An imported picture-branch angular dimension has no arc to justify along.
    select(&mut session, &[DIM_ANGULAR_PICTURE]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Left),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimtedit.justify_unsupported"), "{text}");
    assert!(text.contains("angular dimension"), "{text}");
    assert!(text.contains("no dimension arc"), "{text}");

    assert_eq!(revision(&app), before);
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
fn dimedit_homes_rotates_and_refuses_oblique_by_name() {
    let (mut app, mut session) = dimension_edit_app();
    select(&mut session, &[DIM_A]);
    let before = revision(&app);

    // ROTATE writes group 53 in radians and keeps the override untouched.
    execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Text("<> mm".into())),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Rotate { degrees: 30.0 }),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert!(approx(
        dimension.definition().text_rotation,
        30.0_f64.to_radians()
    ));
    assert_eq!(dimension.text_override(), Some("<> mm"));
    assert_eq!(undo_depth(&app), 2);

    // HOME restores only the derived position.
    execute(
        &mut app,
        &mut session,
        CommandId::DimTextEdit,
        text_edit_payload(DimTextEditCommand::Move { x: 3.0, y: 4.0 }),
    )
    .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Home),
    )
    .unwrap();
    let dimension = stored(&app, DIM_A);
    assert_eq!(dimension.definition().text_middle, point(0.0, 0.0));
    assert!(approx(
        dimension.definition().text_rotation,
        30.0_f64.to_radians()
    ));
    assert_eq!(dimension.text_override(), Some("<> mm"));
    let after_home = revision(&app).0;

    // OBLIQUE names the missing extension-line oblique field and writes nothing.
    let undo_before = undo_depth(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Oblique { degrees: 15.0 }),
    )
    .unwrap_err();
    let text = format!("{error}");
    assert!(text.contains("dimedit.oblique_unsupported"), "{text}");
    assert!(text.contains("oblique"), "{text}");

    // A non-finite rotation is a named refusal too.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimEdit,
        edit_payload(DimEditCommand::Rotate {
            degrees: f64::INFINITY,
        }),
    )
    .unwrap_err();
    assert!(
        format!("{error}").contains("dimedit.bad_rotation"),
        "{error}"
    );

    assert_eq!(revision(&app).0, after_home);
    assert_eq!(undo_depth(&app), undo_before);
    assert!(revision(&app).0 > before.0);
}
