//! Typed-argument and vocabulary contract tests for the 2026-11-07 extras batch
//! (`SKETCH`, `TRACE`, `MOCORO`, `MSTRETCH`, `-PAN`, `VIEWRES`, `VSCURRENT`,
//! `REGEN3`, `LINETYPE`, `-LINETYPE`).

use super::super::super::*;
use cad_app::{CommandPayload, LinetypeCommand, MocoroCommand};

use crate::session::commands::{
    parse_linetype_argument, parse_mocoro_argument, parse_pan_argument, parse_trace_argument,
    parse_viewres_argument, parse_vscurrent_argument, ARGUMENT_SELECTORS, LINETYPE_SELECTOR,
    MOCORO_SELECTOR, PAN_SELECTOR, TRACE_SELECTOR, VIEWRES_SELECTOR, VSCURRENT_SELECTOR,
};

#[test]
fn every_extras_selector_has_a_table_row_and_its_argument_form_is_admitted() {
    assert_eq!(command_name(CommandId::Sketch), Some("SKETCH"));
    assert_eq!(command_name(CommandId::Trace), Some("TRACE"));
    assert_eq!(command_name(CommandId::Mocoro), Some("MOCORO"));
    assert_eq!(command_name(CommandId::MStretch), Some("MSTRETCH"));
    assert_eq!(command_name(CommandId::ViewRes), Some("VIEWRES"));
    assert_eq!(command_name(CommandId::VisualStyle), Some("VSCURRENT"));
    assert_eq!(command_name(CommandId::Linetype), Some("LINETYPE"));
    for selector in [
        PAN_SELECTOR,
        TRACE_SELECTOR,
        MOCORO_SELECTOR,
        VIEWRES_SELECTOR,
        VSCURRENT_SELECTOR,
        LINETYPE_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
}

#[test]
fn regen3_dash_pan_and_dash_linetype_are_aliases_of_the_same_paths() {
    assert_eq!(normalize_command("REGEN3"), Some(CommandId::Regen));
    assert_eq!(normalize_command("-PAN"), Some(CommandId::Pan));
    assert_eq!(normalize_command("-LINETYPE"), Some(CommandId::Linetype));
    assert_eq!(normalize_command("regen3"), Some(CommandId::Regen));
    // An argument after an alias normalizes to its canonical selector.
    assert_eq!(typed_command_argument("-PAN 10 20"), Some(("PAN", "10 20")));
    assert_eq!(
        typed_command_argument("-LINETYPE LIST"),
        Some(("LINETYPE", "LIST"))
    );
}

#[test]
fn pan_accepts_a_delta_or_two_points() {
    let Some(CommandPayload::Points(delta)) = parse_pan_argument("10 -5") else {
        panic!("a two-number delta must parse");
    };
    assert_eq!((delta[0].x, delta[0].y, delta[0].z), (10.0, -5.0, 0.0));
    let Some(CommandPayload::Points(delta)) = parse_pan_argument("1 2 4 6") else {
        panic!("two points must parse to their difference");
    };
    assert_eq!((delta[0].x, delta[0].y), (3.0, 4.0));
    assert!(parse_pan_argument("1").is_none());
    assert!(parse_pan_argument("1 2 3").is_none());
    assert!(parse_pan_argument("1 2 x 4").is_none());
}

#[test]
fn trace_needs_five_numbers_and_carries_its_width() {
    let Some(CommandPayload::Trace { start, end, width }) = parse_trace_argument("0 0 10 0 2.5")
    else {
        panic!("TRACE argument must parse");
    };
    assert_eq!((start.x, start.y), (0.0, 0.0));
    assert_eq!((end.x, end.y), (10.0, 0.0));
    assert_eq!(width, 2.5);
    assert!(parse_trace_argument("0 0 10 0").is_none());
    assert!(parse_trace_argument("0 0 10 0 2 3").is_none());
    // The parser admits any finite decimal; the command refuses width <= 0.
    assert!(parse_trace_argument("0 0 10 0 0").is_some());
}

#[test]
fn mocoro_grammar_covers_the_four_modes() {
    assert!(matches!(
        parse_mocoro_argument("MOVE 1 2"),
        Ok(CommandPayload::Mocoro(MocoroCommand::Move { .. }))
    ));
    assert!(matches!(
        parse_mocoro_argument("copy -1 2.5"),
        Ok(CommandPayload::Mocoro(MocoroCommand::Copy { .. }))
    ));
    assert!(matches!(
        parse_mocoro_argument("ROTATE 0 0 90"),
        Ok(CommandPayload::Mocoro(MocoroCommand::Rotate { degrees, .. })) if degrees == 90.0
    ));
    assert!(matches!(
        parse_mocoro_argument("SCALE 0 0 2"),
        Ok(CommandPayload::Mocoro(MocoroCommand::Scale { factor, .. })) if factor == 2.0
    ));
    assert!(parse_mocoro_argument("SCALE 0 0 0").is_err());
    assert!(parse_mocoro_argument("SCALE 0 0 -1").is_err());
    assert!(parse_mocoro_argument("MOVE 1").is_err());
    assert!(parse_mocoro_argument("EXPLODE 1 2").is_err());
}

#[test]
fn viewres_is_an_integer_percentage_in_range() {
    assert!(matches!(
        parse_viewres_argument("1"),
        Ok(CommandPayload::ViewRes(1))
    ));
    assert!(matches!(
        parse_viewres_argument("1000"),
        Ok(CommandPayload::ViewRes(1000))
    ));
    let low = parse_viewres_argument("0")
        .err()
        .expect("0 is out of range");
    assert!(low.contains("viewres.out_of_range"), "{low}");
    let high = parse_viewres_argument("1001")
        .err()
        .expect("1001 is out of range");
    assert!(high.contains("viewres.out_of_range"), "{high}");
    assert!(parse_viewres_argument("1.5").is_err());
    assert!(parse_viewres_argument("1 2").is_err());
}

#[test]
fn vscurrent_admits_one_word_and_linetype_has_list_and_set_forms() {
    assert!(matches!(
        parse_vscurrent_argument("WIREFRAME"),
        Ok(CommandPayload::VisualStyle(style)) if style == "WIREFRAME"
    ));
    // The command layer is what names the unsupported style (SHADED); the parser
    // only requires one word so that refusal can reach the user.
    assert!(matches!(
        parse_vscurrent_argument("SHADED"),
        Ok(CommandPayload::VisualStyle(style)) if style == "SHADED"
    ));
    assert!(parse_vscurrent_argument("SHADED WITH EDGES").is_err());

    assert!(matches!(
        parse_linetype_argument("LIST"),
        Ok(CommandPayload::Linetype(LinetypeCommand::List))
    ));
    assert!(matches!(
        parse_linetype_argument("SET Center Lines"),
        Ok(CommandPayload::Linetype(LinetypeCommand::Set(name))) if name == "Center Lines"
    ));
    assert!(parse_linetype_argument("SET").is_err());
    assert!(parse_linetype_argument("LIST extra").is_err());
    assert!(parse_linetype_argument("LOAD x").is_err());
}
