//! ARRAYRECT/ARRAYPOLAR selector, routing and `CadTool.params` projection
//! contracts (ADR 0016 decision D).
//!
//! Split out of `tests_tools.rs` to keep that file under the code-size limit.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn array_selectors_are_admitted_once_and_disjoint_from_other_selector_families() {
    // ADR 0016 decision D: ARRAYRECT/ARRAYPOLAR are two real `COMMAND_TABLE`
    // selectors (not a parameter-channel "type" prompt), each appearing exactly
    // once and disjoint from the interaction-only selector families.
    for (selector, id) in [
        ("ARRAYRECT", CommandId::ArrayRectangular),
        ("ARRAYPOLAR", CommandId::ArrayPolar),
    ] {
        assert_eq!(normalize_command(selector), Some(id), "{selector}");
        assert_eq!(
            canonical_command_name(selector),
            Some(selector),
            "{selector}"
        );
        assert_eq!(
            COMMAND_TABLE
                .iter()
                .filter(|(_, mapped)| *mapped == id)
                .count(),
            1,
            "{selector} must appear exactly once in COMMAND_TABLE"
        );
        assert!(admissible_names().contains(&selector), "{selector}");
        assert_eq!(measurement_selector_kind(selector), None, "{selector}");
        assert!(zoom_step_factor(selector).is_none(), "{selector}");
        assert!(
            !COMMAND_ALIASES.iter().any(|(alias, _)| *alias == selector),
            "{selector} must not be an alias"
        );
        assert!(
            !COMMAND_ALIASES
                .iter()
                .any(|(_, canonical)| *canonical == selector),
            "{selector} must not be an alias target"
        );
    }
}

#[test]
fn cad_tool_params_project_array_values_only_for_their_tools() {
    // ADR 0016 decision D (frozen frontend wire shape): the four ARRAYRECT grid
    // parameters and the three ARRAYPOLAR parameters each project **only** while
    // their own tool is active; the legacy `sides` field is unchanged (never
    // renamed).
    let rect = fresh_session();
    rect.command("ARRAYRECT").expect("ARRAYRECT is admitted");
    let tool = rect
        .snapshot(None, None, 0)
        .tool
        .expect("ARRAYRECT is active");
    assert_eq!(tool.key, "draw.arrayrect");
    assert_eq!(tool.points, 0, "ARRAYRECT captures no points");
    assert_eq!(tool.params.array_rows, Some(1.0));
    assert_eq!(tool.params.array_columns, Some(1.0));
    assert_eq!(tool.params.array_row_spacing, Some(0.0));
    assert_eq!(tool.params.array_column_spacing, Some(0.0));
    // The ARRAYPOLAR parameters are absent while ARRAYRECT is active.
    assert_eq!(tool.params.array_items, None);
    assert_eq!(tool.params.array_fill_angle, None);
    assert_eq!(tool.params.array_rotate_items, None);
    rect.set_tool_parameter(ToolParameter::ArrayRows, ToolValue::Number(3.0))
        .expect("a valid row count is admitted");
    rect.set_tool_parameter(ToolParameter::ArrayColumnSpacing, ToolValue::Number(2.5))
        .expect("a valid spacing is admitted");
    let tool = rect
        .snapshot(None, None, 0)
        .tool
        .expect("ARRAYRECT is active");
    assert_eq!(tool.params.array_rows, Some(3.0));
    assert_eq!(tool.params.array_column_spacing, Some(2.5));

    let polar = fresh_session();
    polar.command("ARRAYPOLAR").expect("ARRAYPOLAR is admitted");
    let tool = polar
        .snapshot(None, None, 0)
        .tool
        .expect("ARRAYPOLAR is active");
    assert_eq!(tool.key, "draw.arraypolar");
    assert_eq!(tool.params.array_items, Some(1.0));
    assert_eq!(
        tool.params.array_fill_angle,
        Some(360.0),
        "the full-circle default is projected"
    );
    assert_eq!(tool.params.array_rotate_items, Some(false));
    // The ARRAYRECT parameters are absent while ARRAYPOLAR is active.
    assert_eq!(tool.params.array_rows, None);
    assert_eq!(tool.params.array_column_spacing, None);
    polar
        .set_tool_parameter(ToolParameter::ArrayItems, ToolValue::Number(6.0))
        .expect("a valid item count is admitted");
    polar
        .set_tool_parameter(ToolParameter::ArrayFillAngle, ToolValue::Number(180.0))
        .expect("a partial fill angle is admitted");
    polar
        .set_tool_parameter(ToolParameter::ArrayRotateItems, ToolValue::Flag(true))
        .expect("the rotate flag is admitted");
    let tool = polar
        .snapshot(None, None, 0)
        .tool
        .expect("ARRAYPOLAR is active");
    assert_eq!(tool.params.array_items, Some(6.0));
    assert_eq!(tool.params.array_fill_angle, Some(180.0));
    assert_eq!(tool.params.array_rotate_items, Some(true));

    // A different tool reports none of them, never a carried-over value, and the
    // legacy `sides` field stays absent.
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.array_rows, None);
    assert_eq!(tool.params.array_columns, None);
    assert_eq!(tool.params.array_row_spacing, None);
    assert_eq!(tool.params.array_column_spacing, None);
    assert_eq!(tool.params.array_items, None);
    assert_eq!(tool.params.array_fill_angle, None);
    assert_eq!(tool.params.array_rotate_items, None);
    assert_eq!(tool.params.sides, None);
}
