//! The typed `XCLIP` grammar and its dispatch.
//!
//! These pin the textual grammar (`RECT`/`POLY`/`ON`/`OFF`/`DELETE`/`INVERT`/`?`),
//! the admission of `XCLIP` as an argument form of the real extended command
//! (it is no longer a named refusal), and the payload path: a valid line with no
//! selection reaches the command layer and is refused there by name
//! (`xclip.no_selection`), which proves the mapping without duplicating
//! `cad-app`'s semantics. A malformed line is refused before dispatch.

use super::super::super::*;
use super::super::test_support::*;

use crate::session::commands::{parse_xclip_argument, ARGUMENT_SELECTORS, XCLIP_SELECTOR};
use cad_app::{ExtendedCommandId, RefusalId, XClipCommand};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn xclip_argument_is_a_block_local_boundary_or_a_switch_or_a_report() {
    assert_eq!(
        parse_xclip_argument("RECT 0 0 10 5"),
        Some(XClipCommand::Rect([0.0, 0.0, 10.0, 5.0]))
    );
    assert_eq!(
        parse_xclip_argument("rect -1.5 -2 3 4"),
        Some(XClipCommand::Rect([-1.5, -2.0, 3.0, 4.0])),
        "the keywords are case-insensitive and the corners are signed decimals"
    );
    assert_eq!(parse_xclip_argument("ON"), Some(XClipCommand::On));
    assert_eq!(parse_xclip_argument("off"), Some(XClipCommand::Off));
    assert_eq!(parse_xclip_argument("DELETE"), Some(XClipCommand::Delete));
    assert_eq!(parse_xclip_argument("invert"), Some(XClipCommand::Invert));
    assert_eq!(parse_xclip_argument("?"), Some(XClipCommand::Report));
    assert_eq!(
        parse_xclip_argument("POLY 0 0 10 0 10 5"),
        Some(XClipCommand::Polygon(vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 5.0],
        ]))
    );
    // Ordering and polygon validity are the command's named refusals, not
    // syntax: any four finite numbers or three point pairs are admitted here.
    assert_eq!(
        parse_xclip_argument("RECT 10 0 0 10"),
        Some(XClipCommand::Rect([10.0, 0.0, 0.0, 10.0]))
    );
    for refused in [
        "",
        "RECT",
        "RECT 0 0 1",
        "RECT 0 0 1 1 1",
        "RECT a 0 1 1",
        "POLY 0 0 10 0",
        "POLY 0 0 10 0 10",
        "POLY 0 0 10 0 x 5",
        "ON EXTRA",
        "? EXTRA",
        "NONE",
        "REC 0 0 1 1",
    ] {
        assert_eq!(parse_xclip_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn xclip_is_an_argument_selector_of_the_real_extended_command() {
    assert!(ARGUMENT_SELECTORS.contains(&XCLIP_SELECTOR));
    assert_eq!(
        normalize_command("XCLIP"),
        Some(CommandId::Extended(ExtendedCommandId::Xclip)),
        "XCLIP is a real command, not the named refusal it used to be"
    );
    assert_eq!(canonical_command_name("XCLIP"), Some("XCLIP"));
    assert!(ExtendedCommandId::ALL.contains(&ExtendedCommandId::Xclip));
    assert!(
        ExtendedCommandId::Xclip.requires_work_mode(),
        "the clip write is Work-only"
    );
    // The refusal registry no longer owns the name, and the count dropped by one.
    assert!(!RefusalId::ALL.iter().any(|name| name.selector() == "XCLIP"));
    assert_eq!(RefusalId::ALL.len(), 322);
}

#[test]
fn typed_xclip_reaches_the_command_layer_and_refuses_no_selection() {
    let session = fresh_session();
    for line in [
        "XCLIP RECT 0 0 10 5",
        "XCLIP POLY 0 0 10 0 10 5",
        "XCLIP ON",
        "XCLIP ?",
    ] {
        let error = session.command(line).expect_err(line);
        assert!(
            error.contains("xclip.no_selection"),
            "the payload reached the command layer: {line}: {error}"
        );
    }
    // A bare selector is admitted by name and refused by the command: the
    // grammar never defaults a boundary.
    let error = session.command("XCLIP").expect_err("bare XCLIP");
    assert!(error.contains("xclip.payload"), "{error}");
}

#[test]
fn malformed_xclip_arguments_are_refused_before_dispatch() {
    let session = fresh_session();
    for line in [
        "XCLIP RECT 0 0 1",
        "XCLIP RECT 0 0 1 1 1",
        "XCLIP POLY 0 0 10 0",
        "XCLIP NONE",
    ] {
        let error = session.command(line).expect_err(line);
        assert!(error.starts_with("xclip.argument"), "{line}: {error}");
        assert!(error.contains("nothing was changed"), "{line}: {error}");
    }
}
