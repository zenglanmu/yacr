//! ARRAYRECT / ARRAYPOLAR edit-command contract tests (ADR 0016 decision D).
//!
//! Pins our declared geometry rules (spacing = the distance between base points,
//! growth up/right; polar placement with/without item rotation), the explicit
//! refusal set, and the one-transaction/one-undo-step N-copies shape.

use super::*;

/// Set a numeric session tool parameter (the typed channel validates it).
fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

/// A drawing with one LINE (id 1) `(0,0)-(2,0)` on layer 0. Its bounding-box
/// lower-left is the origin, so it is a clean array base point.
fn one_line_app() -> (Application, SessionState) {
    array_app(vec![(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(2.0, 0.0),
        },
    )])
}

/// A drawing with a supported LINE (id 1) and an untransformable Opaque entity
/// (id 2).
fn line_and_opaque_app() -> (Application, SessionState) {
    array_app(vec![
        (
            1,
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(2.0, 0.0),
            },
        ),
        (
            2,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        ),
    ])
}

/// Build an application with layer 0 and the given model-space entities.
fn array_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (index, (id, geometry)) in entities.into_iter().enumerate() {
        b.insert_entity(ent(id, geometry, index as i64)).unwrap();
    }
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

/// Every model-space LINE's `(start, end)`, sorted by `(start, end)` so the array
/// assertions are order-independent.
fn all_lines(app: &Application) -> Vec<(Point3, Point3)> {
    let mut lines: Vec<(Point3, Point3)> = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect();
    lines.sort_by(|a, b| {
        (a.0.x, a.0.y, a.1.x, a.1.y)
            .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
            .expect("finite coordinates")
    });
    lines
}

/// A LINE whose two endpoints are exactly `a`/`b` (either order).
fn line_at(a: Point3, b: Point3) -> (Point3, Point3) {
    (a, b)
}

/// Whether two line lists match within `1e-9` on every coordinate (polar
/// placement uses trigonometry, so the coordinates are near- but not exactly the
/// declared values).
fn lines_near(actual: &[(Point3, Point3)], expected: &[(Point3, Point3)]) -> bool {
    let near = |a: Point3, b: Point3| {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
    };
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|((a0, a1), (e0, e1))| near(*a0, *e0) && near(*a1, *e1))
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

fn array_rect_payload(refs: Vec<SelectionRef>) -> CommandPayload {
    CommandPayload::ArrayRect { refs }
}

fn array_polar_payload(refs: Vec<SelectionRef>, center: Point3) -> CommandPayload {
    CommandPayload::ArrayPolar { refs, center }
}

#[test]
fn array_rect_spacing_is_the_distance_between_base_points_and_grows_up_and_right() {
    let (mut app, mut session) = one_line_app();
    // 3 rows x 2 columns: cell (row, column) offsets by (column*10, row*5). The
    // grid total counts the source, so 3*2 - 1 = 5 copies are inserted.
    set_number(&mut session, ToolParameter::ArrayRows, 3.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 2.0);
    set_number(&mut session, ToolParameter::ArrayRowSpacing, 5.0);
    set_number(&mut session, ToolParameter::ArrayColumnSpacing, 10.0);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    // 3*2 - 1 = 5 independent copies (the source's own cell is skipped).
    assert_eq!(outcome.objects.len(), 5);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    // The source is retained (our rule) plus the five copies at every cell except
    // the source's own (0, 0).
    let mut expected: Vec<(Point3, Point3)> = vec![line_at(point(0.0, 0.0), point(2.0, 0.0))];
    for row in 0..3 {
        for column in 0..2 {
            if row == 0 && column == 0 {
                continue;
            }
            let dx = column as f64 * 10.0;
            let dy = row as f64 * 5.0;
            expected.push(line_at(point(dx, dy), point(dx + 2.0, dy)));
        }
    }
    expected.sort_by(|a, b| {
        (a.0.x, a.0.y, a.1.x, a.1.y)
            .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
            .unwrap()
    });
    assert_eq!(
        all_lines(&app),
        expected,
        "spacing is the distance between base points, growing up and to the right"
    );
}

#[test]
fn array_rect_single_cell_produces_no_copies_with_an_explicit_diagnostic() {
    // rows x columns counts the source object, so a 1x1 array inserts nothing and
    // leaves the source in place. 0 is a result, reported explicitly.
    let (mut app, mut session) = one_line_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 0, "a 1x1 array produces no copies");
    assert!(outcome.changes.is_none(), "no transaction is opened");
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 1, "the source is retained");
    assert_eq!(undo_depth(&app), 0, "no history entry");
    assert_eq!(
        all_lines(&app),
        vec![line_at(point(0.0, 0.0), point(2.0, 0.0))],
        "the source line is untouched"
    );
    // The explicit diagnostic names the 0-copy result (never a silent success).
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.arrayrect")
        .expect("a no-copy array reports a diagnostic");
    assert!(
        diagnostic.message.contains("0 copies"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn array_rect_rows_or_columns_of_one_produce_no_offset() {
    // 3 rows x 1 column: only row offsets, no column offset (columnSpacing is
    // deliberately non-zero to prove it is never applied).
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayRows, 3.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 1.0);
    set_number(&mut session, ToolParameter::ArrayRowSpacing, 4.0);
    set_number(&mut session, ToolParameter::ArrayColumnSpacing, 99.0);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    assert_eq!(entity_count(&app), 3, "source + 2 row copies");
    for (start, _) in all_lines(&app) {
        assert_eq!(start.x, 0.0, "a single column applies no column offset");
    }

    // 1 row x 3 columns: only column offsets, no row offset.
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayRows, 1.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 3.0);
    set_number(&mut session, ToolParameter::ArrayRowSpacing, 99.0);
    set_number(&mut session, ToolParameter::ArrayColumnSpacing, 4.0);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    assert_eq!(entity_count(&app), 3, "source + 2 column copies");
    for (start, _) in all_lines(&app) {
        assert_eq!(start.y, 0.0, "a single row applies no row offset");
    }
}

#[test]
fn array_count_at_cap_is_accepted_and_cap_plus_one_is_refused() {
    // At the cap: `rows x columns - 1` copies of one source is exactly
    // MAX_ARRAY_ENTITIES, so rows x columns = 100001.
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayRows, 100_001.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    assert_eq!(
        outcome.objects.len(),
        100_000,
        "the count exactly at the cap is accepted"
    );

    // One above the cap is refused explicitly, never truncated.
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayRows, 100_002.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 1.0);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .expect_err("above the cap is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 1, "only the source remains");
    assert_eq!(undo_depth(&app), 0, "a refusal records no history");
}

#[test]
fn array_rect_refuses_negative_spacing_and_changes_nothing() {
    // The channel accepts any finite spacing (including negative); the command is
    // what refuses the unverified reversal.
    let (mut app, mut session) = one_line_app();
    session
        .set_tool_parameter(ToolParameter::ArrayRowSpacing, ToolValue::Number(-1.0))
        .expect("the channel accepts a finite negative spacing");
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .expect_err("negative spacing is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);

    // The column spacing is refused too.
    let (mut app, mut session) = one_line_app();
    session
        .set_tool_parameter(ToolParameter::ArrayColumnSpacing, ToolValue::Number(-2.5))
        .expect("the channel accepts a finite negative spacing");
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .expect_err("negative column spacing is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn array_rect_refuses_an_empty_selection() {
    let (mut app, mut session) = one_line_app();
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(Vec::new()),
    )
    .expect_err("an empty selection is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
}

#[test]
fn array_rect_refuses_an_untransformable_source_and_writes_nothing() {
    // A supported LINE plus an Opaque entity: the whole command fails rather than
    // dropping the untransformable source from the array. rows = 2 so the array
    // really produces copies (and therefore probes the transform).
    let (mut app, mut session) = line_and_opaque_app();
    set_number(&mut session, ToolParameter::ArrayRows, 2.0);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1), ref_of(2)]),
    )
    .expect_err("an untransformable source refuses the whole command");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 2, "both sources are retained");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn array_rect_multi_copy_array_is_one_transaction_and_one_undo_step() {
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayRows, 2.0);
    set_number(&mut session, ToolParameter::ArrayColumns, 2.0);
    set_number(&mut session, ToolParameter::ArrayRowSpacing, 3.0);
    set_number(&mut session, ToolParameter::ArrayColumnSpacing, 3.0);
    let before = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayRectangular,
        array_rect_payload(vec![ref_of(1)]),
    )
    .unwrap();
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step for a three-copy array");
    assert_eq!(entity_count(&app), 4, "source + three copies (2x2 - 1)");

    // Undo restores the document exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1, "undo removes every copy");
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(
        all_lines(&app),
        vec![line_at(point(0.0, 0.0), point(2.0, 0.0))],
        "the source line is exactly restored"
    );
}

#[test]
fn array_rect_refuses_non_integer_or_below_one_counts_at_the_channel() {
    // The channel is the authority for the integer >= 1 rule (the command's own
    // check mirrors it but is unreachable through a validated store).
    for refused in [0.0, 0.5, -1.0] {
        assert!(
            session_refuses(ToolParameter::ArrayRows, ToolValue::Number(refused)),
            "ArrayRows = {refused} must be refused"
        );
        assert!(
            session_refuses(ToolParameter::ArrayColumns, ToolValue::Number(refused)),
            "ArrayColumns = {refused} must be refused"
        );
        assert!(
            session_refuses(ToolParameter::ArrayItems, ToolValue::Number(refused)),
            "ArrayItems = {refused} must be refused"
        );
    }
}

/// Whether a fresh session refuses a tool-parameter set.
fn session_refuses(parameter: ToolParameter, value: ToolValue) -> bool {
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    session.set_tool_parameter(parameter, value).is_err()
}

#[test]
fn array_polar_without_item_rotation_follows_the_declared_placement_rule() {
    // Base point is the selection bounding-box lower-left (0,0); the centre is
    // (0,-10), so the base direction is +y and the radius is 10. With the
    // full-circle default (360) and 4 items, item i sits at 90 + i*90 degrees:
    // (0,0), (-10,-10), (0,-20), (10,-10). The grid total counts the source, so
    // item 0 (the source's own direction) is skipped and 4 - 1 = 3 copies are
    // **translated only** (orientation unchanged).
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 4.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(0.0, -10.0)),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 3, "items - 1 = three copies");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    let expected = {
        let mut lines = vec![
            line_at(point(0.0, 0.0), point(2.0, 0.0)), // retained source (item 0)
            line_at(point(-10.0, -10.0), point(-8.0, -10.0)), // item 1
            line_at(point(0.0, -20.0), point(2.0, -20.0)), // item 2
            line_at(point(10.0, -10.0), point(12.0, -10.0)), // item 3
        ];
        lines.sort_by(|a, b| {
            (a.0.x, a.0.y, a.1.x, a.1.y)
                .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
                .unwrap()
        });
        lines
    };
    assert!(
        lines_near(&all_lines(&app), &expected),
        "translate-only polar placement (orientation unchanged): {:?}",
        all_lines(&app)
    );
}

#[test]
fn array_polar_with_item_rotation_rotates_each_copy_about_the_centre() {
    // Same centre/base as above, but with rotateItems = true: each copy is rotated
    // about the centre. Item 1 (90 degrees) turns the horizontal line into a
    // **vertical** line at x = -10 from (-10,-10) to (-10,-8).
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 4.0);
    session
        .set_tool_parameter(ToolParameter::ArrayRotateItems, ToolValue::Flag(true))
        .expect("the rotate flag is admitted");
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(0.0, -10.0)),
    )
    .unwrap();
    assert_eq!(entity_count(&app), 4, "source + three copies (4 - 1)");

    let lines = all_lines(&app);
    let vertical = |a: Point3, b: Point3| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() > 1e-9;
    // The rotated copies at items 1 and 3 are vertical (item 0 is skipped).
    let vertical_copies: Vec<&(Point3, Point3)> =
        lines.iter().filter(|(a, b)| vertical(*a, *b)).collect();
    assert_eq!(vertical_copies.len(), 2, "items 1 and 3 are vertical");
    for (a, b) in vertical_copies {
        assert!((a.x - b.x).abs() < 1e-9, "vertical copy keeps one x");
    }
    assert!(
        lines.iter().any(|(a, b)| {
            (a.x + 10.0).abs() < 1e-9
                && (b.x + 10.0).abs() < 1e-9
                && (((a.y + 10.0).abs() < 1e-9 && (b.y + 8.0).abs() < 1e-9)
                    || ((a.y + 8.0).abs() < 1e-9 && (b.y + 10.0).abs() < 1e-9))
        }),
        "item 1 is the line (-10,-10)-(-10,-8): {lines:?}"
    );
}

/// Our declared, **unverified** ARRAYPOLAR step rule (ADR 0016 decision D): item
/// `i` (`i` in `0..items`) sits at `i * fillAngle / items` from the base point's
/// direction — the fill is divided into `items` **steps**, not `items - 1`.
///
/// This **diverges** from AutoCAD, which spans a partial fill between the first
/// and last item (`fill / (items - 1)`); AutoCAD's exact rule is unverified, so
/// the divergence is a **declared choice**, not an oversight. It is pinned here
/// numerically so a reader sees the choice.
#[test]
fn array_polar_partial_fill_angle_pins_the_declared_step_rule() {
    // Base point = bounding-box lower-left (0,0); centre (-10,0) => base direction
    // +x, radius 10. The copies are translate-only (rotateItems defaults off).
    //
    // items = 3, fillAngle = 180:
    //   ours    (180 / 3): offsets 0°, 60°, 120°
    //   AutoCAD (180 / 2): offsets 0°, 90°, 180°
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 3.0);
    set_number(&mut session, ToolParameter::ArrayFillAngle, 180.0);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(-10.0, 0.0)),
    )
    .unwrap();

    // A translate-only copy at `degrees` from +x: its start is the target point and
    // its end is the same shifted by the source's (2,0) extent.
    let copy = |degrees: f64| {
        let angle = degrees.to_radians();
        let start = point(-10.0 + 10.0 * angle.cos(), 10.0 * angle.sin());
        line_at(start, point(start.x + 2.0, start.y))
    };
    let expected = {
        let mut lines = vec![
            line_at(point(0.0, 0.0), point(2.0, 0.0)), // retained source (item 0)
            copy(60.0),                                // item 1: 1 * 180 / 3
            copy(120.0),                               // item 2: 2 * 180 / 3
        ];
        lines.sort_by(|a, b| {
            (a.0.x, a.0.y, a.1.x, a.1.y)
                .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
                .unwrap()
        });
        lines
    };
    assert!(
        lines_near(&all_lines(&app), &expected),
        "our rule divides the fill by items (0°/60°/120°): {:?}",
        all_lines(&app)
    );
    // The divergence, made explicit: no copy sits at 90° (AutoCAD's span rule would
    // place item 1 at (-10, 10)).
    assert!(
        !all_lines(&app)
            .iter()
            .any(|(s, _)| (s.x + 10.0).abs() < 1e-9 && (s.y - 10.0).abs() < 1e-9),
        "no copy at 90°: the fill is divided by items, not items - 1"
    );

    // Full-circle case (the declared default): fillAngle = 360, items = 4 =>
    // 0°, 90°, 180°, 270° (360 / 4).
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 4.0);
    set_number(&mut session, ToolParameter::ArrayFillAngle, 360.0);
    execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(-10.0, 0.0)),
    )
    .unwrap();
    let expected = {
        let mut lines = vec![
            line_at(point(0.0, 0.0), point(2.0, 0.0)), // item 0
            copy(90.0),
            copy(180.0),
            copy(270.0),
        ];
        lines.sort_by(|a, b| {
            (a.0.x, a.0.y, a.1.x, a.1.y)
                .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
                .unwrap()
        });
        lines
    };
    assert!(
        lines_near(&all_lines(&app), &expected),
        "full circle: 0°/90°/180°/270° (360/4): {:?}",
        all_lines(&app)
    );
}

#[test]
fn array_polar_single_item_produces_no_copies_with_an_explicit_diagnostic() {
    // `items` counts the source object, so a 1-item array inserts nothing and
    // leaves the source in place. 0 is a result, reported explicitly.
    let (mut app, mut session) = one_line_app();
    // ArrayItems defaults to 1.
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(0.0, -10.0)),
    )
    .unwrap();
    assert_eq!(
        outcome.objects.len(),
        0,
        "a 1-item array produces no copies"
    );
    assert!(outcome.changes.is_none(), "no transaction is opened");
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(entity_count(&app), 1, "the source is retained");
    assert_eq!(undo_depth(&app), 0, "no history entry");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "drawing.arraypolar")
        .expect("a no-copy array reports a diagnostic");
    assert!(
        diagnostic.message.contains("0 copies"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn array_polar_refuses_a_degenerate_centre() {
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 4.0);
    let before = revision(&app);
    // A centre coincident with the base point (0,0) has no direction.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(0.0, 0.0)),
    )
    .expect_err("a degenerate centre is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);

    // A non-finite centre is refused too.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(f64::NAN, 0.0)),
    )
    .expect_err("a non-finite centre is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
}

#[test]
fn array_polar_refuses_an_empty_selection_and_an_untransformable_source() {
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 3.0);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(Vec::new(), point(0.0, -10.0)),
    )
    .expect_err("an empty selection is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");

    // Opaque cannot be rotated about the centre.
    let (mut app, mut session) = line_and_opaque_app();
    set_number(&mut session, ToolParameter::ArrayItems, 3.0);
    session
        .set_tool_parameter(ToolParameter::ArrayRotateItems, ToolValue::Flag(true))
        .expect("the rotate flag is admitted");
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1), ref_of(2)], point(0.0, -10.0)),
    )
    .expect_err("an untransformable source refuses the whole command");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before);
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn array_polar_fill_angle_and_item_refusals_are_the_channel_contract() {
    // The channel accepts `0 < |angle| <= 360` (so the full-circle default 360 is
    // re-settable) and refuses 0, anything beyond a single turn, and a fractional
    // item count.
    for refused in [0.0, 360.5, 720.0, -720.0, -360.5] {
        assert!(
            session_refuses(ToolParameter::ArrayFillAngle, ToolValue::Number(refused)),
            "ArrayFillAngle = {refused} must be refused by the channel"
        );
    }
    for accepted in [360.0, -360.0, 180.0] {
        assert!(
            !session_refuses(ToolParameter::ArrayFillAngle, ToolValue::Number(accepted)),
            "ArrayFillAngle = {accepted} must be accepted (0 < |angle| <= 360)"
        );
    }
    for refused in [0.0, 0.5, -1.0] {
        assert!(
            session_refuses(ToolParameter::ArrayItems, ToolValue::Number(refused)),
            "ArrayItems = {refused} must be refused"
        );
    }
    // The default 360 (full circle) is accepted by the command; 4 items produce
    // 4 - 1 = 3 copies.
    let (mut app, mut session) = one_line_app();
    set_number(&mut session, ToolParameter::ArrayItems, 4.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ArrayPolar,
        array_polar_payload(vec![ref_of(1)], point(0.0, -10.0)),
    )
    .expect("the full-circle default is a valid ARRAYPOLAR");
    assert_eq!(outcome.objects.len(), 3);
}
