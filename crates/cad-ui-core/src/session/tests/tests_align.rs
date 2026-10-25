//! ALIGN selector, routing and `CadTool.params` projection contracts
//! (ADR 0016 decision F).
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
fn align_selector_is_admitted_once_and_disjoint_from_other_selector_families() {
    // ADR 0016 decision F: ALIGN is a real `COMMAND_TABLE` selector (two point
    // pairs, not a parameter-channel type), appearing exactly once and disjoint
    // from the interaction-only selector families.
    let selector = "ALIGN";
    let id = CommandId::AlignEntities;
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
        "{selector} must not be an alias name"
    );
    // An abbreviation *may* target this selector (`AL` -> ALIGN); the alias
    // table's own invariants (unique names, resolvable targets, no shadowing) are
    // contract-tested once in `tests_commands::alias_table`.
}

#[test]
fn align_selector_routes_to_the_align_draw_capture() {
    // The router starts the ALIGN capture (a real `BeginDraw`), not a no-op. The
    // fresh session has no selection, so the capture is started but not ready.
    let session = fresh_session();
    session.command("ALIGN").expect("ALIGN is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("ALIGN is active");
    assert_eq!(tool.key, "draw.align");
    assert_eq!(tool.points, 0);
    assert!(
        !tool.ready,
        "ALIGN needs a non-empty selection before it can confirm"
    );
}

#[test]
fn cad_tool_params_project_align_scale_only_for_align() {
    // ADR 0016 decision F (frozen frontend wire shape): `alignScale` projects
    // **only** while an ALIGN capture is active and defaults to false; the legacy
    // `sides` field is unchanged (never renamed).
    let align = fresh_session();
    align.command("ALIGN").expect("ALIGN is admitted");
    let tool = align.snapshot(None, None, 0).tool.expect("ALIGN is active");
    assert_eq!(tool.key, "draw.align");
    assert_eq!(
        tool.params.align_scale,
        Some(false),
        "the default answer to the scale prompt is No"
    );
    align
        .set_tool_parameter(ToolParameter::AlignScale, ToolValue::Flag(true))
        .expect("the align-scale flag is admitted");
    assert_eq!(
        align
            .snapshot(None, None, 0)
            .tool
            .expect("ALIGN is active")
            .params
            .align_scale,
        Some(true)
    );

    // A different tool reports `null`, never a carried-over value, and the legacy
    // `sides` field stays absent.
    let line = fresh_session();
    line.command("LINE").expect("LINE is admitted");
    let tool = line.snapshot(None, None, 0).tool.expect("LINE is active");
    assert_eq!(tool.params.align_scale, None);
    assert_eq!(tool.params.sides, None);
}
