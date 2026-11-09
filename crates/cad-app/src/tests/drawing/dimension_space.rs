//! DIMSPACE contract tests (AutoCAD 2026).
//!
//! v1 is the **selection-based** form: the first selected dimension is the base,
//! the rest are spaced in pick order at either twice the base **style text
//! height** (Auto), an explicit positive spacing, or explicit `0` (align).
//! The interactive value prompt is replaced by the payload. Pinned here:
//!
//! * target `i` moves to `base.location + n̂ · (i · spacing)`, with `n̂` pointing
//!   away from the base's measured geometry;
//! * every target keeps its own origins, subtype/rotation, style snapshot and
//!   text override, and its measurement is unchanged;
//! * an explicit spacing value produces exact offsets and all changed targets
//!   land in one transaction / one undo step;
//! * an explicit `0` aligns every target on the base dimension line;
//! * an aligned dimension is accepted as base (direction from its measured
//!   segment) and as a target (its `Aligned` subtype is preserved);
//! * angular dimensions are spaced concentrically by arc radius;
//! * every refusal is named, changes nothing and records no history;
//! * a run with no movement is a `dimspace.unchanged` no-op with no transaction;
//! * Viewer mode refuses the command.

use super::*;

const BASE_ID: u128 = 701;
const TARGET_A: u128 = 702;
const TARGET_B: u128 = 703;
const ALIGNED_ID: u128 = 704;
const RADIAL_ID: u128 = 705;
const LINE_ID: u128 = 706;
const PAPER_ID: u128 = 707;

fn standard_style(dimdli: f64) -> DimensionStyleValues {
    // `DimStyle::resolved` substitutes the documented 3.75 default for a
    // non-positive DIMDLI, so the **stored snapshot** is set directly here: the
    // bad-spacing guard must be reachable from a degenerate snapshot.
    let mut values = cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
        "Standard".to_string(),
        StyleId(0),
        None,
    );
    values.dim_line_increment = dimdli;
    values
}

fn synthesized(kind: DimensionKind, points: DimensionPoints, dimdli: f64) -> DimensionGeometry {
    synthesized_with_style(kind, points, standard_style(dimdli))
}

fn synthesized_with_style(
    kind: DimensionKind,
    points: DimensionPoints,
    style: DimensionStyleValues,
) -> DimensionGeometry {
    DimensionGeometry::synthesized(
        DimensionDefinition {
            kind,
            points,
            text_middle: point(0.0, 0.0),
            text_rotation: 0.0,
            normal: p3(0.0, 0.0, 1.0),
        },
        "Standard".to_string(),
        style,
        None,
    )
    .expect("the fixture dimension synthesizes")
}

fn linear_entity(
    first: Point3,
    second: Point3,
    location: Point3,
    rotation: f64,
    dimdli: f64,
) -> SemanticGeometry {
    SemanticGeometry::Dimension(Box::new(synthesized(
        DimensionKind::Linear { rotation },
        DimensionPoints::Linear {
            first,
            second,
            dimension_line: location,
        },
        dimdli,
    )))
}

fn ent_in_space(id: u128, geometry: SemanticGeometry, order: i64, space: SpaceId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: order,
    }
}

/// A drawing whose base dimension line is 10 units below its measured segment,
/// plus two targets at arbitrary dimension lines and one object of every shape
/// DIMSPACE must refuse.
fn space_app(
    dimdli: f64,
    base_location: Point3,
    base_text_height: f64,
) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    let mut base_style = standard_style(dimdli);
    base_style.text_height = base_text_height;
    b.insert_entity(ent(
        BASE_ID,
        SemanticGeometry::Dimension(Box::new(synthesized_with_style(
            DimensionKind::Linear { rotation: 0.0 },
            DimensionPoints::Linear {
                first: point(0.0, 0.0),
                second: point(10.0, 0.0),
                dimension_line: base_location,
            },
            base_style,
        ))),
        0,
    ))
    .unwrap();
    // Target A: its own measured segment, deliberately off any row.
    b.insert_entity(ent_in_space(
        TARGET_A,
        linear_entity(
            point(20.0, 5.0),
            point(30.0, 5.0),
            point(25.0, -3.0),
            0.0,
            dimdli,
        ),
        1,
        SpaceId::Model,
    ))
    .unwrap();
    // Target B: another rotation and dimension line.
    b.insert_entity(ent_in_space(
        TARGET_B,
        linear_entity(
            point(40.0, 0.0),
            point(50.0, 0.0),
            point(45.0, -20.0),
            0.0,
            dimdli,
        ),
        2,
        SpaceId::Model,
    ))
    .unwrap();
    b.insert_entity(ent_in_space(
        ALIGNED_ID,
        SemanticGeometry::Dimension(Box::new(synthesized(
            DimensionKind::Aligned,
            DimensionPoints::Linear {
                first: point(0.0, 60.0),
                second: point(10.0, 70.0),
                dimension_line: point(0.0, 70.0),
            },
            dimdli,
        ))),
        3,
        SpaceId::Model,
    ))
    .unwrap();
    b.insert_entity(ent_in_space(
        RADIAL_ID,
        SemanticGeometry::Dimension(Box::new(synthesized(
            DimensionKind::Radius,
            DimensionPoints::Radial {
                center: point(40.0, 0.0),
                point_on_arc: point(45.0, 0.0),
                leader_length: 0.0,
            },
            dimdli,
        ))),
        4,
        SpaceId::Model,
    ))
    .unwrap();
    b.insert_entity(ent_in_space(
        LINE_ID,
        SemanticGeometry::Line {
            start: point(0.0, 50.0),
            end: point(10.0, 50.0),
        },
        5,
        SpaceId::Model,
    ))
    .unwrap();
    b.insert_entity(ent_in_space(
        PAPER_ID,
        linear_entity(
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(5.0, -10.0),
            0.0,
            dimdli,
        ),
        6,
        SpaceId::Paper(LayoutId(1)),
    ))
    .unwrap();
    let mut style = cad_db::DimStyle::standard_metric(DimStyleId(0));
    style.dim_line_increment = dimdli;
    b.insert_dim_style(style).unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
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

fn default_app() -> (Application, SessionState) {
    space_app(4.0, point(5.0, -10.0), 2.5)
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

fn linear_points(dimension: &DimensionGeometry) -> (Point3, Point3, Point3) {
    match &dimension.definition().points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => (*first, *second, *dimension_line),
        other => panic!("expected Linear points, got {other:?}"),
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

#[test]
fn targets_are_spaced_at_repeated_base_measurement_away_from_the_measured_geometry() {
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A, TARGET_B]);
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2, "the base is never rewritten");
    let changes = out.changes.expect("change set");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after.0, before.0 + 1, "one revision bump");
    assert_eq!(
        out.diagnostics[0].code, "dimspace.applied",
        "the success diagnostic names the applied state"
    );

    // Auto = twice the base dimension's style text height (2 x 2.5 = 5), along
    // -Y (the base line is below its geometry), at
    // `base.location + n̂ · (i · 5)`.
    let (first, second, line) = linear_points(&stored(&app, TARGET_A));
    assert_eq!((first, second), (point(20.0, 5.0), point(30.0, 5.0)));
    assert_eq!(line, point(5.0, -15.0), "target A row 1");
    assert!(approx(stored(&app, TARGET_A).measurement(), 10.0));
    let (first, second, line) = linear_points(&stored(&app, TARGET_B));
    assert_eq!((first, second), (point(40.0, 0.0), point(50.0, 0.0)));
    assert_eq!(line, point(5.0, -20.0), "target B row 2");
    assert!(approx(stored(&app, TARGET_B).measurement(), 10.0));
    // The base is untouched.
    assert_eq!(linear_points(&stored(&app, BASE_ID)).2, point(5.0, -10.0));
}

#[test]
fn explicit_spacing_produces_exact_offsets_in_one_undo_step() {
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A, TARGET_B]);
    let before_undo = undo_depth(&app);
    let before = revision(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(2.5)),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert_eq!(out.changes.as_ref().unwrap().after.0, before.0 + 1);
    // Explicit 2.5: rows at -12.5 and -15, independent of the style DIMDLI (4).
    assert_eq!(linear_points(&stored(&app, TARGET_A)).2, point(5.0, -12.5));
    assert_eq!(linear_points(&stored(&app, TARGET_B)).2, point(5.0, -15.0));
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(linear_points(&stored(&app, TARGET_A)).2, point(25.0, -3.0));
    assert_eq!(linear_points(&stored(&app, TARGET_B)).2, point(45.0, -20.0));
}

#[test]
fn aligned_dimensions_work_as_base_and_keep_their_subtype_as_targets() {
    // Aligned base: direction 45° from (0,60) -> (10,70), dimension line at
    // (0,70). Explicit spacing 2 moves target A along the base's away normal.
    let (mut app, mut session) = default_app();
    select(&mut session, &[ALIGNED_ID, TARGET_A]);
    execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(2.0)),
    )
    .unwrap();
    let (_, _, line) = linear_points(&stored(&app, TARGET_A));
    let expected = point(
        0.0 - 2.0 * (std::f64::consts::FRAC_1_SQRT_2),
        70.0 + 2.0 * (std::f64::consts::FRAC_1_SQRT_2),
    );
    assert!(
        approx(line.x, expected.x) && approx(line.y, expected.y),
        "got {line:?}"
    );

    // Aligned target: subtype and measured points are preserved; only the
    // dimension line moves down one explicit row.
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, ALIGNED_ID]);
    execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(2.0)),
    )
    .unwrap();
    let aligned = stored(&app, ALIGNED_ID);
    assert_eq!(aligned.kind(), DimensionKind::Aligned, "subtype preserved");
    let (first, second, line) = linear_points(&aligned);
    assert_eq!((first, second), (point(0.0, 60.0), point(10.0, 70.0)));
    // The row is measured from the **base** dimension line (5, -10) along the
    // base's away normal (0, -1), not from the target's old line.
    assert_eq!(line, point(5.0, -12.0), "aligned target row 1");
    assert!(approx(aligned.measurement(), 200.0f64.sqrt()));
}

#[test]
fn one_transaction_one_undo_step_and_all_targets_restore() {
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A, TARGET_B]);
    let before_undo = undo_depth(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
    assert!(app.can_undo(&DocumentId(1)));
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        linear_points(&stored(&app, TARGET_A)).2,
        point(25.0, -3.0),
        "one undo restores every staged target"
    );
    assert_eq!(linear_points(&stored(&app, TARGET_B)).2, point(45.0, -20.0));
}

#[test]
fn a_target_already_on_its_row_is_a_noop_with_no_transaction() {
    // Build the two targets exactly at Auto rows 1 and 2 (spacing 5, base at
    // y −10) — the style DIMDLI 4 is deliberately different and must not be read.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        BASE_ID,
        linear_entity(
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(5.0, -10.0),
            0.0,
            4.0,
        ),
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        TARGET_A,
        linear_entity(
            point(20.0, 5.0),
            point(30.0, 5.0),
            point(5.0, -15.0),
            0.0,
            4.0,
        ),
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        TARGET_B,
        linear_entity(
            point(40.0, 0.0),
            point(50.0, 0.0),
            point(5.0, -20.0),
            0.0,
            4.0,
        ),
        2,
    ))
    .unwrap();
    let mut style = cad_db::DimStyle::standard_metric(DimStyleId(0));
    style.dim_line_increment = 4.0;
    b.insert_dim_style(style).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);

    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);
    select(&mut session, &[BASE_ID, TARGET_A, TARGET_B]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap();
    assert!(out.changes.is_none(), "no transaction");
    assert_eq!(out.diagnostics[0].code, "dimspace.unchanged");
    assert_eq!(revision(&app), before_revision, "no revision bump");
    assert_eq!(undo_depth(&app), before_undo, "no undo entry");
}

#[test]
fn refusals_are_named_and_change_nothing() {
    let (mut app, mut session) = default_app();
    let before_revision = revision(&app);
    let before_undo = undo_depth(&app);

    let cases: Vec<(Vec<u128>, &str)> = vec![
        (vec![BASE_ID], "dimspace.selection"),
        (vec![BASE_ID, RADIAL_ID], "dimspace.kind"),
        (vec![BASE_ID, LINE_ID], "dimspace.not_dimension"),
        (vec![BASE_ID, PAPER_ID], "dimspace.space"),
    ];
    for (ids, code) in cases {
        select(&mut session, &ids);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::DimSpace,
            CommandPayload::None,
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains(code),
            "{ids:?}: expected {code}, got {error:?}"
        );
        assert_eq!(revision(&app), before_revision, "{code} bumps no revision");
        assert_eq!(undo_depth(&app), before_undo, "{code} records no undo");
    }
    // A payload is refused by name: the selection is the whole input.
    select(&mut session, &[BASE_ID, TARGET_A]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("dimspace.payload"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before_revision);
}

#[test]
fn bad_explicit_spacing_and_an_undecidable_normal_are_refused() {
    // A negative explicit spacing is refused by name; 0 is the align form (see
    // `explicit_zero_aligns_every_target_on_the_base_dimension_line`).
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(-1.0)),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("dimspace.bad_spacing"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);

    // A base dimension line on its measured segment leaves the side undecidable.
    let (mut app, mut session) = space_app(4.0, point(5.0, 0.0), 2.5);
    select(&mut session, &[BASE_ID, TARGET_A]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("dimspace.undecidable_normal"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn auto_uses_the_style_text_height_not_the_zero_measurement() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        BASE_ID,
        linear_entity(
            point(5.0, 0.0),
            point(5.0, 0.0),
            point(5.0, -10.0),
            0.0,
            4.0,
        ),
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        TARGET_A,
        linear_entity(
            point(20.0, 5.0),
            point(30.0, 5.0),
            point(25.0, -3.0),
            0.0,
            4.0,
        ),
        1,
    ))
    .unwrap();
    let mut style = cad_db::DimStyle::standard_metric(DimStyleId(0));
    style.dim_line_increment = 4.0;
    b.insert_dim_style(style).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);

    let before = revision(&app);
    select(&mut session, &[BASE_ID, TARGET_A]);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        out.changes.as_ref().expect("a transaction").after.0,
        before.0 + 1
    );
    // Auto is 2 x 2.5 = 5 even though the base measures zero.
    assert_eq!(linear_points(&stored(&app, TARGET_A)).2, point(5.0, -15.0));
    assert_eq!(out.diagnostics[0].code, "dimspace.applied");
}

#[test]
fn explicit_zero_aligns_every_target_on_the_base_dimension_line() {
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A, TARGET_B]);
    let before_undo = undo_depth(&app);
    let out = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(0.0)),
    )
    .unwrap();
    assert_eq!(out.objects.len(), 2);
    assert_eq!(linear_points(&stored(&app, TARGET_A)).2, point(5.0, -10.0));
    assert_eq!(linear_points(&stored(&app, TARGET_B)).2, point(5.0, -10.0));
    assert_eq!(undo_depth(&app), before_undo + 1, "one undo step");
    assert!(
        out.diagnostics[0].message.contains("aligned"),
        "{}",
        out.diagnostics[0].message
    );
}

#[test]
fn auto_refuses_a_base_style_without_a_positive_text_height() {
    let (mut app, mut session) = space_app(4.0, point(5.0, -10.0), 0.0);
    select(&mut session, &[BASE_ID, TARGET_A]);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("dimspace.style_height"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
}

#[test]
fn angular_dimensions_are_spaced_concentrically() {
    const ANGULAR_BASE: u128 = 708;
    const ANGULAR_TARGET: u128 = 709;

    let angular = |id: u128, arc_point: Point3, order: i64| {
        ent_in_space(
            id,
            SemanticGeometry::Dimension(Box::new(synthesized(
                DimensionKind::Angular3Pt,
                DimensionPoints::Angular3Pt {
                    vertex: point(0.0, 0.0),
                    first: point(10.0, 0.0),
                    second: point(0.0, 10.0),
                    arc_point,
                },
                4.0,
            ))),
            order,
            SpaceId::Model,
        )
    };
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(angular(ANGULAR_BASE, point(3.5, 3.5), 0))
        .unwrap();
    b.insert_entity(angular(ANGULAR_TARGET, point(1.0, 1.0), 1))
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    select(&mut session, &[ANGULAR_BASE, ANGULAR_TARGET]);

    execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::DimSpace(Some(2.0)),
    )
    .unwrap();

    let base_radius = (3.5f64 * 3.5 + 3.5 * 3.5).sqrt();
    let stored = stored(&app, ANGULAR_TARGET);
    let DimensionPoints::Angular3Pt { arc_point, .. } = &stored.definition().points else {
        panic!("expected Angular3Pt points");
    };
    let radius = arc_point.x.hypot(arc_point.y);
    assert!(
        approx(radius, base_radius + 2.0),
        "target radius {radius} must be base {base_radius} + 2"
    );
    // The target's own radial direction is preserved.
    assert!(
        approx(arc_point.x, arc_point.y),
        "direction stays 45 degrees: {arc_point:?}"
    );
}

#[test]
fn dimspace_is_work_only_and_refused_in_viewer_mode() {
    assert!(CommandId::DimSpace.requires_work_mode());
    let (mut app, mut session) = default_app();
    select(&mut session, &[BASE_ID, TARGET_A]);
    session.switch_mode(AppMode::Viewer).unwrap();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::DimSpace,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    assert_eq!(revision(&app), before);
}
