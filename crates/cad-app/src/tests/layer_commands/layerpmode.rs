//! `LAYERPMODE` (AutoCAD 2026): the `LAYERP` snapshot-recording gate.
//!
//! AutoCAD saves `LAYERPMODE` in the drawing, so it is stored in the document
//! header (`document.json` → `variables.layer_pmode`, written only when Off).
//! `ON` (the default) preserves the existing behaviour; `OFF` suppresses the
//! snapshot push of a real override mutation without changing the applied
//! override. A bare `LAYERPMODE` reports the stored setting and writes nothing.

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

fn document_pmode(app: &Application) -> bool {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .layer_pmode()
}

#[test]
fn layerpmode_off_suppresses_layerp_snapshots_and_on_restores_them() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    assert!(
        document_pmode(&app),
        "ON is the default and the old archive value"
    );

    set_visible(&mut app, &mut session, LayerId(3), false);
    assert_eq!(session.layer_history_len(), 1, "ON records one LAYERP step");

    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(false),
    )
    .expect("LAYERPMODE OFF");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerpmode.applied"]);
    assert!(!document_pmode(&app));
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "OFF is one header revision"
    );
    assert_eq!(
        undo_depth(&app),
        before.1,
        "a header value records no undo step"
    );

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

#[test]
fn writing_the_stored_layerpmode_is_a_named_noop() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(true),
    )
    .expect("ON is already stored");
    assert_untouched(&app, before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["layerpmode.unchanged"]);

    execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(false),
    )
    .expect("OFF");
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(false),
    )
    .expect("OFF is already stored");
    assert_untouched(&app, before);
    assert_eq!(diagnostic_codes(&outcome), vec!["layerpmode.unchanged"]);
}

#[test]
fn layerpmode_refuses_a_foreign_payload_without_writing() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerVisibilities(vec![(LayerId(3), false)]),
    )
    .expect_err("only LAYERPMODE's own payload is accepted");
    assert!(refusal(error).starts_with("layerpmode.invalid_payload:"));
    assert_untouched(&app, before);
    assert!(document_pmode(&app), "the gate is untouched");
}

#[test]
fn layerpmode_round_trips_through_document_json() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    execute(
        &mut app,
        &mut session,
        CommandId::LayerPmode,
        CommandPayload::LayerPmode(false),
    )
    .expect("LAYERPMODE OFF");

    let document = &app.workspace.documents[&DocumentId(1)];
    let snapshot = document
        .drawing
        .snapshot(
            document.units.clone(),
            document.identity.clone(),
            document.resource_keys.clone(),
            document.drawing.revision(),
        )
        .expect("snapshot");
    assert_eq!(
        snapshot.header.variables.get("layer_pmode"),
        Some(&serde_json::Value::Bool(false)),
        "a non-default gate is written"
    );
    let loaded = cad_db::DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert!(!loaded.layer_pmode(), "the gate survives the round trip");

    // A default document writes no key at all: old archives stay identical.
    let (default_app, _session) = app_with(layers(), Vec::new());
    let default_document = &default_app.workspace.documents[&DocumentId(1)];
    let default_snapshot = default_document
        .drawing
        .snapshot(
            default_document.units.clone(),
            default_document.identity.clone(),
            default_document.resource_keys.clone(),
            default_document.drawing.revision(),
        )
        .expect("default snapshot");
    assert!(
        default_snapshot
            .header
            .variables
            .get("layer_pmode")
            .is_none(),
        "the default (On) gate must not be written"
    );
    assert!(
        cad_db::DrawingDatabase::from_snapshot(&default_snapshot)
            .expect("hydrate default")
            .layer_pmode(),
        "an absent key decodes to On"
    );
}
