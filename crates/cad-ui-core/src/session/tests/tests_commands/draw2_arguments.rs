//! Typed-argument and vocab contract tests for the 2026-11-07 draw/modify
//! batch (`DONUT` … `DRAWORDER`, `-OVERKILL`).

use super::super::super::*;
use cad_app::{Draw2Payload, DrawOrderMode, MlStyleCommand, TrexMode};

use crate::session::commands::{
    parse_donut_argument, parse_draworder_argument, parse_helix_argument, parse_mline_argument,
    parse_mlstyle_argument, parse_mpedit_argument, parse_solid_argument, parse_trex_argument,
    ARGUMENT_SELECTORS, DONUT_SELECTOR, DRAWORDER_SELECTOR, HELIX_SELECTOR, MLINE_SELECTOR,
    MLSTYLE_SELECTOR, MPEDIT_SELECTOR, SOLID_SELECTOR, TREX_SELECTOR,
};

#[test]
fn all_batch_selectors_are_admitted_argument_forms() {
    for selector in [
        DONUT_SELECTOR,
        SOLID_SELECTOR,
        HELIX_SELECTOR,
        MLINE_SELECTOR,
        MLSTYLE_SELECTOR,
        TREX_SELECTOR,
        MPEDIT_SELECTOR,
        DRAWORDER_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument selector"
        );
    }
}

#[test]
fn dash_overkill_is_the_same_command_path() {
    assert_eq!(normalize_command("-OVERKILL"), Some(CommandId::Overkill));
    assert_eq!(command_name(CommandId::Overkill), Some("OVERKILL"));
}

#[test]
fn donut_argument_is_four_numbers() {
    assert_eq!(
        parse_donut_argument("1 2 3 5"),
        Some(Draw2Payload::Donut {
            center: cad_domain::Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0
            },
            inner: 3.0,
            outer: 5.0,
        })
    );
    assert_eq!(parse_donut_argument("1 2 3"), None);
    assert_eq!(parse_donut_argument("1 2 3 abc"), None);
    assert_eq!(parse_donut_argument("1 2 3 5 6"), None);
}

#[test]
fn solid_argument_is_three_or_four_xy_points() {
    assert!(matches!(
        parse_solid_argument("0,0 4,0 0,4"),
        Some(Draw2Payload::Solid { points }) if points.len() == 3
    ));
    assert!(matches!(
        parse_solid_argument("0,0 4,0 4,4 0,4"),
        Some(Draw2Payload::Solid { points }) if points.len() == 4
    ));
    assert_eq!(parse_solid_argument("0,0 4,0"), None);
    assert_eq!(parse_solid_argument("0,0 4,0 0,4 1,1 2,2"), None);
    assert_eq!(parse_solid_argument("0,0 4,0 0"), None);
}

#[test]
fn helix_and_mline_arguments_are_the_documented_token_counts() {
    assert!(matches!(
        parse_helix_argument("1 2 3 2 0"),
        Some(Draw2Payload::Helix { .. })
    ));
    assert_eq!(parse_helix_argument("1 2 3 2"), None);
    assert_eq!(parse_helix_argument("1 2 3 2 0 9"), None);

    assert!(matches!(
        parse_mline_argument("2 0.5 0,0 10,0"),
        Some(Draw2Payload::Mline {
            spacing: Some(2.0),
            offset: 0.5,
            ..
        })
    ));
    assert_eq!(parse_mline_argument("0 0 0,0 10,0"), None);
    assert_eq!(parse_mline_argument("2 0 0,0"), None);
}

#[test]
fn mlstyle_trex_mpedit_and_draworder_words() {
    assert_eq!(
        parse_mlstyle_argument("LIST"),
        Some(Draw2Payload::MlStyle(MlStyleCommand::List))
    );
    assert_eq!(
        parse_mlstyle_argument("SET WALL 2.5"),
        Some(Draw2Payload::MlStyle(MlStyleCommand::Set {
            name: "WALL".into(),
            spacing: 2.5
        }))
    );
    assert_eq!(parse_mlstyle_argument("SAVE X 1"), None);
    assert_eq!(parse_mlstyle_argument("SET WALL 0"), None);

    assert_eq!(
        parse_trex_argument("trim"),
        Some(Draw2Payload::Trex(TrexMode::Trim))
    );
    assert_eq!(
        parse_trex_argument("EXTEND"),
        Some(Draw2Payload::Trex(TrexMode::Extend))
    );
    assert_eq!(parse_trex_argument("BOTH"), None);

    assert_eq!(
        parse_mpedit_argument("c"),
        Some(Draw2Payload::Mpedit(cad_app::PeditOption::Close))
    );
    assert_eq!(parse_mpedit_argument("JOIN"), None);

    for (word, mode) in [
        ("FRONT", DrawOrderMode::Front),
        ("BACK", DrawOrderMode::Back),
        ("ABOVE", DrawOrderMode::Above),
        ("BELOW", DrawOrderMode::Below),
    ] {
        assert_eq!(
            parse_draworder_argument(word),
            Some(Draw2Payload::DrawOrder(mode))
        );
    }
    assert_eq!(parse_draworder_argument("TOP"), None);
}
