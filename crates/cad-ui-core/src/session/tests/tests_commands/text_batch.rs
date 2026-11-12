//! The 2026-11-07 2D text batch vocabulary: the five typed-argument grammars,
//! their stable refusals, the Work-only capability and the three aliases.
//!
//! This pins the *routing* half only: the write paths live in `cad-app` and are
//! contract-tested there (`crates/cad-app/src/tests/drawing/text_batch.rs`).

use super::super::super::*;

#[test]
fn justifytext_and_tjust_share_one_grammar() {
    use cad_domain::{TextAlignH, TextAlignV};

    for (argument, expected) in [
        ("LEFT BASELINE", (TextAlignH::Left, TextAlignV::Baseline)),
        ("left baseline", (TextAlignH::Left, TextAlignV::Baseline)),
        (
            "  center\tmiddle  ",
            (TextAlignH::Center, TextAlignV::Middle),
        ),
        ("RIGHT TOP", (TextAlignH::Right, TextAlignV::Top)),
        ("CENTER BOTTOM", (TextAlignH::Center, TextAlignV::Bottom)),
        ("right middle", (TextAlignH::Right, TextAlignV::Middle)),
    ] {
        assert_eq!(
            parse_justifytext_argument(argument),
            Some(expected),
            "{argument}"
        );
    }
    // Missing half, extra word, abbreviation, swapped axes: all refused.
    for refused in [
        "",
        "LEFT",
        "LEFT TOP RIGHT",
        "MIDDLE TOP",
        "LEFT MID",
        "L B",
    ] {
        assert_eq!(parse_justifytext_argument(refused), None, "{refused:?}");
    }
    // One canonical selector with one typed argument form, plus its alias.
    assert_eq!(
        typed_command_argument("JUSTIFYTEXT LEFT TOP"),
        Some(("JUSTIFYTEXT", "LEFT TOP"))
    );
    assert_eq!(
        typed_command_argument("TJUST right bottom"),
        Some(("JUSTIFYTEXT", "right bottom")),
        "TJUST normalizes onto the canonical selector"
    );
    assert_eq!(
        typed_command_argument("JUSTIFYTEXT"),
        None,
        "bare is name-only"
    );
    assert_eq!(normalize_command("TJUST"), Some(CommandId::JustifyText));
    assert_eq!(
        canonical_command_name("TJUZT"),
        None,
        "a typo is not an alias"
    );
    assert!(CommandId::JustifyText.requires_work_mode());
    assert_eq!(command_name(CommandId::JustifyText), Some("JUSTIFYTEXT"));
    // The refusal names the whole grammar.
    let reason = invalid_justifytext_argument_reason("JUSTIFYTEXT MIDDLE TOP");
    assert!(reason.contains("LEFT, CENTER or RIGHT"), "{reason}");
    assert!(
        reason.contains("BASELINE, BOTTOM, MIDDLE or TOP"),
        "{reason}"
    );
}

#[test]
fn tscale_accepts_one_positive_decimal() {
    for (argument, expected) in [("2", 2.0), ("0.5", 0.5), ("1.25", 1.25), (" 2.5 ", 2.5)] {
        assert_eq!(
            parse_tscale_argument(argument),
            Some(expected),
            "{argument}"
        );
    }
    for refused in ["", "0", "0.0", "-1", "+2", "1e3", "2 3", "NaN", "inf"] {
        assert_eq!(parse_tscale_argument(refused), None, "{refused:?}");
    }
    assert_eq!(typed_command_argument("TSCALE 2"), Some(("TSCALE", "2")));
    assert_eq!(typed_command_argument("TSCALE"), None);
    assert!(CommandId::TextScale.requires_work_mode());
    let reason = invalid_tscale_argument_reason("TSCALE 0");
    assert!(reason.contains("positive decimal factor"), "{reason}");
}

#[test]
fn textalign_accepts_the_fifteen_alignment_words() {
    use cad_app::TextAlignEdge;

    for (argument, expected) in [
        ("LEFT", TextAlignEdge::Left),
        ("right", TextAlignEdge::Right),
        ("Center", TextAlignEdge::Center),
        ("TOP", TextAlignEdge::Top),
        ("middle", TextAlignEdge::Middle),
        ("BOTTOM", TextAlignEdge::Bottom),
        ("TOPLEFT", TextAlignEdge::TopLeft),
        ("topcenter", TextAlignEdge::TopCenter),
        ("TopRight", TextAlignEdge::TopRight),
        ("MIDDLELEFT", TextAlignEdge::MiddleLeft),
        ("middlecenter", TextAlignEdge::MiddleCenter),
        ("MIDDLERIGHT", TextAlignEdge::MiddleRight),
        ("BOTTOMLEFT", TextAlignEdge::BottomLeft),
        ("bottomcenter", TextAlignEdge::BottomCenter),
        ("BOTTOMRIGHT", TextAlignEdge::BottomRight),
    ] {
        assert_eq!(
            parse_textalign_argument(argument),
            Some(expected),
            "{argument}"
        );
    }
    for refused in [
        "",
        "L",
        "C",
        "T",
        "CENTRE",
        "TOP LEFT",
        "TOP-LEFT",
        "MIDDLE CENTER",
        "BOTTOM_RIGHT",
    ] {
        assert_eq!(parse_textalign_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("TEXTALIGN TOPLEFT"),
        Some(("TEXTALIGN", "TOPLEFT"))
    );
    assert_eq!(typed_command_argument("TEXTALIGN"), None);
    assert!(CommandId::TextAlignEdges.requires_work_mode());
    assert_eq!(command_name(CommandId::TextAlignEdges), Some("TEXTALIGN"));
    let reason = invalid_textalign_argument_reason("TEXTALIGN TOP LEFT");
    assert!(reason.contains("BOTTOMRIGHT"), "{reason}");
    assert!(reason.contains("DISTRIBUTE [X|Y]"), "{reason}");
}

#[test]
fn textalign_parses_distribute_and_the_named_spacing_refusals() {
    use cad_app::{TextAlignAxis, TextAlignSpacing};

    assert_eq!(
        parse_textalign_spacing_argument("DISTRIBUTE"),
        Some(TextAlignSpacing::Distribute(TextAlignAxis::Y)),
        "the default axis is Y"
    );
    assert_eq!(
        parse_textalign_spacing_argument("distribute x"),
        Some(TextAlignSpacing::Distribute(TextAlignAxis::X))
    );
    assert_eq!(
        parse_textalign_spacing_argument("Distribute Y"),
        Some(TextAlignSpacing::Distribute(TextAlignAxis::Y))
    );
    assert_eq!(
        parse_textalign_spacing_argument("SETSPACING"),
        Some(TextAlignSpacing::SetSpacing)
    );
    assert_eq!(
        parse_textalign_spacing_argument("currentvertical"),
        Some(TextAlignSpacing::CurrentVertical)
    );
    assert_eq!(
        parse_textalign_spacing_argument("CURRENTHORIZONTAL"),
        Some(TextAlignSpacing::CurrentHorizontal)
    );
    for refused in [
        "",
        "DISTRIBUTE Z",
        "DISTRIBUTE X Y",
        "SET SPACING",
        "DISTRIBUT",
    ] {
        assert_eq!(
            parse_textalign_spacing_argument(refused),
            None,
            "{refused:?}"
        );
    }
    assert_eq!(
        typed_command_argument("TEXTALIGN DISTRIBUTE X"),
        Some(("TEXTALIGN", "DISTRIBUTE X"))
    );
}

#[test]
fn tcount_parses_the_run_and_its_affixes() {
    let bare = parse_tcount_argument("1 1").expect("two tokens");
    assert_eq!((bare.start, bare.step), (1, 1));
    assert_eq!((bare.prefix.as_str(), bare.suffix.as_str()), ("", ""));

    let prefix = parse_tcount_argument("10 5 A").expect("three tokens");
    assert_eq!((prefix.start, prefix.step), (10, 5));
    assert_eq!((prefix.prefix.as_str(), prefix.suffix.as_str()), ("A", ""));

    let both = parse_tcount_argument("-2 3 A -").expect("four tokens");
    assert_eq!((both.start, both.step), (-2, 3));
    assert_eq!((both.prefix.as_str(), both.suffix.as_str()), ("A", "-"));

    // A negative/zero step is parsed but refused by the application
    // (`tcount.invalid_step`), so the two refusal sites say the same thing.
    assert_eq!(parse_tcount_argument("1 0").map(|run| run.step), Some(0));

    for refused in ["", "1", "1 1 A - 9", "a 1", "1 b", "1.5 1"] {
        assert_eq!(parse_tcount_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("TCOUNT 1 1 A -"),
        Some(("TCOUNT", "1 1 A -"))
    );
    assert_eq!(typed_command_argument("TCOUNT"), None);
    assert!(CommandId::TextCount.requires_work_mode());
    let reason = invalid_tcount_argument_reason("TCOUNT 1");
    assert!(reason.contains("<start> <step>"), "{reason}");
}

#[test]
fn tcircle_accepts_only_the_two_boundary_words() {
    use cad_app::TextBoundaryShape;

    assert_eq!(
        parse_tcircle_argument("CIRCLE"),
        Some(TextBoundaryShape::Circle)
    );
    assert_eq!(
        parse_tcircle_argument("rectangle"),
        Some(TextBoundaryShape::Rectangle)
    );
    for refused in ["", "C", "RECT", "CIRCLE RECTANGLE"] {
        assert_eq!(parse_tcircle_argument(refused), None, "{refused:?}");
    }
    assert_eq!(
        typed_command_argument("TCIRCLE CIRCLE"),
        Some(("TCIRCLE", "CIRCLE"))
    );
    assert_eq!(typed_command_argument("TCIRCLE"), None);
    assert!(CommandId::TextCircle.requires_work_mode());
    let reason = invalid_tcircle_argument_reason("TCIRCLE C");
    assert!(reason.contains("CIRCLE or RECTANGLE"), "{reason}");
}

#[test]
fn the_bare_batch_selectors_are_admitted_and_work_only() {
    for (selector, id) in [
        ("JUSTIFYTEXT", CommandId::JustifyText),
        ("TSCALE", CommandId::TextScale),
        ("TEXTALIGN", CommandId::TextAlignEdges),
        ("TCOUNT", CommandId::TextCount),
        ("TCIRCLE", CommandId::TextCircle),
        ("TEXTMASK", CommandId::TextMask),
        ("TEXTUNMASK", CommandId::TextUnmask),
    ] {
        assert_eq!(normalize_command(selector), Some(id), "{selector}");
        assert_eq!(
            canonical_command_name(selector),
            Some(selector),
            "{selector}"
        );
        assert!(admissible_names().contains(&selector), "{selector}");
        assert!(id.requires_work_mode(), "{selector} writes the drawing");
        assert_eq!(command_name(id), Some(selector));
    }
    // TEXTMASK takes the P3 option line; TEXTUNMASK takes no argument.
    assert_eq!(
        typed_command_argument("TEXTMASK OFFSET 0.5"),
        Some(("TEXTMASK", "OFFSET 0.5"))
    );
    assert_eq!(typed_command_argument("TEXTMASK"), None);
    assert_eq!(typed_command_argument("TEXTUNMASK 1"), None);
}

#[test]
fn mtedit_and_dash_mtext_are_aliases_of_the_existing_paths() {
    // MTEDIT is AutoCAD's name for editing the selected text; it normalizes onto
    // the existing TEXTEDIT pick tool (no second edit path).
    assert_eq!(canonical_command_name("MTEDIT"), Some("TEXTEDIT"));
    assert_eq!(normalize_command("MTEDIT"), Some(CommandId::TextEdit));
    assert!(admissible_names().contains(&"MTEDIT"));
    // `-MTEXT` is AutoCAD's command-line MTEXT form; it starts the same
    // two-corner tool as `MTEXT` (no second capture path).
    assert_eq!(canonical_command_name("-MTEXT"), Some("MTEXT"));
    assert_eq!(normalize_command("-MTEXT"), Some(CommandId::CreateMText));
    assert_eq!(normalize_command("-mtext"), Some(CommandId::CreateMText));
    assert!(admissible_names().contains(&"-MTEXT"));
    // Neither alias grows an argument form.
    assert_eq!(typed_command_argument("MTEDIT 1"), None);
    assert_eq!(typed_command_argument("-MTEXT 1"), None);
}
