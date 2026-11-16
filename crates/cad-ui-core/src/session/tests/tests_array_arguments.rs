//! `ARRAY RECT …` / `ARRAY POLAR …` (and the `-ARRAY` / `ARRAYCLASSIC`
//! spellings): grammar, exact payload, channel write and the named refusals.
//!
//! The behaviour (copy count, placement, one undo step) is pinned in `cad-app`
//! against the **same** planner; these tests pin what the session decides — which
//! line is admitted, which payload it produces and which values it writes into the
//! one session parameter channel.

use super::super::*;
use super::test_support::*;

use crate::session::commands::{
    array_payload, parse_array_argument, ArrayArgument, ArrayArgumentError, ArrayPolarArgument,
    ArrayRectArgument,
};
use cad_app::CommandPayload;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn selection() -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(1),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn select_one(session: &CadSession) {
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![selection()])
        .expect("the selection");
}

fn logged(session: &CadSession) -> usize {
    session.snapshot(None, None, 0).command_log.len()
}

fn parameter(session: &CadSession, parameter: ToolParameter) -> ToolValue {
    session
        .core
        .controller
        .borrow()
        .session
        .tool_parameter(parameter)
}

/// The numeric value of one tool parameter (the array channel is numeric).
fn parameter_value(session: &CadSession, key: ToolParameter) -> f64 {
    parameter(session, key)
        .number()
        .unwrap_or_else(|| panic!("{key:?} is not a number"))
}

#[test]
fn the_array_grammar_is_the_documented_one() {
    // RECT: the four payload numbers, in the payload's order.
    assert_eq!(
        parse_array_argument("RECT 3 2 10 -5"),
        Ok(ArrayArgument::Rect(ArrayRectArgument {
            rows: 3,
            columns: 2,
            row_spacing: 10.0,
            column_spacing: -5.0,
        }))
    );
    assert_eq!(
        parse_array_argument("rect 1 1 0.5 2.25"),
        Ok(ArrayArgument::Rect(ArrayRectArgument {
            rows: 1,
            columns: 1,
            row_spacing: 0.5,
            column_spacing: 2.25,
        }))
    );
    // POLAR: item count, fill angle (degrees) and the payload's centre.
    assert_eq!(
        parse_array_argument("POLAR 4 90 1.5 -2"),
        Ok(ArrayArgument::Polar(ArrayPolarArgument {
            items: 4,
            fill_angle: 90.0,
            center: [1.5, -2.0],
        }))
    );
    assert_eq!(
        parse_array_argument("polar 6 -180 0 0"),
        Ok(ArrayArgument::Polar(ArrayPolarArgument {
            items: 6,
            fill_angle: -180.0,
            center: [0.0, 0.0],
        }))
    );
    // Zero/negative counts, degenerate spacing, out-of-range angles and the
    // existing cap each have their own code; every other shape is malformed.
    for (refused, expected) in [
        ("", ArrayArgumentError::Malformed),
        ("RECT", ArrayArgumentError::Malformed),
        ("RECT 3 2 10", ArrayArgumentError::Malformed),
        ("RECT 3 2 10 5 1", ArrayArgumentError::Malformed),
        ("R 3 2 10 5", ArrayArgumentError::Malformed),
        ("RECT 0 2 10 5", ArrayArgumentError::Count("rows")),
        ("RECT 3 -2 10 5", ArrayArgumentError::Count("columns")),
        ("RECT 3 2.5 10 5", ArrayArgumentError::Count("columns")),
        ("RECT 3 2 0 5", ArrayArgumentError::Spacing("row-spacing")),
        (
            "RECT 3 2 10 -0",
            ArrayArgumentError::Spacing("column-spacing"),
        ),
        ("RECT 100000 2 1 1", ArrayArgumentError::Cap),
        ("POLAR 0 90 0 0", ArrayArgumentError::Count("items")),
        ("POLAR 4 0 0 0", ArrayArgumentError::Angle),
        ("POLAR 4 360.5 0 0", ArrayArgumentError::Angle),
        ("POLAR 4 -400 0 0", ArrayArgumentError::Angle),
        ("POLAR 100002 90 0 0", ArrayArgumentError::Cap),
        ("POLAR 4 90 0", ArrayArgumentError::Malformed),
    ] {
        assert_eq!(parse_array_argument(refused), Err(expected), "{refused:?}");
    }
}

#[test]
fn every_refusal_answers_with_its_stable_code_and_writes_nothing() {
    for (line, code) in [
        ("ARRAY RECT 0 2 10 5", "array.count"),
        ("ARRAY RECT 3 -2 10 5", "array.count"),
        ("ARRAY RECT 3 2 0 5", "array.spacing"),
        ("ARRAY POLAR 4 0 0 0", "array.angle"),
        ("ARRAY POLAR 4 400 0 0", "array.angle"),
        ("ARRAY RECT 100000 2 1 1", "array.cap"),
        ("ARRAY POLAR 100002 90 0 0", "array.cap"),
        ("ARRAY RECT", "unsupported CAD command"),
        ("ARRAY DIAGONAL 1 2 3 4", "unsupported CAD command"),
    ] {
        let session = fresh_session();
        select_one(&session);
        let error = session.command(line).expect_err("refused");
        assert!(error.starts_with(code), "{line}: {error}");
        assert_eq!(logged(&session), 0, "{line}: nothing was dispatched");
        // The one array channel is untouched by a refusal.
        assert_eq!(
            parameter(&session, ToolParameter::ArrayRows),
            ToolValue::Number(1.0),
            "{line}"
        );
        assert_eq!(
            parameter(&session, ToolParameter::ArrayColumnSpacing),
            ToolValue::Number(0.0),
            "{line}"
        );
    }
}

#[test]
fn an_empty_selection_is_refused_before_the_channel_is_written() {
    let session = fresh_session();
    let error = session
        .command("ARRAY RECT 3 2 10 5")
        .expect_err("no source object");
    assert!(error.starts_with("array.selection:"), "{error}");
    assert_eq!(logged(&session), 0, "nothing was dispatched");
    assert_eq!(
        parameter(&session, ToolParameter::ArrayRows),
        ToolValue::Number(1.0),
        "the refusal wrote no channel value"
    );
}

#[test]
fn the_typed_line_dispatches_the_exact_payload() {
    let refs = vec![selection()];
    // RECT: the payload carries the refs only (the numbers are the channel).
    let payload = array_payload(
        ArrayArgument::Rect(ArrayRectArgument {
            rows: 3,
            columns: 2,
            row_spacing: 10.0,
            column_spacing: 5.0,
        }),
        refs.clone(),
    );
    let CommandPayload::ArrayRect { refs: sent } = payload else {
        panic!("RECT must dispatch the ArrayRect payload");
    };
    assert_eq!(sent, refs);
    // POLAR: the payload carries the refs and the parsed centre (flat, z = 0).
    let payload = array_payload(
        ArrayArgument::Polar(ArrayPolarArgument {
            items: 4,
            fill_angle: 90.0,
            center: [1.5, -2.0],
        }),
        refs.clone(),
    );
    let CommandPayload::ArrayPolar { refs: sent, center } = payload else {
        panic!("POLAR must dispatch the ArrayPolar payload");
    };
    assert_eq!(sent, refs);
    assert_eq!(center.x, 1.5);
    assert_eq!(center.y, -2.0);
    assert_eq!(center.z, 0.0);
}

#[test]
fn the_typed_rect_line_writes_the_session_channel_and_dispatches() {
    let session = fresh_session();
    select_one(&session);
    // The fixture's model space holds one LINE, so the planner really runs.
    session
        .command("ARRAY RECT 2 3 10 5")
        .expect("the fixture line arrays");
    assert!(logged(&session) > 0, "the selector was echoed");
    for (parameter, expected) in [
        (ToolParameter::ArrayRows, 2.0),
        (ToolParameter::ArrayColumns, 3.0),
        (ToolParameter::ArrayRowSpacing, 10.0),
        (ToolParameter::ArrayColumnSpacing, 5.0),
    ] {
        assert_eq!(
            parameter_value(&session, parameter),
            expected,
            "{parameter:?}"
        );
    }
    // The polar selectors of the same channel are untouched.
    assert_eq!(parameter_value(&session, ToolParameter::ArrayItems), 1.0);
    assert_eq!(
        parameter_value(&session, ToolParameter::ArrayFillAngle),
        360.0
    );
}

#[test]
fn the_typed_polar_line_writes_the_session_channel_and_dispatches() {
    let session = fresh_session();
    select_one(&session);
    session
        .command("ARRAY POLAR 4 90 5 5")
        .expect("the fixture line arrays around (5, 5)");
    assert!(logged(&session) > 0, "the selector was echoed");
    assert_eq!(parameter_value(&session, ToolParameter::ArrayItems), 4.0);
    assert_eq!(
        parameter_value(&session, ToolParameter::ArrayFillAngle),
        90.0
    );
    // The rectangular selectors of the same channel are untouched.
    assert_eq!(parameter_value(&session, ToolParameter::ArrayRows), 1.0);
    assert_eq!(parameter_value(&session, ToolParameter::ArrayColumns), 1.0);
}

#[test]
fn dash_array_and_arrayclassic_share_the_one_implementation() {
    for (selector, line) in [
        ("-ARRAY", "-ARRAY RECT 2 2 10 5"),
        ("ARRAYCLASSIC", "ARRAYCLASSIC RECT 2 2 10 5"),
        ("ARRAY", "array rect 2 2 10 5"),
    ] {
        let session = fresh_session();
        select_one(&session);
        session.command(line).expect("the typed line is admitted");
        assert!(logged(&session) > 0, "{selector}: the selector was echoed");
        assert_eq!(
            parameter_value(&session, ToolParameter::ArrayRows),
            2.0,
            "{selector}"
        );
        assert_eq!(
            parameter_value(&session, ToolParameter::ArrayRowSpacing),
            10.0,
            "{selector}"
        );
    }
    // `ARRAY POLAR` through the dash spelling too.
    let session = fresh_session();
    select_one(&session);
    session
        .command("-ARRAY POLAR 3 120 5 5")
        .expect("the typed polar line is admitted");
    assert_eq!(parameter_value(&session, ToolParameter::ArrayItems), 3.0);
}

#[test]
fn a_viewer_session_refuses_the_typed_line_through_the_channel() {
    let session = fresh_session();
    select_one(&session);
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");
    let error = session
        .command("ARRAY RECT 2 2 10 5")
        .expect_err("Viewer must refuse");
    assert!(
        !error.contains("array."),
        "the channel's own refusal: {error}"
    );
    assert_eq!(logged(&session), 0, "nothing was dispatched");
    assert_eq!(
        parameter_value(&session, ToolParameter::ArrayRows),
        1.0,
        "a Viewer refusal writes no channel value"
    );
}
