//! `LAYERPMODE` (AutoCAD 2026): the `LAYERP` snapshot-recording gate.
//!
//! `ON` (the default) preserves the existing behaviour; `OFF` suppresses the
//! snapshot push of a real override mutation without changing the applied
//! override. A bare `LAYERPMODE` reports the current setting and writes nothing.

use super::*;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
        (LayerId(6), "ROOF", LayerAttributes::default()),
    ]
}

fn set_visible(app: &mut Application, session: &mut SessionState, layer: LayerId, visible: bool) {
    execute(
        app,
        session,
        CommandId::SetLayerVisibilities,
        CommandPayload::LayerVisibilities(vec![(layer, visible)]),
    )
    .expect("a valid visibility batch");
}

#[test]
fn layerpmode_off_suppresses_layerp_snapshots_and_on_restores_them() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    assert!(session.layer_states.snapshot_enabled(), "ON is the default");

    set_visible(&mut app, &mut session, LayerId(3), false);
    assert_eq!(session.layer_history_len(), 1, "ON records one LAYERP step");

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(false),
    )
    .expect("LAYERPMODE OFF");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerpmode.applied"]);
    assert!(!session.layer_states.snapshot_enabled());

    set_visible(&mut app, &mut session, LayerId(5), false);
    assert_eq!(
        session.layer_history_len(),
        1,
        "OFF records nothing, but the override is still applied"
    );
    assert_eq!(session.layer_overrides.get(LayerId(5)), Some(false));

    execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(true),
    )
    .expect("LAYERPMODE ON");
    set_visible(&mut app, &mut session, LayerId(6), false);
    assert_eq!(session.layer_history_len(), 2, "ON records again");
}

#[test]
fn bare_layerpmode_reports_the_current_setting_without_writing() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::None,
    )
    .expect("bare LAYERPMODE reports");
    assert_untouched(&app, before);
    assert_eq!(diagnostic_codes(&outcome), vec!["layerpmode.report"]);
    assert!(outcome.diagnostics[0].message.contains("ON"));
}
