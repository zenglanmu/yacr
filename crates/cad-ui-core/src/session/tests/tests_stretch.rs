//! STRETCH selector, routing and `CadTool` projection contracts
//! (ADR 0016 decision E).
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
fn stretch_selector_is_admitted_once_and_disjoint_from_other_selector_families() {
    // ADR 0016 decision E: STRETCH is a real `COMMAND_TABLE` selector (four free
    // points, not a parameter-channel type), appearing exactly once and disjoint
    // from the interaction-only selector families.
    let selector = "STRETCH";
    let id = CommandId::StretchEntities;
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

#[test]
fn stretch_selector_routes_to_the_stretch_draw_capture() {
    // The router starts the STRETCH capture (a real `BeginDraw`), not a no-op.
    // The window itself defines the set, so no selection is required; the capture
    // is simply not ready until the four points are captured.
    let session = fresh_session();
    session.command("STRETCH").expect("STRETCH is admitted");
    let tool = session
        .snapshot(None, None, 0)
        .tool
        .expect("STRETCH is active");
    assert_eq!(tool.key, "draw.stretch");
    assert_eq!(tool.points, 0);
    assert!(
        !tool.ready,
        "STRETCH needs four captured points before it can confirm"
    );
    assert!(
        !tool.open_ended,
        "STRETCH is a fixed four-point capture, never open-ended"
    );
    // STRETCH has no typed tool parameter, so every `params` field stays `null`.
    assert_eq!(tool.params.sides, None);
    assert_eq!(tool.params.align_scale, None);
    assert_eq!(tool.params.fillet_radius, None);
}
