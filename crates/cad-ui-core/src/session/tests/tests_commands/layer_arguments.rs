//! The 2026-11-07 layer batch typed arguments: `LAYERSTATE`, `LAYWALK`,
//! `LTSCALE` and `LAYERPMODE`.
//!
//! These pin the textual grammar and the single-selector rule: each argument form
//! is grammar of its base selector, not a new command name, and an argument
//! outside the grammar is refused by name before any router work.

use super::super::super::*;
use super::super::test_support::*;
use crate::session::commands::parse_layerstatesave_argument;

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
fn layfrz_argument_form_admits_only_the_viewport_scope() {
    // The whole-layout spelling: bare `VP` (or `VIEWPORTS`) is an empty list.
    assert_eq!(parse_layfrz_argument("VP"), Some(Vec::new()));
    assert_eq!(parse_layfrz_argument(" vp "), Some(Vec::new()));
    assert_eq!(parse_layfrz_argument("VIEWPORTS"), Some(Vec::new()));
    assert_eq!(parse_layfrz_argument("viewports"), Some(Vec::new()));
    // Bare and bracketed index lists, zero-based like `VPLAYER`'s `[n]` token.
    assert_eq!(parse_layfrz_argument("VP 0,1"), Some(vec![0, 1]));
    assert_eq!(parse_layfrz_argument("vp [0,1]"), Some(vec![0, 1]));
    assert_eq!(parse_layfrz_argument("VP [2]"), Some(vec![2]));
    assert_eq!(parse_layfrz_argument("VIEWPORTS 1"), Some(vec![1]));
    // Duplicates are admitted here and deduped by the application.
    assert_eq!(parse_layfrz_argument("VP 1,1"), Some(vec![1, 1]));
    for refused in [
        "",
        " ",
        "ON",
        "ALL",
        "VP -1",
        "VP +1",
        "VP 1.5",
        "VP 1e3",
        "VP 1,,2",
        "VP 1,",
        "VP ,1",
        "VP []",
        "VP [0,1",
        "VP 0,1]",
        "VP [0],[1]",
        "VP 0 1",
        "VIEWPORT 0",
    ] {
        assert_eq!(parse_layfrz_argument(refused), None, "{refused:?}");
    }

    assert!(ARGUMENT_SELECTORS.contains(&LAYFRZ_SELECTOR));
    assert_eq!(LAYFRZ_SELECTOR, "LAYFRZ");
    assert_eq!(
        typed_command_argument("LAYFRZ VP 0,1"),
        Some(("LAYFRZ", "VP 0,1"))
    );
    assert_eq!(
        typed_command_argument("  layfrz\tvp  "),
        Some(("LAYFRZ", "vp"))
    );
    assert_eq!(
        typed_command_argument("LAYFRZ"),
        None,
        "bare LAYFRZ keeps the drawing-level freeze and is name-only"
    );
    // The form is an argument of the Work-only command, not a second command.
    assert!(CommandId::LayFrz.requires_work_mode());
    assert_eq!(command_name(CommandId::LayFrz), Some("LAYFRZ"));
    assert_eq!(normalize_command("LAYFRZ VP 0"), None);

    // Dispatch: the typed scope reaches the application's own LAYFRZ refusals
    // (this harness has no selection), never the funnel's "unsupported".
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    for line in ["LAYFRZ VP", "LAYFRZ VP 0,1", "LAYFRZ VP [1]"] {
        let error = session.command(line).unwrap_err();
        assert!(error.contains("layfrz.selection"), "{line}: {error}");
    }
    let error = session.command("LAYFRZ VP bogus").unwrap_err();
    assert!(error.contains("LAYFRZ VP"), "{error}");
    assert!(error.contains("unsupported CAD command"), "{error}");

    // End to end: the typed scope reaches the **Viewports** writer. A bare,
    // drawing-level `LAYFRZ` never consults viewports, so
    // `layfrz.paper_space_required` can only come from the typed line having
    // carried the per-viewport payload.
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![cad_domain::SelectionRef {
            document: cad_domain::DocumentId(1),
            entity: cad_domain::EntityId(1),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        }])
        .expect("the demo document's entity 1 is selectable");
    let error = session.command("LAYFRZ VP").unwrap_err();
    assert!(error.contains("layfrz.paper_space_required"), "{error}");
    let error = session.command("LAYFRZ VP 0,1").unwrap_err();
    assert!(error.contains("layfrz.paper_space_required"), "{error}");
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

#[test]
fn layerstatesave_grammar_is_a_name_or_the_list_spelling() {
    assert_eq!(
        parse_layerstatesave_argument("Plan A"),
        Some(LayerStateOption::Save("Plan A".into())),
        "the name is the trimmed remainder and may contain spaces"
    );
    assert_eq!(
        parse_layerstatesave_argument("?"),
        Some(LayerStateOption::List)
    );
    for bad in ["? extra", "?name"] {
        assert_eq!(parse_layerstatesave_argument(bad), None, "{bad:?}");
    }
    // The parser never produces the other two options.
    for name in ["SAVE", "RESTORE x", "DELETE x"] {
        assert_eq!(
            parse_layerstatesave_argument(name),
            Some(LayerStateOption::Save(name.into())),
            "{name:?} is a legal state name, never a second option grammar"
        );
    }
}

#[test]
fn layerstatesave_typed_line_reaches_the_one_store_and_lists() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // The fake host has the session layer-state store; a name saves and `?`
    // lists through the same `LayerStateSave` route `LAYERSTATE` uses.
    session
        .command("LAYERSTATESAVE Plan A")
        .expect("a name saves the current layer state");
    session
        .command("LAYERSTATESAVE ?")
        .expect("? is the read-only list");
    // `?` with trailing text is outside the grammar and changes nothing.
    let error = session.command("LAYERSTATESAVE ? extra").unwrap_err();
    assert!(error.contains("LAYERSTATESAVE needs"), "{error}");
}
