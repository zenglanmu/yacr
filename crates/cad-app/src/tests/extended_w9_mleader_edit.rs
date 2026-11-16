//! `MLEADEREDIT`'s per-line overrides and the `MLEADER` creation options.
//!
//! Both are the 2026-11-12 follow-up of the MULTILEADER wave: the two per-line
//! fields DXF carries behind the `LEADER_LINE 93` override word (path type `170`,
//! arrowhead size `40`) are now editable one line at a time, and creation accepts
//! the dogleg length/direction and the text attachment pair instead of forcing the
//! resolved style value.
//!
//! Every real edit is one stored definition per selected leader, committed in one
//! transaction / one undo step. Every refusal is named and writes nothing: the
//! revision and the undo depth are asserted unchanged, exactly like the sibling
//! `tests::extended_w9_mleader` contracts.

use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// One straight-line MULTILEADER with text content: one root, one line with an
/// arrow vertex and a dogleg vertex, and the text at `text_position`.
fn leader(id: u128, text: &str, text_position: Point3, arrow: Point3, dogleg: Point3) -> DbEntity {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 1.0,
            landing_gap: 0.1,
            arrowhead_size: 0.5,
            text_height: 0.2,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
            text_angle_type: MLeaderTextAngle::Horizontal,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: text.into(),
            location: text_position,
            rotation: 0.0,
            height: 0.2,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: text_position,
            dogleg_vector: p3(1.0, 0.0, 0.0),
            dogleg_length: 1.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![arrow, dogleg],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.5,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbMultiLeader".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::MultiLeader(Box::new(geometry)),
        draw_order: id as i64,
    }
}

/// A drawing with layer 0 and the given entities (the builder pins the default
/// `Standard` MLEADERSTYLE row the creation path resolves).
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
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

fn model_entities(app: &Application) -> Vec<DbEntity> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .into_iter()
        .cloned()
        .collect()
}

fn select(session: &mut SessionState, ids: &[u128]) {
    session.selection = SelectionSet::from_refs(ids.iter().map(|id| SelectionRef {
        document: DocumentId(1),
        entity: EntityId(*id),
        instance: InstancePath::default(),
        sub_element: None,
    }));
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn leader_of(entity: &DbEntity) -> &MultiLeaderGeometry {
    match &entity.geometry {
        SemanticGeometry::MultiLeader(leader) => leader,
        other => panic!("expected a MULTILEADER, got {other:?}"),
    }
}

/// The one stored multileader.
fn the_leader(app: &Application) -> MultiLeaderGeometry {
    let entities = model_entities(app);
    let leaders: Vec<&DbEntity> = entities
        .iter()
        .filter(|entity| matches!(entity.geometry, SemanticGeometry::MultiLeader(_)))
        .collect();
    assert_eq!(leaders.len(), 1, "exactly one MULTILEADER");
    leader_of(leaders[0]).clone()
}

fn edit_payload(action: MLeaderEditAction) -> CommandPayload {
    CommandPayload::MLeaderEdit(Box::new(MLeaderEditCommand { action }))
}

/// A fixture leader whose single line runs from (0, 5) to (9, 5) with the text at
/// (10, 5).
fn fixture_leader() -> (Application, SessionState) {
    app_with(vec![leader(
        1,
        "A",
        p3(10.0, 5.0, 0.0),
        p3(0.0, 5.0, 0.0),
        p3(9.0, 5.0, 0.0),
    )])
}

// ---------------------------------------------------------------------------
// MLEADEREDIT PATHTYPE / ARROWSIZE / BYSTYLE
// ---------------------------------------------------------------------------

#[test]
fn mleaderedit_pathtype_stores_the_line_override_in_one_undo_step() {
    let (mut app, mut session) = fixture_leader();
    select(&mut session, &[1]);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MLeaderEdit),
        edit_payload(MLeaderEditAction::SetPathType {
            line_index: 0,
            path_type: MLeaderPathType::Spline,
        }),
    )
    .expect("PATHTYPE is admitted");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mleaderedit.applied" && row.message.contains("PATHTYPE")));

    let leader = the_leader(&app);
    let line = &leader.lines()[0];
    assert_eq!(line.path_type, MLeaderPathType::Spline);
    assert_eq!(
        line.override_flags & MLeaderLine::PATH_TYPE,
        MLeaderLine::PATH_TYPE,
        "the per-line override bit is what makes the value survive a reload"
    );
    // Only the override changed: the vertices, the root and the text did not.
    assert_eq!(line.points.len(), 2);
    assert!(approx(line.points[0].x, 0.0));
    assert!(approx(leader.roots()[0].dogleg_length, 1.0));
    assert_eq!(leader.text().string, "A");
}

#[test]
fn mleaderedit_arrowsize_stores_the_line_override() {
    let (mut app, mut session) = fixture_leader();
    select(&mut session, &[1]);
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MLeaderEdit),
        edit_payload(MLeaderEditAction::SetArrowheadSize {
            line_index: 0,
            size: 2.5,
        }),
    )
    .expect("ARROWSIZE is admitted");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    let leader = the_leader(&app);
    let line = &leader.lines()[0];
    assert!(approx(line.arrowhead_size, 2.5));
    assert_eq!(
        line.override_flags & MLeaderLine::ARROWHEAD_SIZE,
        MLeaderLine::ARROWHEAD_SIZE
    );
    // The entity/style arrowhead size is untouched.
    assert!(approx(leader.style().arrowhead_size, 0.5));
}

#[test]
fn mleaderedit_bystyle_clears_the_override_and_restamps_the_style_value() {
    let (mut app, mut session) = fixture_leader();
    select(&mut session, &[1]);
    for action in [
        MLeaderEditAction::SetPathType {
            line_index: 0,
            path_type: MLeaderPathType::Invisible,
        },
        MLeaderEditAction::SetArrowheadSize {
            line_index: 0,
            size: 3.0,
        },
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::MLeaderEdit),
            edit_payload(action),
        )
        .expect("both overrides are admitted");
    }
    let overridden = the_leader(&app);
    assert_eq!(overridden.lines()[0].override_flags, 17, "bits 1 + 16");
    assert_eq!(overridden.lines()[0].path_type, MLeaderPathType::Invisible);

    // Clearing both bits restores the style values, in one transaction.
    let before = revision(&app);
    for field in [
        MLeaderLineOverrideField::PathType,
        MLeaderLineOverrideField::ArrowheadSize,
    ] {
        execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::MLeaderEdit),
            edit_payload(MLeaderEditAction::ClearOverride {
                line_index: 0,
                field,
            }),
        )
        .expect("BYSTYLE is admitted");
    }
    let cleared = the_leader(&app);
    let line = &cleared.lines()[0];
    assert_eq!(line.override_flags, 0, "both override bits cleared");
    assert_eq!(line.path_type, MLeaderPathType::StraightLineSegments);
    assert!(approx(line.arrowhead_size, 0.5));
    assert_eq!(
        revision(&app),
        Revision(before.0 + 2),
        "one transaction per BYSTYLE line"
    );
}

#[test]
fn mleaderedit_override_refuses_an_out_of_range_index_by_name() {
    for action in [
        MLeaderEditAction::SetPathType {
            line_index: 1,
            path_type: MLeaderPathType::Spline,
        },
        MLeaderEditAction::SetArrowheadSize {
            line_index: 7,
            size: 1.0,
        },
        MLeaderEditAction::ClearOverride {
            line_index: 2,
            field: MLeaderLineOverrideField::PathType,
        },
    ] {
        let (mut app, mut session) = fixture_leader();
        select(&mut session, &[1]);
        let before = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::MLeaderEdit),
            edit_payload(action),
        )
        .expect_err("the index is out of range");
        assert!(error.to_string().contains("mleaderedit.index"), "{error}");
        assert_eq!(revision(&app), before, "the refusal wrote nothing");
        assert_eq!(undo_depth(&app), 0, "and opened no transaction");
        assert_eq!(the_leader(&app).lines()[0].override_flags, 0);
    }
}

#[test]
fn mleaderedit_arrowsize_refuses_an_unusable_size_by_name() {
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let (mut app, mut session) = fixture_leader();
        select(&mut session, &[1]);
        let before = revision(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ExtendedCommandId::MLeaderEdit),
            edit_payload(MLeaderEditAction::SetArrowheadSize {
                line_index: 0,
                size,
            }),
        )
        .expect_err("an unusable size is refused");
        assert!(
            error.to_string().contains("mleaderedit.arrowhead_size"),
            "{size}: {error}"
        );
        assert_eq!(revision(&app), before);
        assert_eq!(undo_depth(&app), 0);
    }
}

#[test]
fn mleaderedit_override_no_ops_when_the_line_already_stores_the_value() {
    let (mut app, mut session) = fixture_leader();
    select(&mut session, &[1]);
    execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MLeaderEdit),
        edit_payload(MLeaderEditAction::SetPathType {
            line_index: 0,
            path_type: MLeaderPathType::Spline,
        }),
    )
    .expect("the first PATHTYPE writes");
    let before = revision(&app);
    let depth = undo_depth(&app);

    // The same value again: a named no-op, no second transaction.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MLeaderEdit),
        edit_payload(MLeaderEditAction::SetPathType {
            line_index: 0,
            path_type: MLeaderPathType::Spline,
        }),
    )
    .expect("the no-op report is admitted");
    assert!(outcome.changes.is_none(), "no transaction for a no-op");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), depth);
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mleaderedit.unchanged"));

    // Clearing an override a line never had is the same declared no-op.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::MLeaderEdit),
        edit_payload(MLeaderEditAction::ClearOverride {
            line_index: 0,
            field: MLeaderLineOverrideField::ArrowheadSize,
        }),
    )
    .expect("clearing an absent override is admitted");
    assert!(outcome.changes.is_none());
    assert_eq!(undo_depth(&app), depth);
}

// ---------------------------------------------------------------------------
// MLEADER creation options
// ---------------------------------------------------------------------------

fn create_payload(
    dogleg_length: Option<f64>,
    dogleg_angle: Option<f64>,
    attachment: Option<MLeaderAttachment>,
) -> CommandPayload {
    CommandPayload::MLeader {
        arrow: p3(0.0, 0.0, 0.0),
        landing: p3(10.0, 2.0, 0.0),
        text: "Opts".into(),
        style_name: None,
        dogleg_length,
        dogleg_angle,
        attachment,
        text_angle: None,
    }
}

#[test]
fn mleader_creation_options_land_in_the_stored_geometry() {
    let (mut app, mut session) = app_with(Vec::new());
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        create_payload(
            Some(2.5),
            Some(std::f64::consts::FRAC_PI_2),
            Some(MLeaderAttachment { point: 1, left: 4 }),
        ),
    )
    .expect("the typed creation options are admitted");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app), Revision(before.0 + 1), "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "mleader.options_applied"));

    let leader = the_leader(&app);
    let root = &leader.roots()[0];
    // The requested dogleg length is the root's own `LEADER 40`, not the style's.
    assert!(approx(root.dogleg_length, 2.5));
    // The requested direction is the root's `LEADER 11` (`+Y` for 90°): the
    // dogleg travels from the dogleg start toward the annotation.
    assert!(approx(root.dogleg_vector.x, 0.0));
    assert!(approx(root.dogleg_vector.y, 1.0));
    assert!(approx(root.dogleg_vector.z, 0.0));
    // The second vertex is `landing - direction * length` = (10, -0.5).
    let start = leader.lines()[0].points[1];
    assert!(approx(start.x, 10.0));
    assert!(approx(start.y, -0.5));
    // The attachment pair is stored on the annotation text.
    assert_eq!(leader.text().attachment_point, 1);
    assert_eq!(leader.text().left_attachment, 4);
    // No per-line override: the root/style values apply, exactly like a leader
    // created from the style alone.
    assert_eq!(leader.lines()[0].override_flags, 0);
}

#[test]
fn mleader_creation_stays_on_the_style_values_when_no_option_is_typed() {
    let (mut app, mut session) = app_with(Vec::new());
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MLeader,
        create_payload(None, None, None),
    )
    .expect("the plain creation path is unchanged");
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|row| row.code != "mleader.options_applied"),
        "no option was typed, so no option report"
    );
    let leader = the_leader(&app);
    // The pinned `standard_metric` row: dogleg 0.36, centred text.
    assert!(approx(leader.roots()[0].dogleg_length, 0.36));
    assert!(approx(leader.roots()[0].dogleg_vector.x, 1.0));
    assert!(approx(leader.roots()[0].dogleg_vector.y, 0.0));
    assert_eq!(
        leader.text().attachment_point,
        cad_domain::CREATED_TEXT_ATTACHMENT_POINT
    );
}

#[test]
fn mleader_creation_refuses_an_unusable_option_by_name() {
    let cases: [(&str, CommandPayload); 5] = [
        (
            "mleader.option_dogleg",
            create_payload(Some(0.0), None, None),
        ),
        (
            "mleader.option_dogleg",
            create_payload(Some(-2.0), None, None),
        ),
        (
            "mleader.option_angle",
            create_payload(None, Some(f64::NAN), None),
        ),
        (
            "mleader.option_attachment",
            create_payload(None, None, Some(MLeaderAttachment { point: 4, left: 4 })),
        ),
        (
            "mleader.option_attachment",
            create_payload(None, None, Some(MLeaderAttachment { point: 2, left: 11 })),
        ),
    ];
    for (code, payload) in cases {
        let (mut app, mut session) = app_with(Vec::new());
        let before = revision(&app);
        let error = execute(&mut app, &mut session, CommandId::MLeader, payload)
            .expect_err("the option is unusable");
        assert!(error.to_string().contains(code), "{code}: {error}");
        assert_eq!(revision(&app), before, "{code}: the refusal wrote nothing");
        assert_eq!(undo_depth(&app), 0, "{code}: and opened no transaction");
        assert!(
            model_entities(&app)
                .iter()
                .all(|entity| !matches!(entity.geometry, SemanticGeometry::MultiLeader(_))),
            "{code}: no leader was stored"
        );
    }
}
