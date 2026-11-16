//! `ARRAY` / `-ARRAY` / `ARRAYCLASSIC` **router** contracts.
//!
//! The typed command line writes the session array channel and then dispatches the
//! ordinary `ArrayRect`/`ArrayPolar` payload to the selector the user typed. These
//! tests pin what that path actually produces: the copy count, the placement and
//! the one transaction / one undo step, for each of the three spellings — plus the
//! planner's own cap refusal writing nothing.

use super::*;

use super::array::{all_lines, array_app, one_line_app, undo_depth};

use crate::ExtendedCommandId;

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn line_at(a: Point3, b: Point3) -> (Point3, Point3) {
    (a, b)
}

fn sorted(mut lines: Vec<(Point3, Point3)>) -> Vec<(Point3, Point3)> {
    lines.sort_by(|a, b| {
        (a.0.x, a.0.y, a.1.x, a.1.y)
            .partial_cmp(&(b.0.x, b.0.y, b.1.x, b.1.y))
            .expect("finite coordinates")
    });
    lines
}

fn rect_settings(session: &mut SessionState, rows: f64, columns: f64, row: f64, column: f64) {
    set_number(session, ToolParameter::ArrayRows, rows);
    set_number(session, ToolParameter::ArrayColumns, columns);
    set_number(session, ToolParameter::ArrayRowSpacing, row);
    set_number(session, ToolParameter::ArrayColumnSpacing, column);
}

/// The `ARRAY` family selector of one name, as the session dispatches it.
fn selector(name: &str) -> CommandId {
    match name {
        "ARRAY" => CommandId::Extended(ExtendedCommandId::Array),
        "-ARRAY" => CommandId::Extended(ExtendedCommandId::DashArray),
        "ARRAYCLASSIC" => CommandId::Extended(ExtendedCommandId::ArrayClassic),
        other => panic!("not an array selector: {other}"),
    }
}

#[test]
fn every_spelling_routes_the_rect_payload_to_the_one_grid_planner() {
    // 2x2 with (10, 5) spacing: the source's own cell is skipped, so three
    // copies are inserted at (10,0), (0,5) and (10,5).
    let expected = sorted(vec![
        line_at(point(0.0, 0.0), point(2.0, 0.0)),
        line_at(point(10.0, 0.0), point(12.0, 0.0)),
        line_at(point(0.0, 5.0), point(2.0, 5.0)),
        line_at(point(10.0, 5.0), point(12.0, 5.0)),
    ]);
    for name in ["ARRAY", "-ARRAY", "ARRAYCLASSIC"] {
        let (mut app, mut session) = one_line_app();
        rect_settings(&mut session, 2.0, 2.0, 5.0, 10.0);
        let before = revision(&app);
        let outcome = execute(
            &mut app,
            &mut session,
            selector(name),
            CommandPayload::ArrayRect {
                refs: vec![ref_of(1)],
            },
        )
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(
            outcome.objects.len(),
            3,
            "{name}: two rows x two columns - 1"
        );
        assert_eq!(revision(&app).0, before.0 + 1, "{name}: one transaction");
        assert_eq!(undo_depth(&app), 1, "{name}: one undo step");
        assert_eq!(entity_count(&app), 4, "{name}: the source is retained");
        assert_eq!(all_lines(&app), expected, "{name}: the grid placement");
    }
}

#[test]
fn the_polar_payload_places_the_copies_around_the_payload_centre() {
    for name in ["ARRAY", "-ARRAY", "ARRAYCLASSIC"] {
        let (mut app, mut session) = one_line_app();
        // 3 items over 180 degrees about (0, -2): the source sits 2 above the
        // centre, so the copies land at 90 and 180 degrees (`fill/(items-1)`).
        set_number(&mut session, ToolParameter::ArrayItems, 3.0);
        set_number(&mut session, ToolParameter::ArrayFillAngle, 180.0);
        let before = revision(&app);
        let outcome = execute(
            &mut app,
            &mut session,
            selector(name),
            CommandPayload::ArrayPolar {
                refs: vec![ref_of(1)],
                center: point(0.0, -2.0),
            },
        )
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(outcome.objects.len(), 2, "{name}: items - 1 copies");
        assert_eq!(revision(&app).0, before.0 + 1, "{name}: one transaction");
        assert_eq!(undo_depth(&app), 1, "{name}: one undo step");
        // The copies are placed at the 90 and 180 degree positions. `arrayRotateItems`
        // keeps its default (false), so each copy is translated only — the
        // declared rule — and its own orientation is unchanged.
        let lines = all_lines(&app);
        assert_eq!(lines.len(), 3, "{name}: the source is retained");
        let translated = sorted(vec![
            line_at(point(0.0, 0.0), point(2.0, 0.0)),
            line_at(point(-2.0, -2.0), point(0.0, -2.0)),
            line_at(point(0.0, -4.0), point(2.0, -4.0)),
        ]);
        let near = |a: (Point3, Point3), b: (Point3, Point3)| {
            (a.0.x - b.0.x).abs() < 1e-9
                && (a.0.y - b.0.y).abs() < 1e-9
                && (a.1.x - b.1.x).abs() < 1e-9
                && (a.1.y - b.1.y).abs() < 1e-9
        };
        assert!(
            lines.len() == translated.len()
                && lines.iter().zip(&translated).all(|(a, b)| near(*a, *b)),
            "{name}: got {lines:?}"
        );
    }
}

#[test]
fn a_bare_array_still_reports_the_missing_payload_and_writes_nothing() {
    for name in ["ARRAY", "-ARRAY", "ARRAYCLASSIC"] {
        let (mut app, mut session) = one_line_app();
        let before = revision(&app);
        let outcome = execute(&mut app, &mut session, selector(name), CommandPayload::None)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(outcome.objects.len(), 0, "{name}");
        assert_eq!(
            outcome.diagnostics[0].code, "array.needs_payload",
            "{name}: {:?}",
            outcome.diagnostics
        );
        assert_eq!(revision(&app), before, "{name}: nothing was written");
        assert_eq!(undo_depth(&app), 0, "{name}: no history");
    }
}

#[test]
fn the_planner_cap_refusal_writes_nothing() {
    // `MAX_ARRAY_ENTITIES` is the planner's own bound; the typed grammar refuses
    // the same request earlier with `array.cap` (see the cad-ui-core tests), and a
    // direct payload caller still hits this one — never a truncated array.
    let (mut app, mut session) = one_line_app();
    let overflow = crate::command_id::array::MAX_ARRAY_ENTITIES as f64 + 2.0;
    set_number(&mut session, ToolParameter::ArrayRows, overflow);
    set_number(&mut session, ToolParameter::ArrayColumns, 2.0);
    set_number(&mut session, ToolParameter::ArrayRowSpacing, 1.0);
    set_number(&mut session, ToolParameter::ArrayColumnSpacing, 1.0);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        selector("ARRAY"),
        CommandPayload::ArrayRect {
            refs: vec![ref_of(1)],
        },
    )
    .expect_err("over the cap");
    let text = format!("{error:?}");
    assert!(text.contains("array cap"), "{text}");
    assert_eq!(revision(&app), before, "nothing was written");
    assert_eq!(undo_depth(&app), 0, "no history");
    assert_eq!(entity_count(&app), 1, "no copy was inserted");
}

#[test]
fn the_router_refuses_an_untransformable_source_without_writing() {
    let (mut app, mut session) = array_app(vec![
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
    ]);
    rect_settings(&mut session, 2.0, 2.0, 5.0, 5.0);
    let before = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        selector("ARRAY"),
        CommandPayload::ArrayRect {
            refs: vec![ref_of(1), ref_of(2)],
        },
    )
    .expect_err("an Opaque entity cannot be translated");
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), before, "nothing was written");
    assert_eq!(undo_depth(&app), 0, "no history");
    assert_eq!(entity_count(&app), 2, "no copy was inserted");
}
