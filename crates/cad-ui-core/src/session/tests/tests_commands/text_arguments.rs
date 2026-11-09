//! Typed-argument contracts for `TORIENT`: the numeric angle form is unchanged
//! and the `REVERSE`/`R` words dispatch `CommandPayload::TorientReverse`.

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn torient_reverse_words_carry_the_reverse_payload() {
    for argument in ["REVERSE", "reverse", "ReVeRsE", "R", "r"] {
        assert_eq!(
            parse_torient_argument(argument),
            Some(TorientArgument::Reverse),
            "{argument}"
        );
    }
    for (argument, expected) in [("45", 45.0), ("22.5", 22.5), ("0", 0.0), ("-90", -90.0)] {
        assert_eq!(
            parse_torient_argument(argument),
            Some(TorientArgument::Angle(expected)),
            "{argument}"
        );
    }
    for refused in [
        "",
        "REV",
        "REVERSE 45",
        "R 1",
        "abc",
        "1e3",
        ".5",
        "5.",
        "45 30",
        "NaN",
        "inf",
    ] {
        assert_eq!(parse_torient_argument(refused), None, "{refused:?}");
    }
}

#[test]
fn torient_refusal_names_both_the_angle_and_reverse_forms() {
    let reason = invalid_torient_argument_reason("TORIENT x");
    assert!(reason.contains("degrees"), "{reason}");
    assert!(reason.contains("REVERSE"), "{reason}");
    assert!(reason.contains("Most Readable"), "{reason}");
}

#[test]
fn typed_torient_reverse_and_r_reach_the_application() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    for line in ["TORIENT REVERSE", "TORIENT R", "torient reverse"] {
        let error = session.command(line).unwrap_err();
        assert!(error.contains("torient.needs_selection"), "{line}: {error}");
    }
    // The numeric form keeps its existing absolute-angle path.
    let error = session.command("TORIENT 45").unwrap_err();
    assert!(
        error.contains("torient.needs_selection"),
        "TORIENT 45: {error}"
    );
}

#[test]
fn typed_torient_bad_word_is_refused_by_name_and_changes_nothing() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("TORIENT x").unwrap_err();
    assert!(
        error.contains("TORIENT needs one finite decimal angle"),
        "{error}"
    );
    assert!(error.contains("REVERSE"), "{error}");
    assert!(
        session.snapshot(None, None, 0).command_log.is_empty(),
        "the refusal happens before the command is recorded"
    );
}
