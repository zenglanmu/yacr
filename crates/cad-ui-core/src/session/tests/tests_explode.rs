//! EXPLODE selector and routing contracts (ADR 0016 decision G).
//!
//! EXPLODE runs on the current selection with **no extra prompt** (AutoCAD asks
//! nothing else), so it is a payload-free **direct** command like ERASE: the
//! selector maps to `CommandId::ExplodeEntities` and no `DrawToolKind`/capture
//! state exists.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn explode_selector_is_admitted_once_and_disjoint_from_other_selector_families() {
    // ADR 0016 decision G: EXPLODE is a real `COMMAND_TABLE` selector (no typed
    // parameter and no capture), appearing exactly once and disjoint from the
    // interaction-only selector families.
    let selector = "EXPLODE";
    let id = CommandId::ExplodeEntities;
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
    // An abbreviation *may* target this selector (`X` -> EXPLODE); the alias
    // table's own invariants are contract-tested once in
    // `tests_commands::alias_table`.
}

#[test]
fn explode_routes_to_a_payload_free_selection_command_like_erase() {
    // The selector is a **direct** command (the ERASE precedent): it never starts
    // a draw capture, and an empty selection is an explicit refusal through the
    // application, never a silent success.
    let session = fresh_session();
    assert!(
        session.snapshot(None, None, 0).tool.is_none(),
        "EXPLODE never starts a draw capture"
    );
    let error = session
        .command("EXPLODE")
        .expect_err("an empty EXPLODE is refused");
    assert!(error.contains("non-empty selection"), "{error}");
    assert!(
        session.snapshot(None, None, 0).tool.is_none(),
        "a refused EXPLODE leaves no tool active"
    );
}
