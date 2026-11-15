//! `COLOR <aci>` / `COLOR BYLAYER` / `COLOR BYBLOCK` and the `-COLOR` prompt
//! spelling: the typed form of the session current-colour token the create funnel
//! stamps on every entity it inserts.
//!
//! These pin the one grammar (a whole ACI in `0..=256`, or one of the two symbolic
//! words), the one payload (`CommandPayload::CurrentColor`), the two named
//! refusals (`color.invalid_value` for a number outside the domain,
//! `color.invalid_argument` for a word that is not a colour token) and that the
//! bare selectors stay the read-only report.

use super::super::super::*;
use super::super::test_support::*;
use cad_app::{AppMode, CommandPayload, ExtendedCommandId};

use crate::session::commands::{
    parse_color_argument, ARGUMENT_SELECTORS, COLOR_SELECTOR, DASH_COLOR_SELECTOR,
};

/// The live session `COLOR` token (the host controller owns it).
fn color_of(session: &CadSession) -> i16 {
    session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .current_color()
}

fn session_with_fake_files() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn color_admits_one_aci_in_the_closed_domain_and_the_symbolic_words() {
    for (text, expected) in [
        ("0", 0),
        ("1", 1),
        ("255", 255),
        ("256", 256),
        (" 7 ", 7),
        ("BYLAYER", 256),
        ("bylayer", 256),
        ("BYNAME", 256),
        ("BYBLOCK", 0),
        ("byblock", 0),
    ] {
        assert!(
            matches!(
                parse_color_argument(text),
                Ok(CommandPayload::CurrentColor(aci)) if aci == expected
            ),
            "{text:?} must parse to exactly CurrentColor({expected})"
        );
    }
    // Both spellings are argument forms of one selector pair, never new commands.
    assert_eq!(typed_command_argument("COLOR 1"), Some(("COLOR", "1")));
    assert_eq!(typed_command_argument("-COLOR 5"), Some(("-COLOR", "5")));
    assert_eq!(
        typed_command_argument("COLOR"),
        None,
        "bare COLOR is the read-only report"
    );
    assert_eq!(
        typed_command_argument("-COLOR"),
        None,
        "bare -COLOR is the read-only report"
    );
    for selector in [COLOR_SELECTOR, DASH_COLOR_SELECTOR] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
    assert_eq!(
        command_name(CommandId::Extended(ExtendedCommandId::Color)),
        Some("COLOR")
    );
    assert_eq!(
        command_name(CommandId::Extended(ExtendedCommandId::DashColor)),
        Some("-COLOR")
    );
}

#[test]
fn color_refuses_out_of_domain_numbers_with_the_handler_code() {
    for refused in ["257", "300", "65536", "-1", "9999999999999999999999"] {
        let reason = parse_color_argument(refused)
            .err()
            .unwrap_or_else(|| panic!("{refused:?} must be refused"));
        assert!(
            reason.starts_with("color.invalid_value:"),
            "{refused:?} must name the handler's code, got {reason}"
        );
    }
}

#[test]
fn color_refuses_non_colour_words_with_its_own_argument_code() {
    // A colour name needs a name→ACI table this build does not have, and a
    // decimal/exponent/hex spelling is not an ACI either: each is refused by
    // name rather than read as some other value (silently as 0 in particular).
    for refused in [
        "fancy", "RED", "red", "", "1 2", "1.0", "1e3", "0x1", "aci:1",
    ] {
        let reason = parse_color_argument(refused)
            .err()
            .unwrap_or_else(|| panic!("{refused:?} must be refused"));
        assert!(
            reason.starts_with("color.invalid_argument:"),
            "{refused:?} must name the argument code, got {reason}"
        );
    }
}

#[test]
fn color_command_stores_the_one_token_and_bare_color_still_reports() {
    let session = session_with_fake_files();
    assert_eq!(color_of(&session), 256, "ByLayer is the documented default");

    session.command("COLOR 1").expect("COLOR 1 is admitted");
    assert_eq!(color_of(&session), 1, "the typed line stores the ACI");

    session
        .command("COLOR BYLAYER")
        .expect("COLOR BYLAYER is admitted");
    assert_eq!(color_of(&session), 256);

    session
        .command("-COLOR 5")
        .expect("-COLOR is the same token");
    assert_eq!(color_of(&session), 5);

    session
        .command("COLOR BYBLOCK")
        .expect("COLOR BYBLOCK is admitted");
    assert_eq!(color_of(&session), 0);

    // The bare selectors are the report form: they never reset the token.
    session.command("COLOR").expect("bare COLOR reports");
    assert_eq!(color_of(&session), 0);
    session.command("-COLOR").expect("bare -COLOR reports");
    assert_eq!(color_of(&session), 0);
}

#[test]
fn color_refusals_change_nothing_and_record_no_selector() {
    let session = session_with_fake_files();
    session.command("COLOR 4").expect("COLOR 4 is admitted");
    for line in ["COLOR 257", "COLOR -1", "COLOR fancy", "COLOR 1 2"] {
        let error = session
            .command(line)
            .expect_err("an argument outside the grammar never dispatches");
        assert!(
            error.starts_with("color.invalid_value:")
                || error.starts_with("color.invalid_argument:"),
            "{line}: {error}"
        );
        assert_eq!(color_of(&session), 4, "{line} must not move the token");
    }
    // A refused argument is refused *before* dispatch, so the command log keeps
    // only the one admitted line (and never the typed value).
    let echoed: Vec<String> = session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect();
    assert_eq!(
        echoed.iter().filter(|line| *line == "COLOR").count(),
        1,
        "{echoed:?}"
    );
    assert!(!echoed.iter().any(|line| line.contains(' ')), "{echoed:?}");
}

#[test]
fn viewer_refuses_the_write_and_keeps_the_report() {
    let session = session_with_fake_files();
    session
        .core
        .controller
        .borrow_mut()
        .session
        .switch_mode(AppMode::Viewer)
        .expect("switch to Viewer");
    let error = session
        .command("COLOR 1")
        .expect_err("a Viewer session cannot steer new-entity colour");
    assert_eq!(error, "PermissionDenied", "the shared permission refusal");
    assert_eq!(
        color_of(&session),
        256,
        "the refused line never moves the token"
    );
    // The bare selector is the read-only report, which stays Viewer-legal.
    session
        .command("COLOR")
        .expect("the report is Viewer-legal");
    assert_eq!(color_of(&session), 256);
}

#[test]
fn the_host_gate_admits_the_line_and_the_session_owns_the_value() {
    // The shell gate admits the argument form by its base selector and queues the
    // line verbatim; the value is validated by the session (a router refusal),
    // exactly like `ZOOM 0` or `SETVAR PDMODE abc`.
    assert_eq!(
        crate::shell::check_command(true, false, 0, "COLOR 1"),
        Ok("COLOR 1".to_string())
    );
    assert_eq!(
        crate::shell::check_command(true, false, 0, "-COLOR 5"),
        Ok("-COLOR 5".to_string())
    );
    assert_eq!(
        crate::shell::check_command(true, false, 0, "COLOR 257"),
        Ok("COLOR 257".to_string()),
        "the gate never judges the value"
    );
}
