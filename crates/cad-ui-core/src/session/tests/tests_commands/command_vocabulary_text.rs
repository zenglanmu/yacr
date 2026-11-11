//! The Express-tool text selectors (`TORIENT` / `TCASE` / `TXT2MTXT`) and their
//! typed argument grammars.
//!
//! Split out of `command_vocabulary.rs` (2026-11-12) so both files stay inside
//! the 1000-line code gate; every assertion is unchanged.

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn text_case_and_orientation_selectors_are_work_only_argument_selectors() {
    // The three Express-tool text commands (2026-11-07) are canonical,
    // selection-based selectors — never aliases of TEXT/MTEXT/SCALETEXT.
    for (selector, id) in [
        ("TORIENT", CommandId::Torient),
        ("TCASE", CommandId::TCase),
        ("TXT2MTXT", CommandId::Txt2MText),
    ] {
        assert_eq!(normalize_command(selector), Some(id), "{selector}");
        assert_eq!(
            normalize_command(&selector.to_ascii_lowercase()),
            Some(id),
            "{selector} is case-insensitive"
        );
        assert_eq!(canonical_command_name(selector), Some(selector));
        assert_eq!(command_name(id), Some(selector));
        assert!(admissible_names().contains(&selector), "{selector}");
        assert_eq!(measurement_selector_kind(selector), None, "{selector}");
        assert_eq!(zoom_step_factor(selector), None, "{selector}");
        assert!(
            !COMMAND_ALIASES.iter().any(|(alias, _)| *alias == selector),
            "{selector} must be canonical, not an alias"
        );
        assert!(id.requires_work_mode(), "{selector} must be Work-only");
    }
    // TORIENT/TCASE take one typed argument; the bare TXT2MTXT is name-only.
    assert_eq!(
        typed_command_argument("TORIENT 45"),
        Some(("TORIENT", "45"))
    );
    assert_eq!(
        typed_command_argument("  tcase\tupper  "),
        Some(("TCASE", "upper"))
    );
    assert_eq!(
        typed_command_argument("TCASE"),
        None,
        "the bare form is Title"
    );
    assert_eq!(typed_command_argument("TXT2MTXT"), None);
    // `TCASE` accepts the five modes and their single letters, nothing else.
    for (argument, mode) in [
        ("UPPER", cad_app::TextCaseMode::Upper),
        ("u", cad_app::TextCaseMode::Upper),
        ("LOWER", cad_app::TextCaseMode::Lower),
        ("l", cad_app::TextCaseMode::Lower),
        ("TITLE", cad_app::TextCaseMode::Title),
        ("T", cad_app::TextCaseMode::Title),
        ("SENTENCE", cad_app::TextCaseMode::Sentence),
        ("s", cad_app::TextCaseMode::Sentence),
        ("TOGGLE", cad_app::TextCaseMode::Toggle),
        ("g", cad_app::TextCaseMode::Toggle),
    ] {
        assert_eq!(parse_tcase_argument(argument), Some(mode), "{argument}");
    }
    for refused in ["", "Sentence2", "TITL", "X", "Title case", "UPPER TITLE"] {
        assert_eq!(parse_tcase_argument(refused), None, "{refused:?}");
    }
    // TORIENT accepts one finite signed decimal angle in degrees or REVERSE/R.
    for (argument, expected) in [("45", 45.0), ("22.5", 22.5), ("0", 0.0), ("-90", -90.0)] {
        assert_eq!(
            parse_torient_argument(argument),
            Some(TorientArgument::Angle(expected)),
            "{argument}"
        );
    }
    for refused in ["", "abc", "1e3", ".5", "5.", "45 30", "NaN", "inf", "REV"] {
        assert_eq!(parse_torient_argument(refused), None, "{refused:?}");
    }
    assert!(invalid_tcase_argument_reason("TCASE NOPE").contains("UPPER"));
    assert!(invalid_torient_argument_reason("TORIENT x").contains("degrees"));
    // The bare selectors are real names, so they are refused by the command
    // layer for their own reason (no selection) rather than being unknown.
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    for (selector, code) in [
        ("TORIENT", "torient.needs_selection"),
        ("TCASE", "tcase.needs_selection"),
        ("TXT2MTXT", "txt2mtxt.needs_selection"),
    ] {
        let error = session.command(selector).unwrap_err();
        assert!(error.contains(code), "{selector}: {error}");
    }
}
