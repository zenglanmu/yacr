//! DIMSPACE contract tests (AutoCAD 2026).
//!
//! v1 is the **selection-based, always-automatic** form: the first selected
//! linear dimension is the base, the rest are spaced in pick order at the base
//! style's `DIMDLI`, and the interactive value prompt is NOT IMPLEMENTED. Pinned
//! here:
//!
//! * target `i` moves to `base.location + n̂ · (i · DIMDLI)`, with `n̂` pointing
//!   away from the base's measured geometry;
//! * every target keeps its own origins, rotation, style snapshot and text
//!   override, and its measurement is unchanged;
//! * all changed targets land in one transaction / one undo step;
//! * every refusal is named, changes nothing and records no history;
//! * a run where every target already sits on its row is a `dimspace.unchanged`
//!   no-op with no transaction;
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
    DimensionGeometry::synthesized(
        DimensionDefinition {
            kind,
            points,
            text_middle: point(0.0, 0.0),
            text_rotation: 0.0,
            normal: p3(0.0, 0.0, 1.0),
        },
        "Standard".to_string(),
        standard_style(dimdli),
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
fn space_app(dimdli: f64, base_location: Point3) -> (Application, SessionState) {
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
    b.insert_entity(ent(
        BASE_ID,
        linear_entity(
            point(0.0, 0.0),
            point(10.0, 0.0),
            base_location,
            0.0,
            dimdli,
        ),
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
    space_app(4.0, point(5.0, -10.0))
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
fn targets_are_spaced_at_repeated_dimdli_away_from_the_measured_geometry() {
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

    // Row 1 and row 2, along -Y (the base line is below its geometry), at
    // `base.location + n̂ · (i · 4)`.
    let (first, second, line) = linear_points(&stored(&app, TARGET_A));
    assert_eq!((first, second), (point(20.0, 5.0), point(30.0, 5.0)));
    assert_eq!(line, point(5.0, -14.0), "target A row 1");
    assert!(approx(stored(&app, TARGET_A).measurement(), 10.0));
    let (first, second, line) = linear_points(&stored(&app, TARGET_B));
    assert_eq!((first, second), (point(40.0, 0.0), point(50.0, 0.0)));
    assert_eq!(line, point(5.0, -18.0), "target B row 2");
    assert!(approx(stored(&app, TARGET_B).measurement(), 10.0));
    // The base is untouched.
    assert_eq!(linear_points(&stored(&app, BASE_ID)).2, point(5.0, -10.0));
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
    // Build the two targets exactly at rows 1 and 2 (spacing 4, base at y −10).
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
            point(5.0, -14.0),
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
            point(5.0, -18.0),
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
        (vec![BASE_ID, ALIGNED_ID], "dimspace.kind"),
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
fn bad_spacing_and_an_undecidable_normal_are_refused() {
    // DIMDLI = 0 would stack every target on the base line.
    let (mut app, mut session) = space_app(0.0, point(5.0, -10.0));
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
        format!("{error:?}").contains("dimspace.bad_spacing"),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);

    // A base dimension line on its measured segment leaves the side undecidable.
    let (mut app, mut session) = space_app(4.0, point(5.0, 0.0));
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
