//! Typed-argument and vocabulary contracts for the 2026-11-07 dimension-editing
//! batch (`DIMSTYLE`/`-DIMSTYLE`, `DIMTEDIT`, `DIMEDIT`, `DIMOVERRIDE`,
//! `DIMJOGLINE`, and the argument-free `DIMREGEN`/`DIMDISASSOCIATE`/`QDIM`/
//! `DIMBREAK`).

use super::super::super::*;
use cad_app::{DimEditCommand, DimJogLineCommand, DimStyleCommand, DimTextEditCommand};

use crate::session::commands::{
    ARGUMENT_SELECTORS, DIMEDIT_SELECTOR, DIMJOGLINE_SELECTOR, DIMOVERRIDE_SELECTOR,
    DIMSTYLE_SELECTOR, DIMTEDIT_SELECTOR,
};
use crate::session::dim_arguments::{
    parse_dimedit_argument, parse_dimjogline_argument, parse_dimoverride_argument,
    parse_dimstyle_argument, parse_dimtedit_argument,
};

#[test]
fn every_argument_selector_is_admitted() {
    for selector in [
        DIMSTYLE_SELECTOR,
        DIMTEDIT_SELECTOR,
        DIMEDIT_SELECTOR,
        DIMOVERRIDE_SELECTOR,
        DIMJOGLINE_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
}

#[test]
fn dash_dimstyle_is_the_same_command_path() {
    assert_eq!(normalize_command("DIMSTYLE"), Some(CommandId::DimStyle));
    assert_eq!(normalize_command("-DIMSTYLE"), Some(CommandId::DimStyle));
    assert_eq!(command_name(CommandId::DimStyle), Some("DIMSTYLE"));
    assert!(admissible_names().contains(&"-DIMSTYLE"));
}

#[test]
fn dimension_batch_selectors_normalize() {
    for (name, id) in [
        ("DIMSTYLE", CommandId::DimStyle),
        ("DIMTEDIT", CommandId::DimTextEdit),
        ("DIMEDIT", CommandId::DimEdit),
        ("DIMOVERRIDE", CommandId::DimOverride),
        ("DIMREGEN", CommandId::DimRegen),
        ("DIMDISASSOCIATE", CommandId::DimDisassociate),
        ("QDIM", CommandId::QDim),
        ("DIMJOGLINE", CommandId::DimJogLine),
        ("DIMBREAK", CommandId::DimBreak),
    ] {
        assert_eq!(normalize_command(name), Some(id), "{name}");
        assert_eq!(canonical_command_name(name), Some(name), "{name}");
    }
}

#[test]
fn only_the_four_writing_edits_require_work_mode() {
    for id in [
        CommandId::DimTextEdit,
        CommandId::DimEdit,
        CommandId::DimOverride,
        CommandId::QDim,
    ] {
        assert!(id.requires_work_mode(), "{id:?} writes the drawing");
    }
    for id in [
        CommandId::DimStyle,
        CommandId::DimRegen,
        CommandId::DimDisassociate,
        CommandId::DimJogLine,
        CommandId::DimBreak,
    ] {
        assert!(!id.requires_work_mode(), "{id:?} cannot write");
    }
}

#[test]
fn dimstyle_grammar_is_list_current_or_set_name() {
    assert_eq!(parse_dimstyle_argument("LIST"), Some(DimStyleCommand::List));
    assert_eq!(
        parse_dimstyle_argument(" current "),
        Some(DimStyleCommand::Current)
    );
    assert_eq!(
        parse_dimstyle_argument("SET Standard"),
        Some(DimStyleCommand::Set("Standard".into()))
    );
    assert_eq!(
        parse_dimstyle_argument("set My Style"),
        Some(DimStyleCommand::Set("My Style".into()))
    );
    for bad in ["", "LIST extra", "SET", "SET   ", "SHOW"] {
        assert_eq!(parse_dimstyle_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimtedit_grammar_is_move_rotate_or_reset() {
    assert_eq!(
        parse_dimtedit_argument("MOVE 1 2"),
        Some(DimTextEditCommand::Move { x: 1.0, y: 2.0 })
    );
    assert_eq!(
        parse_dimtedit_argument("move -1.5 2.5"),
        Some(DimTextEditCommand::Move { x: -1.5, y: 2.5 })
    );
    assert_eq!(
        parse_dimtedit_argument("ROTATE 30"),
        Some(DimTextEditCommand::Rotate { degrees: 30.0 })
    );
    assert_eq!(
        parse_dimtedit_argument("ROTATE -45.5"),
        Some(DimTextEditCommand::Rotate { degrees: -45.5 })
    );
    assert_eq!(
        parse_dimtedit_argument("RESET"),
        Some(DimTextEditCommand::Reset)
    );
    for bad in ["", "MOVE 1", "MOVE 1 2 3", "ROTATE x", "RESET 1", "HOME"] {
        assert_eq!(parse_dimtedit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimedit_grammar_is_text_override_or_reset() {
    assert_eq!(
        parse_dimedit_argument("TEXT <> mm"),
        Some(DimEditCommand::Text("<> mm".into()))
    );
    assert_eq!(
        parse_dimedit_argument("text done"),
        Some(DimEditCommand::Text("done".into()))
    );
    assert_eq!(parse_dimedit_argument("RESET"), Some(DimEditCommand::Reset));
    for bad in ["", "TEXT", "TEXT   ", "RESET now", "HOME"] {
        assert_eq!(parse_dimedit_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimoverride_grammar_is_variable_plus_verbatim_value() {
    let parsed = parse_dimoverride_argument("DIMTXT 5").expect("DIMTXT");
    assert_eq!(parsed.variable, "DIMTXT");
    assert_eq!(parsed.value, "5");
    let parsed = parse_dimoverride_argument("DIMPOST <> mm").expect("DIMPOST");
    assert_eq!(parsed.variable, "DIMPOST");
    assert_eq!(parsed.value, "<> mm");
    for bad in ["", "DIMTXT", "DIMTXT   "] {
        assert_eq!(parse_dimoverride_argument(bad), None, "{bad:?}");
    }
}

#[test]
fn dimjogline_grammar_is_add_point_or_remove() {
    assert_eq!(
        parse_dimjogline_argument("ADD 1 2"),
        Some(DimJogLineCommand::Add { x: 1.0, y: 2.0 })
    );
    assert_eq!(
        parse_dimjogline_argument("remove"),
        Some(DimJogLineCommand::Remove)
    );
    for bad in ["", "ADD 1", "ADD 1 2 3", "REMOVE 1", "TOGGLE"] {
        assert_eq!(parse_dimjogline_argument(bad), None, "{bad:?}");
    }
}
