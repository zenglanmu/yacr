//! The 2026-11-07 layer batch typed arguments: `LAYERSTATE`, `LAYWALK`,
//! `LTSCALE` and `LAYERPMODE`.
//!
//! These pin the textual grammar and the single-selector rule: each argument form
//! is grammar of its base selector, not a new command name, and an argument
//! outside the grammar is refused by name before any router work.

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn layerstate_has_one_typed_grammar_and_no_second_selector() {
    assert_eq!(
        parse_layerstate_argument("SAVE Plan A"),
        Some(LayerStateOption::Save("Plan A".into()))
    );
    assert_eq!(
        parse_layerstate_argument("restore plan a"),
        Some(LayerStateOption::Restore("plan a".into()))
    );
    assert_eq!(
        parse_layerstate_argument("LIST"),
        Some(LayerStateOption::List)
    );
    assert_eq!(
        parse_layerstate_argument("delete  Old  "),
        Some(LayerStateOption::Delete("Old".into()))
    );
    for refused in [
        "",
        "SAVE",
        "RESTORE",
        "DELETE",
        "LIST extra",
        "BOGUS",
        "SAVE ",
    ] {
        assert_eq!(parse_layerstate_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("LAYERSTATE SAVE Plan A"),
        Some(("LAYERSTATE", "SAVE Plan A"))
    );
    assert_eq!(typed_command_argument("LAYERSTATE"), None);
    assert!(CommandId::LayerState.requires_work_mode());
    assert_eq!(command_name(CommandId::LayerState), Some("LAYERSTATE"));
}

#[test]
fn laywalk_accepts_comma_separated_names_and_refuses_empty_entries() {
    assert_eq!(
        parse_laywalk_argument("WALLS,DIMS"),
        Some(vec!["WALLS".to_string(), "DIMS".to_string()])
    );
    assert_eq!(
        parse_laywalk_argument(" walls , dims "),
        Some(vec!["walls".to_string(), "dims".to_string()])
    );
    for refused in ["", " ", "WALLS,", ",WALLS", "WALLS,,DIMS", ","] {
        assert_eq!(parse_laywalk_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("LAYWALK WALLS,DIMS"),
        Some(("LAYWALK", "WALLS,DIMS"))
    );
    assert_eq!(typed_command_argument("LAYWALK"), None);
    // A view operation: Viewer-legal, like LAYISO.
    assert!(!CommandId::LayWalk.requires_work_mode());
    assert_eq!(command_name(CommandId::LayWalk), Some("LAYWALK"));
}

#[test]
fn ltscale_and_layerpmode_grammars_are_narrow() {
    assert_eq!(parse_ltscale_argument("2.5"), Some(2.5));
    assert_eq!(parse_ltscale_argument(" 1 "), Some(1.0));
    for refused in ["", "0", "-1", "+1", "1e3", "NaN", "2 3"] {
        assert_eq!(parse_ltscale_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("LTSCALE 2.5"),
        Some(("LTSCALE", "2.5"))
    );
    assert_eq!(typed_command_argument("LTSCALE"), None);
    assert!(CommandId::SetLinetypeScale.requires_work_mode());

    assert_eq!(parse_layerpmode_argument("ON"), Some(true));
    assert_eq!(parse_layerpmode_argument("off"), Some(false));
    for refused in ["", "on off", "1", "TRUE"] {
        assert_eq!(parse_layerpmode_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("LAYERPMODE OFF"),
        Some(("LAYERPMODE", "OFF"))
    );
    assert_eq!(
        typed_command_argument("LAYERPMODE"),
        None,
        "bare LAYERPMODE is name-only (it reports)"
    );
    assert!(!CommandId::LayerPmode.requires_work_mode());
}

#[test]
fn the_dash_spellings_are_aliases_not_second_commands() {
    for (alias, id) in [
        ("-LAYMCH", CommandId::LayMCh),
        ("-LAYMRG", CommandId::LayMrg),
        ("-LAYDEL", CommandId::LayDel),
    ] {
        assert_eq!(normalize_command(alias), Some(id), "{alias}");
        assert_eq!(canonical_command_name(alias), command_name(id), "{alias}");
    }
}

#[test]
fn bare_names_and_out_of_grammar_arguments_are_refused_at_the_funnel() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let error = session.command("LAYERSTATE").unwrap_err();
    assert!(error.contains("layerstate.needs_option"), "{error}");
    let error = session.command("LAYERSTATE SAVE").unwrap_err();
    assert!(error.contains("LAYERSTATE needs SAVE"), "{error}");

    let error = session.command("LAYWALK").unwrap_err();
    assert!(error.contains("layers.walk.needs_argument"), "{error}");

    let error = session.command("LTSCALE").unwrap_err();
    assert!(error.contains("ltscale.needs_value"), "{error}");
    let error = session.command("LTSCALE 0").unwrap_err();
    assert!(
        error.contains("LTSCALE needs one positive finite decimal"),
        "{error}"
    );

    let error = session.command("LAYERPMODE MAYBE").unwrap_err();
    assert!(error.contains("LAYERPMODE needs ON or OFF"), "{error}");

    // A bare LAYERPMODE is a successful report, not a refusal.
    session
        .command("LAYERPMODE")
        .expect("bare LAYERPMODE reports");
}
