//! HATCH selector, routing and `CadTool.params` projection contracts
//! (ADR 0018 decision D, P6b).
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
fn the_hatch_selector_is_admitted_once_and_disjoint_from_other_selector_families() {
    let selector = "HATCH";
    assert_eq!(normalize_command(selector), Some(CommandId::Hatch));
    assert_eq!(canonical_command_name(selector), Some(selector));
    assert_eq!(
        COMMAND_TABLE
            .iter()
            .filter(|(_, mapped)| *mapped == CommandId::Hatch)
            .count(),
        1,
        "HATCH must appear exactly once in COMMAND_TABLE"
    );
    assert!(admissible_names().contains(&selector));
    assert_eq!(measurement_selector_kind(selector), None);
    assert!(zoom_step_factor(selector).is_none());
    assert!(!COMMAND_ALIASES.iter().any(|(alias, _)| *alias == selector));
    // An abbreviation *may* target this selector (`H` -> HATCH); the alias table's
    // own invariants are contract-tested once in `tests_commands::alias_table`.
}

#[test]
fn the_hatch_selector_starts_the_zero_point_pre_selection_tool() {
    let session = fresh_session();
    session.command("HATCH").expect("HATCH is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("HATCH is active");
    assert_eq!(tool.key, "draw.hatch");
    assert_eq!(tool.points, 0, "HATCH captures no points");
    // No selection yet: the pre-selection boundary is missing, so the tool is
    // honestly not ready and reports no fabricated point count.
    assert!(!tool.ready);
    assert_eq!(tool.params.hatch_pattern_type, Some(0.0));
    assert_eq!(tool.params.hatch_spacing, Some(1.0));
    assert_eq!(tool.params.hatch_angle, Some(0.0));
    assert_eq!(tool.params.hatch_double, Some(false));
    assert_eq!(tool.params.hatch_style, Some(0.0));
}

#[test]
fn cad_tool_params_project_hatch_values_only_for_the_hatch_tool() {
    let hatch = fresh_session();
    hatch.command("HATCH").expect("HATCH is admitted");
    // The typed channel is the authority: the session validates and stores.
    hatch
        .set_tool_parameter(
            ToolParameter::HatchPatternType,
            ToolValue::Number(cad_app::HATCH_PATTERN_TYPE_USER_DEFINED),
        )
        .expect("the user-defined type is admitted");
    hatch
        .set_tool_parameter(ToolParameter::HatchSpacing, ToolValue::Number(2.5))
        .expect("a positive spacing is admitted");
    hatch
        .set_tool_parameter(ToolParameter::HatchAngle, ToolValue::Number(45.0))
        .expect("a finite angle is admitted");
    hatch
        .set_tool_parameter(ToolParameter::HatchDouble, ToolValue::Flag(true))
        .expect("the double flag is admitted");
    hatch
        .set_tool_parameter(
            ToolParameter::HatchStyle,
            ToolValue::Number(cad_app::HATCH_STYLE_IGNORE),
        )
        .expect("the Ignore style is admitted");
    let tool = hatch.snapshot(None, None, 0).tool.expect("HATCH is active");
    assert_eq!(tool.params.hatch_pattern_type, Some(1.0));
    assert_eq!(tool.params.hatch_spacing, Some(2.5));
    assert_eq!(tool.params.hatch_angle, Some(45.0));
    assert_eq!(tool.params.hatch_double, Some(true));
    assert_eq!(tool.params.hatch_style, Some(2.0));

    // Every other tool reports none of the hatch parameters (never a carried-over
    // value), and HATCH reports none of theirs.
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.hatch_pattern_type, None);
    assert_eq!(tool.params.hatch_spacing, None);
    assert_eq!(tool.params.hatch_angle, None);
    assert_eq!(tool.params.hatch_double, None);
    assert_eq!(tool.params.hatch_style, None);
    assert_eq!(tool.params.array_rows, None);
    assert_eq!(tool.params.align_scale, None);
}

#[test]
fn a_hatch_parameter_set_is_refused_in_viewer_mode() {
    let session = fresh_session();
    session.command("HATCH").expect("HATCH is admitted");
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");
    let error = session
        .set_tool_parameter(ToolParameter::HatchSpacing, ToolValue::Number(2.0))
        .expect_err("a Viewer-mode set is refused");
    assert!(
        !error.is_empty(),
        "an explicit refusal, not an empty success"
    );
    // The refusal left the stored default untouched.
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .tool_parameter(ToolParameter::HatchSpacing),
        ToolValue::Number(1.0)
    );
}
