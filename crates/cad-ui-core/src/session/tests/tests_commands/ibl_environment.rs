//! `IBL` (PBR environment lighting) router and status-projection contracts.
//!
//! The token itself lives in `cad-app`
//! (`SessionState::view_extras.pbr_environment`); these tests pin the session
//! side: the bare selector toggles, `IBL ON|OFF`/`1|0` set it, an out-of-grammar
//! value is refused by name, and `cad_status.iblEnabled` projects the token.
//! Session state only: a refused or successful line writes no drawing, revision
//! or history entry.

use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn ibl_defaults_to_image_based_and_a_bare_selector_toggles_it() {
    let session = fresh_session();
    let (document, before) = {
        let c = session.core.controller.borrow();
        (c.document_id, c.drawing().unwrap())
    };

    assert!(
        session.snapshot(None, None, 0).ibl_enabled,
        "IBL image-based lighting is the session default"
    );

    // The bare selector flips the switch; it is case-insensitive.
    session.command("IBL").expect("bare IBL toggles");
    assert!(!session.snapshot(None, None, 0).ibl_enabled);
    assert_eq!(
        session
            .snapshot(None, None, 0)
            .command_log
            .last()
            .and_then(|entry| entry.command.as_deref()),
        Some("IBL"),
        "the canonical selector is echoed, never the typed value"
    );
    session.command("ibl").expect("IBL is case-insensitive");
    assert!(session.snapshot(None, None, 0).ibl_enabled);

    // Session/display state only: no revision bump, no entity change, no undo.
    let after = session.core.controller.borrow().drawing().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.entity_count(), before.entity_count());
    {
        let c = session.core.controller.borrow();
        assert!(!c.application.can_undo(&document));
        assert!(!c.application.can_redo(&document));
    }
    assert!(!CommandId::Ibl.requires_work_mode());
}

#[test]
fn ibl_on_off_and_numeric_spellings_set_the_projected_flag() {
    let session = fresh_session();

    for typed in ["IBL OFF", "ibl off", "IBL 0"] {
        session
            .command(typed)
            .unwrap_or_else(|error| panic!("{typed:?} refused: {error}"));
        assert!(
            !session.snapshot(None, None, 0).ibl_enabled,
            "{typed:?} must project `iblEnabled: false`"
        );
    }
    for typed in ["IBL ON", "ibl on", "IBL 1"] {
        session
            .command(typed)
            .unwrap_or_else(|error| panic!("{typed:?} refused: {error}"));
        assert!(
            session.snapshot(None, None, 0).ibl_enabled,
            "{typed:?} must project `iblEnabled: true`"
        );
    }
}

#[test]
fn an_out_of_grammar_ibl_value_is_refused_by_name_and_changes_nothing() {
    let session = fresh_session();
    session.command("IBL OFF").expect("OFF is admitted");

    for typed in ["IBL maybe", "IBL 2", "IBL TRUE", "IBL ON OFF"] {
        let error = session
            .command(typed)
            .expect_err(&format!("{typed:?} must be refused"));
        assert!(
            error.contains("ibl.invalid_value"),
            "{typed:?} must be refused by name, got {error:?}"
        );
        assert!(
            !session.snapshot(None, None, 0).ibl_enabled,
            "{typed:?} must leave the stored token untouched (OFF)"
        );
        // The refusal is recorded for `cad_status.error`, never a silent ignore.
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str())
        );
    }
}
