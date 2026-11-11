//! The paper-space/session/point-style typed lines (`VPLAYER`, `MVSETUP`,
//! `APERTURE`, `PTYPE`, `REVCLOUDPROPERTIES`) at the router boundary.
//!
//! These pin the selector admission, the narrow grammars and the dispatch: a
//! valid line reaches the existing payload path (the application's own
//! validation and write), and a malformed line is refused with the selector's
//! stable `*.` reason and changes nothing. The semantic write behaviour is
//! pinned in `cad-app` (`layout_commands`, `extended_session2`,
//! `revision_cloud`, `point_display_style`); here the session must route the
//! parsed value to the right `CommandId`.

use super::super::super::commands::{
    aperture_out_of_range_reason, invalid_mvsetup_argument_reason, invalid_ptype_argument_reason,
    invalid_revcloudproperties_argument_reason, invalid_vplayer_argument_reason,
    parse_aperture_argument, parse_mvsetup_argument, parse_ptype_argument,
    parse_revcloud_properties_argument, parse_spacetrans_argument, parse_vplayer_argument,
    PointTypeArgument, VPlayerArgument, APERTURE_SELECTOR, MVSETUP_SELECTOR, PTYPE_SELECTOR,
    REVCLOUDPROPERTIES_SELECTOR, SPACETRANS_SELECTOR, VPLAYER_SELECTOR,
};
use super::super::super::*;
use super::super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn tokens(values: &[&str]) -> VPlayerArgument {
    VPlayerArgument::Tokens(values.iter().map(|value| (*value).to_string()).collect())
}

#[test]
fn the_five_selectors_are_admitted_argument_forms_with_one_row_each() {
    for selector in [
        VPLAYER_SELECTOR,
        MVSETUP_SELECTOR,
        APERTURE_SELECTOR,
        PTYPE_SELECTOR,
        REVCLOUDPROPERTIES_SELECTOR,
    ] {
        assert!(
            ARGUMENT_SELECTORS.contains(&selector),
            "{selector} must be an argument form of its canonical selector"
        );
        assert_eq!(
            typed_command_argument(&format!("{selector} 1")),
            Some((selector, "1")),
            "{selector} must split into selector + argument"
        );
        assert!(
            canonical_command_name(selector).is_some(),
            "{selector} is the existing canonical name"
        );
    }
}

#[test]
fn vplayer_grammar_keeps_the_handlers_tokens_verbatim() {
    assert_eq!(parse_vplayer_argument("?"), Some(VPlayerArgument::Report));
    assert_eq!(parse_vplayer_argument(" ? "), Some(VPlayerArgument::Report));
    assert_eq!(
        parse_vplayer_argument("FREEZE WALLS,DIMS"),
        Some(tokens(&["FREEZE", "WALLS,DIMS"]))
    );
    assert_eq!(
        parse_vplayer_argument(" thaw WALLS,DIMS [2] "),
        Some(tokens(&["thaw", "WALLS,DIMS", "[2]"]))
    );
    // A bare numeric token stays a token: the application resolves layer names
    // first, so `FREEZE 2` addresses the layer named `2`.
    assert_eq!(
        parse_vplayer_argument("FREEZE 2"),
        Some(tokens(&["FREEZE", "2"]))
    );
    assert_eq!(
        parse_vplayer_argument("F WALLS 2"),
        Some(tokens(&["F", "WALLS", "2"]))
    );
    for refused in [
        "",
        "BOGUS WALLS",
        "FREEZE",
        "FREEZE [2]",
        "FREEZE WALLS [x]",
        "FREEZE WALLS [2",
    ] {
        assert_eq!(parse_vplayer_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_vplayer_argument_reason("VPLAYER BOGUS");
    assert!(reason.starts_with("vplayer.invalid_argument:"), "{reason}");
    assert!(reason.contains("VPLAYER BOGUS"), "{reason}");
}

#[test]
fn mvsetup_grammar_is_exactly_rows_cols_and_a_scale() {
    assert_eq!(
        parse_mvsetup_argument("2 2 1:100").unwrap(),
        vec!["2", "2", "1:100"]
    );
    assert_eq!(
        parse_mvsetup_argument(" 1\t3  50 ").unwrap(),
        vec!["1", "3", "50"]
    );
    for refused in [
        "",
        "2 2",
        "2 2 1:100 extra",
        "0 2 1:1",
        "-1 2 1:1",
        "a b c",
        "2.5 2 1:1",
    ] {
        assert_eq!(parse_mvsetup_argument(refused), None, "{refused:?}");
    }
    let reason = invalid_mvsetup_argument_reason("MVSETUP 2 2");
    assert!(reason.starts_with("mvsetup.invalid_argument:"), "{reason}");
}

#[test]
fn aperture_grammar_is_one_whole_pixel_value_in_autocads_range() {
    for accepted in [1u32, 10, 24, 50] {
        assert_eq!(parse_aperture_argument(&accepted.to_string()), Ok(accepted));
    }
    for out_of_range in ["0", "51", "1000"] {
        let reason = parse_aperture_argument(out_of_range).expect_err(out_of_range);
        assert!(
            reason.starts_with("aperture.out_of_range:"),
            "{out_of_range}: {reason}"
        );
    }
    for malformed in ["", "abc", "1.5", "-1", "+5", "1e1", "  "] {
        let reason = parse_aperture_argument(malformed).expect_err(malformed);
        assert!(
            reason.starts_with("aperture.invalid_argument:"),
            "{malformed}: {reason}"
        );
    }
    assert!(aperture_out_of_range_reason("APERTURE 0").contains("1..=50"));
}

#[test]
fn ptype_grammar_reuses_the_pdmode_and_pdsize_rules() {
    assert_eq!(
        parse_ptype_argument("35"),
        Ok(PointTypeArgument {
            mode: 35,
            size: None
        })
    );
    assert_eq!(
        parse_ptype_argument(" -1 2.5 "),
        Ok(PointTypeArgument {
            mode: -1,
            size: Some(2.5)
        })
    );
    for refused in ["", "3.5", "40000", "35 NaN", "35 2 3", "abc"] {
        let reason = parse_ptype_argument(refused).expect_err(refused);
        assert!(
            reason.starts_with("ptype.invalid_argument:"),
            "{refused:?}: {reason}"
        );
    }
    assert!(invalid_ptype_argument_reason("PTYPE x").contains("`$PDMODE`"));
}

#[test]
fn revcloud_properties_grammar_is_one_finite_positive_decimal() {
    assert_eq!(parse_revcloud_properties_argument("15"), Ok(15.0));
    assert_eq!(parse_revcloud_properties_argument(" 0.5 "), Ok(0.5));
    for refused in ["", "0", "-1", "NaN", "inf", "abc", "1e3"] {
        let reason = parse_revcloud_properties_argument(refused).expect_err(refused);
        assert!(
            reason.starts_with("revcloudproperties.invalid_argument:"),
            "{refused:?}: {reason}"
        );
    }
    assert!(
        invalid_revcloudproperties_argument_reason("REVCLOUDPROPERTIES 0").contains("positive")
    );
}

#[test]
fn typed_aperture_and_revcloud_properties_write_the_session_tokens() {
    let session = fresh_session();
    let controller = session.core.controller.borrow();
    assert_eq!(controller.session.drafting_extras.aperture(), 10);
    drop(controller);

    session.command("APERTURE 24").expect("24 is in range");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .aperture(),
        24
    );
    assert!(session
        .core
        .controller
        .borrow()
        .session
        .drafting_extras
        .aperture_explicit());

    session
        .command("REVCLOUDPROPERTIES 2.5")
        .expect("2.5 is a positive arc length");
    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .drafting_extras
            .revcloud_arc_length(),
        2.5
    );

    // Bare forms stay the read-only reports.
    session.command("APERTURE").expect("bare APERTURE reports");
    session
        .command("REVCLOUDPROPERTIES")
        .expect("bare REVCLOUDPROPERTIES reports");
}

#[test]
fn a_malformed_session_token_line_is_refused_by_name_and_changes_nothing() {
    let session = fresh_session();
    for (typed, needle) in [
        ("APERTURE 0", "aperture.out_of_range"),
        ("APERTURE 51", "aperture.out_of_range"),
        ("APERTURE 1.5", "aperture.invalid_argument"),
        (
            "REVCLOUDPROPERTIES 0",
            "revcloudproperties.invalid_argument",
        ),
        (
            "REVCLOUDPROPERTIES NaN",
            "revcloudproperties.invalid_argument",
        ),
        ("PTYPE 3.5", "ptype.invalid_argument"),
        ("PTYPE 40000", "ptype.invalid_argument"),
    ] {
        let error = session.command(typed).expect_err(typed);
        assert!(error.contains(needle), "{typed:?} got {error:?}");
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str()),
            "the refusal is recorded for cad_status.error"
        );
    }
    let controller = session.core.controller.borrow();
    assert_eq!(controller.session.drafting_extras.aperture(), 10);
    assert_eq!(
        controller.session.drafting_extras.revcloud_arc_length(),
        15.0
    );
}

#[test]
fn typed_ptype_writes_both_point_style_headers_through_the_existing_path() {
    let session = fresh_session();
    let before = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a drawing is open");
        (
            drawing.point_display_mode(),
            drawing.point_display_size(),
            drawing.revision(),
        )
    };
    session
        .command("PTYPE 35 2.5")
        .expect("a legal code and size are admitted");
    let (mode, size, revision) = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a drawing is open");
        (
            drawing.point_display_mode(),
            drawing.point_display_size(),
            drawing.revision(),
        )
    };
    assert_eq!(mode, 35, "the code is the raw $PDMODE");
    assert_eq!(size, 2.5, "the optional size is the raw $PDSIZE");
    assert_eq!(
        revision,
        cad_domain::Revision(before.2 .0 + 2),
        "each of the two header writes bumps the revision"
    );
    assert_eq!(before.0, 0);
    assert_eq!(before.1, 0.0);
}

#[test]
fn typed_vplayer_and_mvsetup_reach_their_existing_handlers() {
    // A paper layout with no viewports: `VPLAYER` reaches the handler and is
    // refused for the missing viewport (not for a malformed argument), and
    // `MVSETUP` creates its real grid.
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    session
        .command("LAYOUT SET Sheet One")
        .expect("LAYOUT SET switches to the paper layout");

    let error = session
        .command("VPLAYER FREEZE 0")
        .expect_err("the fixture drawing has no layer to freeze");
    assert!(error.contains("vplayer.unknown_layer"), "{error}");

    // `?` stays the read-only report, which answers the empty layout honestly.
    session
        .command("VPLAYER ?")
        .expect("the `?` form is the read-only report");

    session
        .command("MVSETUP 2 2 1:100")
        .expect("MVSETUP writes the grid");
    let viewports = {
        let controller = session.core.controller.borrow();
        controller
            .drawing()
            .expect("a drawing is open")
            .layouts()
            .find(|layout| layout.id == cad_domain::LayoutId(2))
            .map(|layout| layout.viewports.len())
            .unwrap_or(0)
    };
    assert_eq!(viewports, 4, "2 x 2 viewports were written");

    let before = viewports;
    let error = session
        .command("MVSETUP 0 2 1:1")
        .expect_err("a zero row count is malformed");
    assert!(error.contains("mvsetup.invalid_argument"), "{error}");
    let after = {
        let controller = session.core.controller.borrow();
        controller
            .drawing()
            .expect("a drawing is open")
            .layouts()
            .find(|layout| layout.id == cad_domain::LayoutId(2))
            .map(|layout| layout.viewports.len())
            .unwrap_or(0)
    };
    assert_eq!(after, before, "a refusal changes no viewport");
}

#[test]
fn spacetrans_is_one_positive_decimal_and_reaches_the_conversion_command() {
    assert_eq!(parse_spacetrans_argument("100"), Some(100.0));
    assert_eq!(parse_spacetrans_argument(" 2.5 "), Some(2.5));
    for refused in ["", "0", "-4", "1e3", "1.2.3", "10 20"] {
        assert_eq!(parse_spacetrans_argument(refused), None, "{refused:?}");
    }
    assert!(
        ARGUMENT_SELECTORS.contains(&SPACETRANS_SELECTOR),
        "SPACETRANS is an argument form of its canonical selector"
    );
    let session = fresh_session();
    let error = session.command("SPACETRANS 100").unwrap_err();
    assert!(error.contains("spacetrans.model_space"), "{error}");
}
