//! AutoCAD's typed `ZOOM <argument>`: the scale factor and `ZOOM ALL`.
//!
//! The argument form is a **selector argument**, not a new command: a positive
//! factor reuses the `ZOOMIN`/`ZOOMOUT` step (anchored at the viewport centre) and
//! `ALL`/`EXTENTS` is the ordinary `FIT`. These tests pin the three ways that can
//! rot: a factor that changes the camera by something other than itself, a
//! refused argument that still moved the camera, and a fabricated default factor
//! for a bare `ZOOM`.

use super::super::super::*;
use super::super::test_support::*;
use crate::shell::{admit_command, ShellState};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// The projected 2D camera scale (world units per logical pixel).
fn camera_scale(session: &CadSession) -> f64 {
    session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan")
        .scale
}

/// Assert the session recorded no undo step (a camera move is session state).
fn assert_no_history(session: &CadSession) {
    let c = session.core.controller.borrow();
    assert!(
        !c.application.can_undo(&c.document_id),
        "a zoom must not record an undo step"
    );
    assert!(
        !c.application.can_redo(&c.document_id),
        "a zoom must not record a redo step"
    );
}

#[test]
fn zoom_with_a_factor_scales_the_camera_about_the_centre() {
    let session = fresh_session();
    let base = camera_scale(&session);
    let centre = session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan")
        .center;

    session.command("ZOOM 2").expect("ZOOM 2 is admitted");
    let zoomed_in = camera_scale(&session);
    assert!(
        (zoomed_in - base / 2.0).abs() < 1e-9,
        "a factor > 1 zooms in by exactly that factor: {base} -> {zoomed_in}"
    );
    let after = session
        .snapshot(None, None, 0)
        .camera
        .expect("still a 2D plan");
    assert_eq!(after.center, centre, "the zoom is anchored at the centre");
    assert_no_history(&session);

    // The inverse factor restores the original framing.
    session.command("ZOOM 0.5").expect("ZOOM 0.5 is admitted");
    let zoomed_out = camera_scale(&session);
    assert!(
        (zoomed_out - base).abs() < 1e-9,
        "a factor < 1 zooms out by exactly that factor: {zoomed_in} -> {zoomed_out}"
    );
    assert_no_history(&session);
}

#[test]
fn zoom_factors_are_the_same_convention_as_the_button_steps() {
    // The factor form is the button step with a caller-supplied factor, so
    // `ZOOM <ZOOM_IN_FACTOR>` must land exactly where `ZOOMIN` lands.
    let typed = fresh_session();
    let button = fresh_session();
    let base = camera_scale(&typed);
    typed
        .command(&format!("ZOOM {ZOOM_IN_FACTOR}"))
        .expect("ZOOM <ZOOM_IN_FACTOR> is admitted");
    button.command("ZOOMIN").expect("ZOOMIN is admitted");
    assert!(
        (camera_scale(&typed) - camera_scale(&button)).abs() < 1e-9,
        "ZOOM n and ZOOMIN must share one zoom path"
    );
    assert!((camera_scale(&button) - base / ZOOM_IN_FACTOR).abs() < 1e-9);
}

#[test]
fn zoom_all_is_the_fit_path() {
    let zoom_all = fresh_session();
    let fit = fresh_session();
    // Zoom away first so `ZOOM ALL` has a different framing to restore.
    zoom_all.command("ZOOM 3").expect("ZOOM 3 is admitted");

    zoom_all.command("ZOOM ALL").expect("ZOOM ALL is admitted");
    fit.command("FIT").expect("FIT is admitted");
    let all = zoom_all
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");
    let fitted = fit
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");
    assert_eq!(all.scale, fitted.scale, "ZOOM ALL must frame like FIT");
    assert_eq!(all.center, fitted.center, "ZOOM ALL must centre like FIT");
    assert_no_history(&zoom_all);
}

#[test]
fn zoom_extents_still_resolves_through_the_existing_alias() {
    // `ZOOM EXTENTS` is the pre-existing alias onto `FIT`; the new `ZOOM ALL`
    // spelling must not disturb it.
    assert_eq!(canonical_command_name("ZOOM EXTENTS"), Some("FIT"));
    let session = fresh_session();
    session
        .command("ZOOM EXTENTS")
        .expect("ZOOM EXTENTS is admitted");
    let extents = session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");

    let fit = fresh_session();
    fit.command("FIT").expect("FIT is admitted");
    let fitted = fit
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");
    assert_eq!(extents.scale, fitted.scale);
    assert_eq!(extents.center, fitted.center);
}

#[test]
fn whitespace_between_the_name_and_the_argument_is_tolerated() {
    let plain = fresh_session();
    plain.command("ZOOM 2").expect("ZOOM 2 is admitted");
    let expected = camera_scale(&plain);
    for text in ["zoom   2", "  ZOOM 2  ", "zoom\t2", "Zoom 2"] {
        let spaced = fresh_session();
        let base = camera_scale(&spaced);
        spaced
            .command(text)
            .unwrap_or_else(|error| panic!("{text:?} refused: {error}"));
        assert!(
            (camera_scale(&spaced) - base / 2.0).abs() < 1e-9,
            "{text:?} must apply the factor 2 like `ZOOM 2`"
        );
        assert!(
            (camera_scale(&spaced) - expected).abs() < 1e-9,
            "{text:?} must land exactly where `ZOOM 2` lands"
        );
    }
}

#[test]
fn an_argument_outside_the_grammar_is_refused_without_moving_the_camera() {
    let session = fresh_session();
    let base = camera_scale(&session);
    for text in [
        "ZOOM 0", "ZOOM -1", "ZOOM abc", "ZOOM 2 3", "ZOOM .5", "ZOOM NaN",
    ] {
        let error = session
            .command(text)
            .expect_err(&format!("{text:?} must be refused"));
        assert!(
            error.contains("ZOOM needs a positive factor, ALL, EXTENTS, WINDOW or PREVIOUS"),
            "{text:?} must carry the named reason, got {error:?}"
        );
        assert!(
            error.contains(text.trim()),
            "{text:?} must echo the typed line, got {error:?}"
        );
        assert_eq!(
            camera_scale(&session),
            base,
            "{text:?} must leave the camera untouched"
        );
        // The refusal is recorded for `cad_status.error`, never a silent ignore.
        assert_eq!(
            session.snapshot(None, None, 0).error.as_deref(),
            Some(error.as_str()),
            "{text:?} must be visible in cad_status.error"
        );
    }
    assert_no_history(&session);
}

#[test]
fn the_argument_grammar_is_exactly_the_documented_one() {
    assert_eq!(parse_zoom_argument("2"), Some(ZoomArgument::Factor(2.0)));
    assert_eq!(parse_zoom_argument("0.5"), Some(ZoomArgument::Factor(0.5)));
    assert_eq!(
        parse_zoom_argument("1.25"),
        Some(ZoomArgument::Factor(1.25))
    );
    assert_eq!(parse_zoom_argument("2.0"), Some(ZoomArgument::Factor(2.0)));
    assert_eq!(parse_zoom_argument("ALL"), Some(ZoomArgument::Fit));
    assert_eq!(parse_zoom_argument("all"), Some(ZoomArgument::Fit));
    assert_eq!(parse_zoom_argument("EXTENTS"), Some(ZoomArgument::Fit));
    for refused in ["0", "0.0", "-1", "+2", "abc", "2 3", ".5", "2.", "1e3", ""] {
        assert_eq!(parse_zoom_argument(refused), None, "{refused:?}");
    }

    // Only `ZOOM` carries a typed argument; every other selector stays name-only.
    assert_eq!(typed_command_argument("ZOOM 2"), Some(("ZOOM", "2")));
    assert_eq!(
        typed_command_argument("  zoom\t0.5  "),
        Some(("ZOOM", "0.5"))
    );
    assert_eq!(typed_command_argument("ZOOM ALL"), Some(("ZOOM", "ALL")));
    assert_eq!(
        typed_command_argument("ZOOM"),
        None,
        "a bare name has no argument"
    );
    assert_eq!(typed_command_argument("LINE 2"), None);
    assert_eq!(typed_command_argument(""), None);
}

#[test]
fn bare_zoom_keeps_the_name_only_payload_refusal() {
    let session = fresh_session();
    let base = camera_scale(&session);
    // `ZOOM` alone is a payload-bearing selector: it is not given a fabricated
    // default factor, and it does not silently become a no-op.
    let error = session.command("ZOOM").expect_err("bare ZOOM is refused");
    assert!(!error.is_empty(), "the refusal carries a real reason");
    assert_eq!(camera_scale(&session), base, "nothing moved");
}

#[test]
fn a_name_only_selector_with_an_argument_is_still_an_unknown_name() {
    let session = fresh_session();
    let base = camera_scale(&session);
    let error = session
        .command("LINE 2")
        .expect_err("LINE takes no argument");
    assert!(
        error.contains("unsupported CAD command"),
        "an argument on a name-only selector is the ordinary refusal: {error:?}"
    );
    assert_eq!(camera_scale(&session), base);
    assert_eq!(
        session.snapshot(None, None, 0).tool,
        None,
        "nothing was started"
    );
}

#[test]
fn shell_admission_queues_the_full_typed_argument_line() {
    let mut state = ShellState::new(true);
    assert!(admit_command(&mut state, "zoom 2").is_ok());
    assert_eq!(state.cad_commands, vec!["ZOOM 2".to_string()]);
    // An out-of-grammar value is still admitted by name: the session validates
    // the value (and records the refusal for `cad_status.error`).
    assert!(admit_command(&mut state, "ZOOM 0").is_ok());
    assert_eq!(
        state.cad_commands.last().map(String::as_str),
        Some("ZOOM 0")
    );
    // A name-only selector with an argument is refused at admission.
    assert!(admit_command(&mut state, "LINE 2").is_err());
    assert_eq!(state.cad_commands.len(), 2);
}
