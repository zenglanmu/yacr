//! ALIGN edit-command contract tests (ADR 0016 decision F).
//!
//! Pins the two-pair move+rotate transform, the `AlignScale` uniform-scale
//! behaviour (and the default-off difference), the explicit refusal set, and the
//! one-transaction/one-undo-step in-place shape.

use super::*;

/// A drawing with a LINE `(0,0)-(1,0)` (id 1) and a closed 2×1 rectangle polyline
/// `(0,0)-(2,0)-(2,1)-(0,1)` (id 2) on layer 0.
fn align_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(2.0, 0.0),
                point(2.0, 1.0),
                point(0.0, 1.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
        },
        1,
    ))
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
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn align_payload(
    refs: Vec<SelectionRef>,
    source1: Point3,
    dest1: Point3,
    source2: Point3,
    dest2: Point3,
) -> CommandPayload {
    CommandPayload::Align {
        refs,
        source1,
        dest1,
        source2,
        dest2,
    }
}

fn set_align_scale(session: &mut SessionState, value: bool) {
    session
        .set_tool_parameter(ToolParameter::AlignScale, ToolValue::Flag(value))
        .unwrap_or_else(|error| panic!("AlignScale = {value} refused: {error}"));
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn line_of(app: &Application, id: u128) -> (Point3, Point3) {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => (*start, *end),
        other => panic!("expected a line, got {other:?}"),
    }
}

fn polyline_of(app: &Application, id: u128) -> Vec<Point3> {
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Polyline { points, .. } => points.clone(),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

/// The source/destination pairs used by most tests: `(0,0)→(10,0)` plus a source
/// direction `(1,0)` that turns onto the destination direction `(0,3)` (a +90°
/// rotation), with a non-unit destination length so scale-on/off differ.
fn two_pairs() -> (Point3, Point3, Point3, Point3) {
    (
        point(0.0, 0.0),
        point(10.0, 0.0),
        point(1.0, 0.0),
        point(10.0, 3.0),
    )
}

#[test]
fn align_two_pairs_move_and_rotate_without_scaling() {
    // AlignScale is off (the documented default), so the transform is rigid:
    // translate `(0,0)→(10,0)` then rotate +90° about the first source point. The
    // line `(0,0)-(1,0)` becomes `(10,0)-(10,1)`. A wrong pivot (e.g. rotating
    // about the origin) or a wrong rotation order would not land here.
    let (mut app, mut session) = align_app();
    let before = revision(&app);
    let (source1, dest1, source2, dest2) = two_pairs();
    execute(
        &mut app,
        &mut session,
        CommandId::AlignEntities,
        align_payload(vec![ref_of(1)], source1, dest1, source2, dest2),
    )
    .unwrap();
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(10.0, 0.0)), "{start:?}");
    assert!(near(end, point(10.0, 1.0)), "{end:?}");
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn align_scale_true_applies_a_uniform_scale_about_the_first_source_point() {
    // Same pairs with AlignScale = true: factor = |(0,3)| / |(1,0)| = 3, so the
    // line `(0,0)-(1,0)` becomes `(10,0)-(10,3)` — the source end maps exactly
    // onto `dest2`, which is the whole point of the scale prompt.
    let (mut app, mut session) = align_app();
    set_align_scale(&mut session, true);
    let (source1, dest1, source2, dest2) = two_pairs();
    execute(
        &mut app,
        &mut session,
        CommandId::AlignEntities,
        align_payload(vec![ref_of(1)], source1, dest1, source2, dest2),
    )
    .unwrap();
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(10.0, 0.0)), "{start:?}");
    assert!(near(end, point(10.0, 3.0)), "{end:?}");
}

#[test]
fn align_scale_true_keeps_a_rectangle_a_rectangle_with_the_expected_side_ratio() {
    // A 2×1 rectangle, aligned with a source pair of length 1 and a destination
    // pair of length 2 (a uniform factor 2) plus a +90° rotation about the origin:
    // the sides become 4 and 2, so the 2:1 ratio is preserved. A non-uniform scale
    // would break it.
    let (mut app, mut session) = align_app();
    set_align_scale(&mut session, true);
    execute(
        &mut app,
        &mut session,
        CommandId::AlignEntities,
        align_payload(
            vec![ref_of(2)],
            point(0.0, 0.0),
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(0.0, 2.0),
        ),
    )
    .unwrap();
    let points = polyline_of(&app, 2);
    for expected in [
        point(0.0, 0.0),
        point(0.0, 4.0),
        point(-2.0, 4.0),
        point(-2.0, 0.0),
    ] {
        assert!(
            points.iter().any(|q| near(*q, expected)),
            "missing {expected:?} in {points:?}"
        );
    }
    // Adjacent side lengths are 4 (vertical) and 2 (horizontal): the 2:1 ratio of
    // the source rectangle is preserved by the uniform scale.
    let side = |a: Point3, b: Point3| (a.x - b.x).hypot(a.y - b.y);
    let mut sides = [
        side(points[0], points[1]),
        side(points[1], points[2]),
        side(points[2], points[3]),
        side(points[3], points[0]),
    ];
    sides.sort_by(|a, b| a.partial_cmp(b).expect("finite sides"));
    assert!((sides[0] - 2.0).abs() < 1e-9, "{sides:?}");
    assert!((sides[1] - 2.0).abs() < 1e-9, "{sides:?}");
    assert!((sides[2] - 4.0).abs() < 1e-9, "{sides:?}");
    assert!((sides[3] - 4.0).abs() < 1e-9, "{sides:?}");
}

#[test]
fn align_refuses_non_finite_degenerate_and_empty_inputs_without_change() {
    // Every refused input leaves the document and history untouched.
    let cases: [(&str, CommandPayload); 4] = [
        (
            "non-finite point",
            align_payload(
                vec![ref_of(1)],
                point(f64::NAN, 0.0),
                point(10.0, 0.0),
                point(1.0, 0.0),
                point(10.0, 3.0),
            ),
        ),
        (
            "degenerate source pair",
            align_payload(
                vec![ref_of(1)],
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(0.0, 0.0),
                point(10.0, 3.0),
            ),
        ),
        (
            "degenerate destination pair",
            align_payload(
                vec![ref_of(1)],
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(1.0, 0.0),
                point(10.0, 0.0),
            ),
        ),
        (
            "empty selection",
            align_payload(
                Vec::new(),
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(1.0, 0.0),
                point(10.0, 3.0),
            ),
        ),
    ];
    for (label, payload) in cases {
        let (mut app, mut session) = align_app();
        let before = revision(&app);
        let error = execute(&mut app, &mut session, CommandId::AlignEntities, payload).unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(_)),
            "{label}: {error:?}"
        );
        assert_eq!(revision(&app), before, "{label} must change nothing");
        assert_eq!(entity_count(&app), 2, "{label}");
        assert_eq!(undo_depth(&app), 0, "{label} must record no history");
        let (start, end) = line_of(&app, 1);
        assert!(
            near(start, point(0.0, 0.0)) && near(end, point(1.0, 0.0)),
            "{label}: the geometry must be untouched"
        );
    }
}

#[test]
fn align_refuses_untransformable_geometry_and_writes_nothing() {
    // A supported LINE plus an Opaque entity: the whole command fails rather than
    // dropping the untransformable source from the transform.
    let mut app = line_and_opaque_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::AlignEntities,
        align_payload(
            vec![ref_of(1), ref_of(2)],
            point(0.0, 0.0),
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(0.0, 1.0),
        ),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 2, "both entities are retained");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn align_is_one_transaction_one_undo_step_and_undo_restores_exactly() {
    let (mut app, mut session) = align_app();
    let original_line = line_of(&app, 1);
    let original_rect = polyline_of(&app, 2);
    let before = revision(&app);
    let (source1, dest1, source2, dest2) = two_pairs();
    execute(
        &mut app,
        &mut session,
        CommandId::AlignEntities,
        align_payload(vec![ref_of(1), ref_of(2)], source1, dest1, source2, dest2),
    )
    .unwrap();
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(
        undo_depth(&app),
        1,
        "one undo step covers the whole selection"
    );
    assert!(
        !near(polyline_of(&app, 2)[0], original_rect[0]),
        "both entities were transformed in place"
    );
    assert_eq!(
        entity_count(&app),
        2,
        "ALIGN transforms in place (no copies added)"
    );

    // Undo restores the exact original coordinates.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(line_of(&app, 1), original_line);
    assert_eq!(polyline_of(&app, 2), original_rect);
    assert_eq!(undo_depth(&app), 0);
}
