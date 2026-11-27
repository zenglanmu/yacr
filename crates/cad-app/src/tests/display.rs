//! `SHADOWS`/`FOG` command contracts.
//!
//! A write replaces the whole document display record (`tables/display.json`) as
//! one revision with one undo step; a bare selector reports and writes nothing; a
//! write that changes nothing answers `display.unchanged`; and every value is
//! re-validated here so a direct payload caller gets the same refusal.

use super::*;
use cad_domain::display::{DisplaySettings, FogSettings, ShadowSettings};

fn display(
    app: &mut Application,
    session: &mut SessionState,
    option: DisplayCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Shadows, CommandPayload::Display(option)),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn settings(app: &Application) -> DisplaySettings {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .display_settings()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, cad_history::History::undo_depth)
}

fn code<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected diagnostic {code}: {:?}", outcome.diagnostics))
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn shadows_on_is_one_revision_and_one_undo_step() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = display(&mut app, &mut session, DisplayCommand::Shadows(true)).expect("shadows");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(undo_depth(&app), 1);
    assert!(settings(&app).shadows.enabled);
    code(&outcome, "display.saved");

    // Undo restores the default record through the display replay branch; the
    // table write is a fresh revision, like every other table replay.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(!settings(&app).shadows.enabled);
    assert_eq!(settings(&app), DisplaySettings::default());
    assert_eq!(undo_depth(&app), 0);
    assert!(revision(&app).0 > before.0, "undo is itself a table write");
}

#[test]
fn a_sun_override_keeps_the_switch_and_stores_both_angles() {
    let (mut app, mut session) = application_with_document();
    display(&mut app, &mut session, DisplayCommand::Shadows(true)).expect("shadows on");
    display(
        &mut app,
        &mut session,
        DisplayCommand::Sun {
            azimuth: 90.0,
            elevation: 45.0,
        },
    )
    .expect("sun");
    let record = settings(&app);
    assert!(record.shadows.enabled);
    assert_eq!(record.shadows.azimuth, 90.0);
    assert_eq!(record.shadows.elevation, 45.0);
    // Azimuth 90, elevation 45: a unit vector toward +X at 45° above the horizon.
    let sun = record.shadows.sun_direction().expect("enabled");
    let horizontal = std::f64::consts::FRAC_1_SQRT_2;
    assert!((sun[0] - horizontal).abs() < 1e-9, "{sun:?}");
    assert!(sun[1].abs() < 1e-9, "{sun:?}");
    assert!((sun[2] - horizontal).abs() < 1e-9, "{sun:?}");
}

#[test]
fn fog_toggles_range_and_colour_are_writes_and_a_report_is_not() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    display(
        &mut app,
        &mut session,
        DisplayCommand::FogRange {
            start: 2.0,
            end: 20.0,
        },
    )
    .expect("range");
    display(&mut app, &mut session, DisplayCommand::FogColor([9, 8, 7])).expect("colour");
    display(&mut app, &mut session, DisplayCommand::Fog(true)).expect("on");
    let record = settings(&app);
    assert!(record.fog.enabled);
    assert_eq!((record.fog.start, record.fog.end), (2.0, 20.0));
    assert_eq!(record.fog.color, [9, 8, 7]);
    assert_eq!(revision(&app), Revision(before.0 + 3));

    // A bare selector reports the live record and writes nothing.
    let revision_now = revision(&app);
    let outcome = display(&mut app, &mut session, DisplayCommand::ReportFog).expect("report");
    assert_eq!(revision(&app), revision_now);
    let diagnostic = code(&outcome, "display.fog");
    assert!(diagnostic.message.contains("fog on"), "{diagnostic:?}");
    assert!(
        diagnostic.message.contains("2.000..20.000"),
        "{diagnostic:?}"
    );
}

#[test]
fn an_unchanged_write_is_a_named_no_op() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let outcome = display(&mut app, &mut session, DisplayCommand::Shadows(false))
        .expect("the record is already off");
    assert_eq!(revision(&app), before, "nothing was written");
    code(&outcome, "display.unchanged");
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn out_of_range_values_are_refused_by_the_domain_marker() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = display(
        &mut app,
        &mut session,
        DisplayCommand::Sun {
            azimuth: 400.0,
            elevation: 10.0,
        },
    )
    .expect_err("azimuth outside [0, 360)");
    assert!(refusal_text(error).contains("display.invalid_value"));
    let error = display(
        &mut app,
        &mut session,
        DisplayCommand::FogRange {
            start: 20.0,
            end: 10.0,
        },
    )
    .expect_err("reversed fog band");
    assert!(refusal_text(error).contains("display.invalid_value"));
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn viewer_mode_refuses_both_selectors() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    let error = display(&mut app, &mut session, DisplayCommand::Fog(true))
        .expect_err("a document write is work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    let error = display(&mut app, &mut session, DisplayCommand::ReportShadows)
        .expect_err("the selector itself is work-only");
    assert!(matches!(error, CadError::PermissionDenied), "{error:?}");
    let _ = FogSettings::default();
    let _ = ShadowSettings::default();
}
