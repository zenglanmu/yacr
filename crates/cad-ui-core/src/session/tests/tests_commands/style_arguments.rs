//! The named style-table typed lines (`STYLE`, `-STYLE`, `MLEADERSTYLE`).
//!
//! These pin selector admission, the grammar and that a bare selector stays
//! name-only (the read-only report). The semantic write behaviour (revision,
//! undo, name/delete refusals) is pinned in `cad-app::style_tables`.

use cad_app::{StyleTableKind, StyleTableOp};

use super::super::super::commands::{
    invalid_style_argument_reason, parse_mleader_style_argument, parse_text_style_argument,
    typed_command_argument, DASH_STYLE_SELECTOR, MLEADERSTYLE_SELECTOR, STYLE_SELECTOR,
};

#[test]
fn the_three_selectors_are_admitted_argument_forms_and_bare_forms_stay_reports() {
    for selector in [STYLE_SELECTOR, DASH_STYLE_SELECTOR, MLEADERSTYLE_SELECTOR] {
        assert!(
            typed_command_argument(&format!("{selector} ?")).is_some(),
            "{selector} must accept a typed argument"
        );
        assert_eq!(
            typed_command_argument(selector),
            None,
            "the bare {selector} keeps the read-only report path"
        );
    }
    assert_eq!(
        typed_command_argument("STYLE NEW Header"),
        Some((STYLE_SELECTOR, "NEW Header"))
    );
    assert_eq!(
        typed_command_argument("  -style\tRENAME A B "),
        Some((DASH_STYLE_SELECTOR, "RENAME A B"))
    );
    assert_eq!(
        typed_command_argument("MLEADERSTYLE SET Basic"),
        Some((MLEADERSTYLE_SELECTOR, "SET Basic"))
    );
}

#[test]
fn text_style_grammar_covers_every_write_form() {
    let parse = |argument: &str| parse_text_style_argument(argument).map(|command| command.op);
    assert_eq!(parse("?"), Some(StyleTableOp::List));
    assert_eq!(
        parse("NEW Header"),
        Some(StyleTableOp::Create {
            name: "Header".into(),
            from: None
        })
    );
    assert_eq!(
        parse("NEW \"Big Header\" Standard"),
        Some(StyleTableOp::Create {
            name: "Big Header".into(),
            from: Some("Standard".into())
        })
    );
    assert_eq!(
        parse("RENAME A B"),
        Some(StyleTableOp::Rename {
            old: "A".into(),
            new: "B".into()
        })
    );
    assert_eq!(
        parse("DELETE A"),
        Some(StyleTableOp::Delete { name: "A".into() })
    );
    assert_eq!(
        parse("SET A FONT arial.ttf"),
        Some(StyleTableOp::Modify {
            name: "A".into(),
            assignments: vec![("FONT".into(), "arial.ttf".into())]
        })
    );
    assert_eq!(
        parse("SET A KEYS a.ttf b.ttf"),
        None,
        "an odd pair count is outside the grammar"
    );

    for bad in ["SET", "NEW", "RENAME A", "DELETE", "set a font"] {
        assert_eq!(parse(bad), None, "{bad} must be refused");
    }
    let command = parse_text_style_argument("NEW Header").expect("parsed");
    assert_eq!(command.kind, StyleTableKind::TextStyle);
    assert!(invalid_style_argument_reason(STYLE_SELECTOR, "NOPE").contains("NEW <name>"));
}

#[test]
fn mleader_style_grammar_has_a_session_set_and_a_modify_pair_grammar() {
    let parse = |argument: &str| parse_mleader_style_argument(argument).map(|c| c.op);
    assert_eq!(
        parse("SET Basic"),
        Some(StyleTableOp::Set {
            name: "Basic".into()
        })
    );
    assert_eq!(
        parse("MODIFY Basic TEXT_HEIGHT 3.5 PATH_TYPE SPLINE"),
        Some(StyleTableOp::Modify {
            name: "Basic".into(),
            assignments: vec![
                ("TEXT_HEIGHT".into(), "3.5".into()),
                ("PATH_TYPE".into(), "SPLINE".into())
            ]
        })
    );
    assert_eq!(
        parse("MODIFY Basic TEXT_HEIGHT"),
        None,
        "a bare field is outside the pair grammar"
    );
    let command = parse_mleader_style_argument("DELETE Basic").expect("parsed");
    assert_eq!(command.kind, StyleTableKind::MLeaderStyle);
    assert!(invalid_style_argument_reason(MLEADERSTYLE_SELECTOR, "X").contains("MODIFY <name>"));
}
