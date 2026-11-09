//! The typed argument forms of the 2026-11-07 layout / viewport batch.
//!
//! These tests pin the selector admission, the narrow grammar and the
//! normalized `-VPORTS` alias. The semantic write behaviour is pinned in
//! `cad-app`'s `layout_commands` tests; here the session must route the parsed
//! payload to the right `CommandId` and surface the real outcome (a successful
//! write, or the payload/space-level refusal such as `vpclip.no_viewport`).

use super::super::super::*;
use super::super::test_support::*;

fn active_space(session: &CadSession) -> CadActiveSpace {
    session.snapshot(None, None, 0).active_space
}

fn switch_to_sheet(session: &CadSession) {
    session
        .command("LAYOUT SET Sheet One")
        .expect("LAYOUT SET resolves the paper layout");
}

#[test]
fn layout_list_and_set_are_typed_forms_of_one_selector() {
    let session = session_with_layouts(&[(2, "Sheet One", None), (3, "Sheet Two", None)]);
    session
        .command("LAYOUT LIST")
        .expect("LAYOUT LIST is admitted");
    assert_eq!(active_space(&session), CadActiveSpace::Model);
    session
        .command("LAYOUT SET Sheet Two")
        .expect("LAYOUT SET switches the active space");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 3 });

    let error = session
        .command("LAYOUT SET Nowhere")
        .expect_err("an unknown layout is refused");
    assert!(error.contains("layout.unknown"), "{error}");
    assert_eq!(active_space(&session), CadActiveSpace::Paper { id: 3 });
}

#[test]
fn layout_grammar_rejects_missing_options_and_illegal_names() {
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    for text in ["LAYOUT BOGUS", "LAYOUT NEW", "LAYOUT RENAME One"] {
        let error = session.command(text).expect_err(text);
        assert!(error.contains("expected"), "{text}: {error}");
    }
    let duplicate = session
        .command("LAYOUT NEW sheet one")
        .expect_err("a case-insensitive duplicate is refused");
    assert!(duplicate.contains("layout.duplicate_name"), "{duplicate}");
    let illegal = session
        .command("LAYOUT NEW Bad/Name")
        .expect_err("an illegal name is refused");
    assert!(illegal.contains("layout.invalid_name"), "{illegal}");
    session
        .command("LAYOUT NEW Sheet Two")
        .expect("a valid name creates the layout");
}

#[test]
fn layoutmerge_and_layoutwizard_apply_and_keep_their_name_grammar() {
    let session = session_with_layouts(&[
        (2, "Sheet One", None),
        (3, "Sheet Two", None),
        (4, "Dup", None),
    ]);
    let same = session
        .command("LAYOUTMERGE Dup Dup")
        .expect_err("same layouts are refused");
    assert!(same.contains("layoutmerge.same_layout"), "{same}");
    session
        .command("LAYOUTMERGE Dup Sheet One")
        .expect("LAYOUTMERGE moves and deletes the source row");

    session
        .command("LAYOUTWIZARD SheetThree A4 210 297")
        .expect("LAYOUTWIZARD creates the layout and reports the paper boundary");
    let bad_paper = session
        .command("LAYOUTWIZARD SheetThree A4 0 297")
        .expect_err("a zero paper width is refused before dispatch");
    assert!(bad_paper.contains("finite numbers > 0"), "{bad_paper}");
}

#[test]
fn vports_typed_form_and_the_dash_vports_alias_share_one_path() {
    assert_eq!(
        normalize_command("-VPORTS"),
        Some(CommandId::Vports),
        "the dash spelling is an alias, not a second command"
    );
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    let invalid = session
        .command("VPORTS 0 2")
        .expect_err("a zero row count is refused");
    assert!(
        invalid.contains("VPORTS needs <rows> <cols> >= 1"),
        "{invalid}"
    );
    let model = session
        .command("VPORTS 2 3")
        .expect_err("model space cannot host paper viewports");
    assert!(model.contains("vports.paper_space_required"), "{model}");

    switch_to_sheet(&session);
    for text in ["VPORTS 2 3", "-VPORTS 2 3"] {
        session
            .command(text)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
    }
}

#[test]
fn vpclip_rect_is_the_only_grammar_and_degenerate_rects_are_named() {
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    let polygon = session
        .command("VPCLIP POLY 0,0 10,0 10,10")
        .expect_err("a polygonal clip is NOT IMPLEMENTED");
    assert!(
        polygon.contains("vpclip.polygonal_not_implemented"),
        "{polygon}"
    );
    let degenerate = session
        .command("VPCLIP RECT 0 0 0 10")
        .expect_err("a zero-width rectangle is degenerate");
    assert!(
        degenerate.contains("vpclip.degenerate_rect"),
        "{degenerate}"
    );

    switch_to_sheet(&session);
    let valid = session
        .command("VPCLIP RECT 0 0 100 50")
        .expect_err("the fixture layout has no viewport to clip");
    assert!(valid.contains("vpclip.no_viewport"), "{valid}");
}

#[test]
fn vpscale_refuses_zero_non_finite_and_unknown_before_the_write() {
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    for (text, code) in [
        ("VPSCALE 0", "vpscale.zero_scale"),
        ("VPSCALE 1:0", "vpscale.zero_scale"),
        ("VPSCALE NaN", "vpscale.non_finite_scale"),
        ("VPSCALE -2", "vpscale.invalid_scale"),
    ] {
        let error = session.command(text).expect_err(text);
        assert!(error.contains(code), "{text}: {error}");
    }
    switch_to_sheet(&session);
    let ratio = session
        .command("VPSCALE 2:1")
        .expect_err("the fixture layout has no viewport to scale");
    assert!(ratio.contains("vpscale.no_viewport"), "{ratio}");
    let unknown = session
        .command("VPSCALE NOPE")
        .expect_err("an unknown named scale is refused");
    assert!(unknown.contains("vpscale.unknown_scale"), "{unknown}");
}

#[test]
fn layvpi_is_parsed_and_refused_without_ever_freezing_globally() {
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    switch_to_sheet(&session);
    let unknown = session
        .command("LAYVPI WALLS")
        .expect_err("this fixture has no such layer");
    assert!(unknown.contains("layvpi.unknown_layer"), "{unknown}");
    let empty = session
        .command("LAYVPI WALLS,")
        .expect_err("an empty comma entry is refused");
    assert!(empty.contains("comma-separated layer names"), "{empty}");
}

#[test]
fn vpmax_and_vpmin_are_payload_free_session_commands() {
    let session = session_with_layouts(&[(2, "Sheet One", None)]);
    let model = session
        .command("VPMAX")
        .expect_err("VPMAX needs a paper layout");
    assert!(model.contains("vpmax.paper_space_required"), "{model}");
    session
        .command("VPMIN")
        .expect("VPMIN with an empty state is a named no-op, not an error");
}
