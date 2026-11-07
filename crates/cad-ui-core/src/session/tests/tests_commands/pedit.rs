//! `PEDIT` (AutoCAD 2026) vocabulary: the declared v1 option subset.
//!
//! Pins the typed-argument grammar (`CLOSE`/`OPEN`/`REVERSE` and their single
//! letters), the stable invalid-argument refusal, the Work-only capability and
//! the one-canonical-selector rule (the option words are grammar, not names).

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn pedit_is_the_declared_work_only_v1_option_subset() {
    // The one typed argument grammar: three options and their single letters.
    for (argument, option) in [
        ("CLOSE", cad_app::PeditOption::Close),
        ("close", cad_app::PeditOption::Close),
        ("C", cad_app::PeditOption::Close),
        ("c", cad_app::PeditOption::Close),
        ("OPEN", cad_app::PeditOption::Open),
        ("o", cad_app::PeditOption::Open),
        ("REVERSE", cad_app::PeditOption::Reverse),
        ("r", cad_app::PeditOption::Reverse),
        ("  reverse  ", cad_app::PeditOption::Reverse),
    ] {
        assert_eq!(parse_pedit_argument(argument), Some(option), "{argument}");
    }
    // No other AutoCAD option, option stack, partial spelling or empty argument.
    for refused in [
        "", "JOIN", "WIDTH", "FIT", "SPLINE", "C O", "REV", "MULTIPLE",
    ] {
        assert_eq!(parse_pedit_argument(refused), None, "{refused:?}");
    }
    // The refusal names the implemented set, so an unimplemented AutoCAD option
    // is refused by name and never silently defaulted.
    let reason = invalid_pedit_argument_reason("PEDIT JOIN");
    assert!(reason.contains("CLOSE, OPEN or REVERSE"), "{reason}");
    assert!(reason.contains("NOT implemented"), "{reason}");
    // It is a typed argument of one canonical selector, not a new command name.
    assert_eq!(
        typed_command_argument("PEDIT CLOSE"),
        Some(("PEDIT", "CLOSE"))
    );
    assert_eq!(
        typed_command_argument("PEDIT"),
        None,
        "bare PEDIT is name-only"
    );
    assert!(CommandId::Pedit.requires_work_mode());
    assert_eq!(command_name(CommandId::Pedit), Some("PEDIT"));

    // A bare `PEDIT` is admitted by name and refused for its missing option; a
    // bad argument never reaches the command layer; a good one reaches it and
    // reports the absent selection (there is no drawing in this fixture).
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("PEDIT").unwrap_err();
    assert!(error.contains("pedit.needs_option"), "{error}");
    let error = session.command("PEDIT JOIN").unwrap_err();
    assert!(
        error.contains("PEDIT needs CLOSE, OPEN or REVERSE"),
        "{error}"
    );
    let error = session.command("PEDIT CLOSE").unwrap_err();
    assert!(error.contains("pedit.selection"), "{error}");
}
