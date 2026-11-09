//! The typed P3 option lines (`OVERKILL`, `REVCLOUD`, `WIPEOUT`, `TEXTMASK`,
//! `TXT2MTXT`) at the router boundary.
//!
//! Each form is an **argument of its canonical selector**, never a new name, and
//! a bare selector keeps the previous payload path. Malformed arguments are
//! refused by their stable grammar reason and record nothing. The dispatch tests
//! drive the real host application through `CadSession::command` so the built
//! payload is what the option-aware entry point actually consumes.

use super::super::super::commands::{
    invalid_overkill_argument_reason, invalid_revcloud_argument_reason,
    invalid_textmask_argument_reason, invalid_txt2mtext_argument_reason,
    invalid_wipeout_argument_reason, parse_overkill_argument, parse_revcloud_argument,
    parse_textmask_argument, parse_txt2mtext_argument, parse_wipeout_argument, RevCloudArgument,
    TextMaskArgument, Txt2MTextArgument, WipeoutArgument, OVERKILL_SELECTOR, REVCLOUD_SELECTOR,
    TEXTMASK_SELECTOR, TXT2MTXT_SELECTOR, WIPEOUT_SELECTOR,
};
use super::super::super::*;
use super::super::test_support::*;

use cad_app::{OverkillSettings, RevCloudStyle, TextMaskKind, TextOrder, WipeoutFrames};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn recorded(session: &CadSession) -> Vec<String> {
    session
        .snapshot(None, None, 0)
        .command_log
        .iter()
        .filter_map(|entry| entry.command.clone())
        .collect()
}

#[test]
fn overkill_grammar_builds_the_dialog_settings() {
    assert_eq!(
        parse_overkill_argument("TOLERANCE 2.5"),
        Some(OverkillSettings {
            tolerance: 2.5,
            ..OverkillSettings::default()
        })
    );
    assert_eq!(
        parse_overkill_argument("TOLERANCE 0"),
        Some(OverkillSettings::default()),
        "zero tolerance is exact comparison"
    );
    let ignored =
        parse_overkill_argument("IGNORE COLOR LAYER LINETYPE LTSCALE LINEWEIGHT TRANSPARENCY")
            .expect("every modelled ignore property");
    assert!(ignored.ignore_color);
    assert!(ignored.ignore_layer);
    assert!(ignored.ignore_linetype);
    assert!(ignored.ignore_linetype_scale);
    assert!(ignored.ignore_lineweight);
    assert!(ignored.ignore_transparency);

    // Repeated IGNORE words accumulate for the invocation.
    let repeated = parse_overkill_argument("IGNORE COLOR IGNORE LAYER").expect("repeated IGNORE");
    assert!(repeated.ignore_color && repeated.ignore_layer);

    let disabled = parse_overkill_argument("NOOPTIMIZE NOBREAK NOCOLLINEAR NOENDTOEND")
        .expect("disabled option set");
    assert!(!disabled.optimize_segments);
    assert!(disabled.do_not_break_polylines);
    assert!(!disabled.combine_partial_overlap);
    assert!(!disabled.combine_end_to_end);

    let enabled =
        parse_overkill_argument("OPTIMIZE BREAK COLLINEAR ENDTOEND").expect("enabled option set");
    assert!(enabled.optimize_segments);
    assert!(!enabled.do_not_break_polylines);
    assert!(enabled.combine_partial_overlap);
    assert!(enabled.combine_end_to_end);

    for refused in [
        "TOLERANCE -1",
        "TOLERANCE NaN",
        "TOLERANCE",
        "IGNORE",
        "IGNORE THICKNESS",
        "IGNORE PLOTSTYLE",
        "IGNORE MATERIAL",
        "IGNORE COLOR BOGUS",
        "BOGUS",
    ] {
        assert_eq!(parse_overkill_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_overkill_argument_reason("OVERKILL BOGUS");
    assert!(reason.contains("OVERKILL needs TOLERANCE"), "{reason}");
    assert!(reason.contains("THICKNESS"), "{reason}");
}

#[test]
fn revcloud_grammar_builds_each_mode_and_rejects_bad_chords() {
    assert_eq!(
        parse_revcloud_argument("ARC 2.5"),
        Some(RevCloudArgument::Object {
            arc_length: Some(2.5)
        })
    );
    assert_eq!(
        parse_revcloud_argument("OBJECT"),
        Some(RevCloudArgument::Object { arc_length: None })
    );
    for word in ["RECT", "RECTANGULAR"] {
        assert_eq!(
            parse_revcloud_argument(word),
            Some(RevCloudArgument::Rectangular),
            "{word}"
        );
    }
    for word in ["POLY", "POLYGONAL"] {
        assert_eq!(
            parse_revcloud_argument(word),
            Some(RevCloudArgument::Polygonal),
            "{word}"
        );
    }
    assert_eq!(
        parse_revcloud_argument("FREEHAND"),
        Some(RevCloudArgument::Freehand)
    );
    assert_eq!(
        parse_revcloud_argument("STYLE NORMAL"),
        Some(RevCloudArgument::Style(RevCloudStyle::Normal))
    );
    assert_eq!(
        parse_revcloud_argument("STYLE CALLIGRAPHY"),
        Some(RevCloudArgument::Style(RevCloudStyle::Calligraphy)),
        "the grammar admits it; the command layer refuses it by name"
    );
    assert_eq!(
        parse_revcloud_argument("MODIFY"),
        Some(RevCloudArgument::Modify)
    );

    for refused in ["ARC 0", "ARC -1", "ARC", "STYLE WEIRD", "STYLE", "BOGUS"] {
        assert_eq!(parse_revcloud_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_revcloud_argument_reason("REVCLOUD BOGUS");
    assert!(reason.contains("REVCLOUD needs ARC"), "{reason}");
}

#[test]
fn wipeout_grammar_builds_frames_and_polyline_options() {
    assert_eq!(
        parse_wipeout_argument("FRAMES ON"),
        Some(WipeoutArgument::Frames(WipeoutFrames::On))
    );
    assert_eq!(
        parse_wipeout_argument("FRAMES OFF"),
        Some(WipeoutArgument::Frames(WipeoutFrames::Off))
    );
    assert_eq!(
        parse_wipeout_argument("FRAMES DISPLAY"),
        Some(WipeoutArgument::Frames(WipeoutFrames::DisplayNotPlot)),
        "DISPLAY is display-but-not-plot"
    );
    assert_eq!(
        parse_wipeout_argument("POLYLINE"),
        Some(WipeoutArgument::Polyline { erase: false })
    );
    assert_eq!(
        parse_wipeout_argument("POLYLINE ERASE YES"),
        Some(WipeoutArgument::Polyline { erase: true })
    );
    assert_eq!(
        parse_wipeout_argument("POLYLINE ERASE NO"),
        Some(WipeoutArgument::Polyline { erase: false })
    );

    for refused in [
        "FRAMES MAYBE",
        "FRAMES",
        "POLYLINE ERASE MAYBE",
        "POLYLINE ERASE",
        "BOGUS",
    ] {
        assert_eq!(parse_wipeout_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_wipeout_argument_reason("WIPEOUT BOGUS");
    assert!(reason.contains("WIPEOUT needs FRAMES"), "{reason}");
}

#[test]
fn textmask_grammar_builds_offset_and_mask_types() {
    assert_eq!(
        parse_textmask_argument("OFFSET 0"),
        Some(TextMaskArgument::Offset(0.0))
    );
    assert_eq!(
        parse_textmask_argument("OFFSET 0.35"),
        Some(TextMaskArgument::Offset(0.35))
    );
    assert_eq!(
        parse_textmask_argument("MASK WIPEOUT"),
        Some(TextMaskArgument::Mask(TextMaskKind::Wipeout))
    );
    assert_eq!(
        parse_textmask_argument("MASK 3DFACE"),
        Some(TextMaskArgument::Mask(TextMaskKind::Face3d))
    );
    assert_eq!(
        parse_textmask_argument("MASK SOLID"),
        Some(TextMaskArgument::Mask(TextMaskKind::Solid))
    );

    for refused in ["OFFSET -1", "OFFSET", "MASK BOGUS", "BOGUS"] {
        assert_eq!(parse_textmask_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_textmask_argument_reason("TEXTMASK BOGUS");
    assert!(reason.contains("TEXTMASK needs OFFSET"), "{reason}");
}

#[test]
fn txt2mtext_grammar_builds_each_settings_toggle() {
    assert_eq!(
        parse_txt2mtext_argument("COMBINE"),
        Some(Txt2MTextArgument::Combine(true))
    );
    assert_eq!(
        parse_txt2mtext_argument("SEPARATE"),
        Some(Txt2MTextArgument::Combine(false))
    );
    assert_eq!(
        parse_txt2mtext_argument("ORDER TOPDOWN"),
        Some(Txt2MTextArgument::Order(TextOrder::TopDown))
    );
    assert_eq!(
        parse_txt2mtext_argument("ORDER SELECT"),
        Some(Txt2MTextArgument::Order(TextOrder::Selection))
    );
    assert_eq!(
        parse_txt2mtext_argument("WRAP"),
        Some(Txt2MTextArgument::WordWrap)
    );
    assert_eq!(
        parse_txt2mtext_argument("UNIFORM"),
        Some(Txt2MTextArgument::Uniform)
    );

    for refused in ["ORDER BOGUS", "ORDER", "COMBINE NOW", "BOGUS"] {
        assert_eq!(parse_txt2mtext_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_txt2mtext_argument_reason("TXT2MTXT BOGUS");
    assert!(reason.contains("TXT2MTXT needs COMBINE"), "{reason}");
}

#[test]
fn the_five_selectors_are_admitted_and_bare_names_stay_name_only() {
    for (line, selector, argument) in [
        ("OVERKILL TOLERANCE 1", OVERKILL_SELECTOR, "TOLERANCE 1"),
        ("REVCLOUD ARC 1", REVCLOUD_SELECTOR, "ARC 1"),
        ("WIPEOUT FRAMES ON", WIPEOUT_SELECTOR, "FRAMES ON"),
        ("TEXTMASK OFFSET 0.5", TEXTMASK_SELECTOR, "OFFSET 0.5"),
        ("TXT2MTXT COMBINE", TXT2MTXT_SELECTOR, "COMBINE"),
    ] {
        assert_eq!(
            typed_command_argument(line),
            Some((selector, argument)),
            "{line}"
        );
        assert!(ARGUMENT_SELECTORS.contains(&selector), "{selector}");
    }
    for bare in ["OVERKILL", "REVCLOUD", "WIPEOUT", "TEXTMASK", "TXT2MTXT"] {
        assert_eq!(typed_command_argument(bare), None, "{bare}");
    }
}

#[test]
fn wipeout_frames_reach_the_session_state_through_the_typed_line() {
    let session = fresh_session();
    let frames = || {
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .wipeout_frames()
    };
    session.command("WIPEOUT FRAMES OFF").expect("OFF");
    assert_eq!(frames(), WipeoutFrames::Off);
    session.command("WIPEOUT FRAMES DISPLAY").expect("DISPLAY");
    assert_eq!(frames(), WipeoutFrames::DisplayNotPlot);
    session.command("WIPEOUT FRAMES ON").expect("ON");
    assert_eq!(frames(), WipeoutFrames::On);
    assert!(
        recorded(&session)
            .iter()
            .filter(|name| name.as_str() == "WIPEOUT")
            .count()
            >= 3,
        "each typed line echoes the canonical selector"
    );
}

#[test]
fn option_payloads_reach_the_command_layer() {
    let session = fresh_session();
    // The command layer checks these settings before it reads the selection, so
    // the refusal is proof the built payload arrived with the typed value.
    let error = session.command("TXT2MTXT UNIFORM").unwrap_err();
    assert!(error.contains("txt2mtxt.uniform_line_spacing"), "{error}");
    let error = session.command("TEXTMASK MASK SOLID").unwrap_err();
    assert!(error.contains("textmask.mask_type"), "{error}");
    let error = session.command("REVCLOUD STYLE CALLIGRAPHY").unwrap_err();
    assert!(error.contains("revcloud.style_unsupported"), "{error}");

    // These reach their option-aware entry point and refuse the empty selection
    // by name instead of falling through to the unsupported-command path.
    let error = session.command("OVERKILL TOLERANCE 1").unwrap_err();
    assert!(error.contains("overkill.selection"), "{error}");
    let error = session.command("REVCLOUD ARC 1").unwrap_err();
    assert!(error.contains("revcloud.selection"), "{error}");
    let error = session.command("TEXTMASK OFFSET 0.5").unwrap_err();
    assert!(error.contains("textmask.needs_selection"), "{error}");
    let error = session.command("TXT2MTXT SEPARATE").unwrap_err();
    assert!(error.contains("txt2mtxt.needs_selection"), "{error}");
    let error = session.command("WIPEOUT POLYLINE").unwrap_err();
    assert!(error.contains("wipeout.polyline_selection"), "{error}");
    let error = session.command("REVCLOUD RECT").unwrap_err();
    assert!(error.contains("revcloud.selection"), "{error}");
}

#[test]
fn malformed_option_lines_are_refused_and_not_recorded() {
    let session = fresh_session();
    for (line, grammar) in [
        ("OVERKILL TOLERANCE -1", "OVERKILL needs TOLERANCE"),
        ("OVERKILL IGNORE THICKNESS", "THICKNESS"),
        ("REVCLOUD ARC 0", "REVCLOUD needs ARC"),
        ("WIPEOUT FRAMES MAYBE", "WIPEOUT needs FRAMES"),
        ("TEXTMASK OFFSET -1", "TEXTMASK needs OFFSET"),
        ("TXT2MTXT ORDER BOGUS", "TXT2MTXT needs COMBINE"),
    ] {
        let before = recorded(&session).len();
        let error = session.command(line).unwrap_err();
        assert!(error.contains(grammar), "{line}: {error}");
        assert_eq!(
            recorded(&session).len(),
            before,
            "{line} must not be recorded as a command"
        );
    }
}
