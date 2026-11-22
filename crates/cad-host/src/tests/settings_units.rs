//! The `ui.units` / `ui.recovery` settings (ADR-0037 D9/D12).
//!
//! What is pinned here is the **host wiring**: the four closed keys, the
//! persisted leaves, the document-header write [`cad_app::host::HostController::set_units`]
//! performs (one revision, dirty, the next save's header) and the refusals that
//! leave both the preference and the document untouched.

use super::settings::{persisted, settings_core};
use super::*;
use cad_app::viewer_config::UnitsConfig;
use cad_domain::Revision;

/// The four keys are in the closed table exactly once, parse from their
/// camelCase wire spelling, are host-writable, and their defaults are the
/// schema's (drawing/drawing, 3, 60).
#[test]
fn the_units_and_recovery_keys_are_in_the_closed_table() {
    assert_eq!(
        CadSettingKey::ALL.len(),
        33,
        "the closed table grew with the persisted font directory"
    );
    for key in [
        CadSettingKey::DrawingUnit,
        CadSettingKey::DisplayUnit,
        CadSettingKey::UnitPrecision,
        CadSettingKey::RecoveryInterval,
    ] {
        assert_eq!(
            CadSettingKey::ALL
                .into_iter()
                .filter(|candidate| *candidate == key)
                .count(),
            1,
            "the key is declared exactly once"
        );
        assert_eq!(CadSettingKey::parse(key.key()), Some(key));
    }
    assert_eq!(CadSettingKey::DrawingUnit.key(), "drawingUnit");
    assert_eq!(CadSettingKey::DisplayUnit.key(), "displayUnit");
    assert_eq!(CadSettingKey::UnitPrecision.key(), "unitPrecision");
    assert_eq!(CadSettingKey::RecoveryInterval.key(), "recoveryInterval");
    for leaf in [
        "ui.units.drawingUnit",
        "ui.units.displayUnit",
        "ui.units.decimalPlaces",
        "ui.recovery.intervalSeconds",
    ] {
        assert!(
            HOST_ALLOWED_PATHS_JSON.contains(leaf),
            "{leaf} must be in the host's closed allowed-path policy"
        );
    }

    let core = core();
    let settings = core.settings();
    assert_eq!(settings.drawing_unit, "drawing");
    assert_eq!(settings.display_unit, "drawing");
    assert_eq!(settings.unit_precision, 3);
    assert_eq!(settings.recovery_interval, 60);
    // The snapshot is the effective config, never a second source.
    let effective = core.config_snapshot().effective;
    assert_eq!(effective.ui.units, UnitsConfig::default());
    assert_eq!(
        effective.ui.recovery.interval_seconds,
        cad_app::viewer_config::DEFAULT_RECOVERY_INTERVAL_SECONDS
    );
}

/// A units write persists all three leaves, applies the derived context to the
/// header (one revision, dirty) and reports it back; the same units again are a
/// no-op.
#[test]
fn a_units_write_bumps_the_header_once_and_persists_the_leaves() {
    let (core, config) = settings_core();
    let before = core
        .controller
        .borrow()
        .drawing()
        .expect("demo document")
        .revision();
    let generation = core.controller.borrow().session.generation;

    core.apply_setting(
        CadSettingKey::DrawingUnit,
        CadSettingValue::Text("mm".into()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::DisplayUnit,
        CadSettingValue::Text("cm".into()),
    )
    .unwrap();

    let after = core
        .controller
        .borrow()
        .drawing()
        .expect("demo document")
        .revision();
    assert_eq!(
        after,
        Revision(before.0 + 2),
        "each effective unit change is one header revision"
    );
    assert!(
        core.controller.borrow().is_dirty(),
        "a units change must become dirty so the next save writes the header"
    );
    assert!(
        core.controller.borrow().session.generation > generation,
        "the session generation advances like any other applied change"
    );

    let settings = core.settings();
    assert_eq!(settings.drawing_unit, "mm");
    assert_eq!(settings.display_unit, "cm");
    let written = persisted(&config).expect("the units write persists");
    assert!(written.contains(r#""drawingUnit":"mm""#), "{written}");
    assert!(written.contains(r#""displayUnit":"cm""#), "{written}");
    assert!(written.contains(r#""decimalPlaces":3"#), "{written}");

    // The document slot really holds the derived context.
    let units = {
        let controller = core.controller.borrow();
        controller
            .application
            .workspace
            .documents
            .get(&controller.document_id)
            .expect("document")
            .units
            .clone()
    };
    assert_eq!(units.display_per_source, Some(0.1));

    // Identical units are a no-op: no revision, no generation, and the
    // preference document that was already written does not change.
    let writes = persisted(&config);
    let revision = after;
    core.apply_setting(
        CadSettingKey::DisplayUnit,
        CadSettingValue::Text("cm".into()),
    )
    .unwrap();
    assert_eq!(
        core.controller
            .borrow()
            .drawing()
            .expect("demo document")
            .revision(),
        revision,
        "re-applying identical units must not manufacture an edit"
    );
    assert_eq!(
        persisted(&config),
        writes,
        "re-applying identical units leaves the durable preference unchanged"
    );
}

/// Switching the source to the untyped `drawing` token writes the only legal
/// display pairing in the same patch.
#[test]
fn drawing_units_also_resets_the_display_token() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::DrawingUnit,
        CadSettingValue::Text("mm".into()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::DisplayUnit,
        CadSettingValue::Text("cm".into()),
    )
    .unwrap();

    core.apply_setting(
        CadSettingKey::DrawingUnit,
        CadSettingValue::Text("drawing".into()),
    )
    .unwrap();
    let settings = core.settings();
    assert_eq!(settings.drawing_unit, "drawing");
    assert_eq!(
        settings.display_unit, "drawing",
        "a typed display over untyped coordinates is not a legal pair"
    );
    let written = persisted(&config).expect("write");
    assert!(written.contains(r#""drawingUnit":"drawing""#));
    assert!(written.contains(r#""displayUnit":"drawing""#));
}

/// A typed display unit over the untyped source is refused by name and changes
/// nothing.
#[test]
fn a_typed_display_over_untyped_coordinates_is_refused() {
    let (core, config) = settings_core();
    let revision = core
        .controller
        .borrow()
        .drawing()
        .expect("demo document")
        .revision();
    let error = core
        .apply_setting(
            CadSettingKey::DisplayUnit,
            CadSettingValue::Text("m".into()),
        )
        .unwrap_err();
    match &error {
        CadError::InvalidInput(reason) => assert!(
            reason.contains("ui.units.displayUnit"),
            "the refusal names the field: {reason}"
        ),
        other => panic!("expected the named refusal, got {other:?}"),
    }
    assert!(
        persisted(&config).is_none(),
        "a refused pair must not reach the preference file"
    );
    assert_eq!(core.settings().display_unit, "drawing");
    assert_eq!(
        core.controller
            .borrow()
            .drawing()
            .expect("demo document")
            .revision(),
        revision,
        "a refusal writes no header"
    );
}

/// Out-of-range precision and interval values are refused before any write.
#[test]
fn precision_and_interval_refuse_out_of_range_values() {
    let (core, config) = settings_core();
    for value in [9, 10_000] {
        let error = core
            .apply_setting(CadSettingKey::UnitPrecision, CadSettingValue::Number(value))
            .unwrap_err();
        assert!(matches!(
            &error,
            CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE
        ));
    }
    for value in [3601, 100_000] {
        let error = core
            .apply_setting(
                CadSettingKey::RecoveryInterval,
                CadSettingValue::Number(value),
            )
            .unwrap_err();
        assert!(matches!(
            &error,
            CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE
        ));
    }
    assert!(persisted(&config).is_none());
    assert_eq!(core.settings().unit_precision, 3);
    assert_eq!(core.settings().recovery_interval, 60);

    // An unknown unit token is the generic invalid value, never `drawing`.
    for token in ["MM", "metre", "", "drawing units"] {
        let error = core
            .apply_setting(
                CadSettingKey::DrawingUnit,
                CadSettingValue::Text(token.into()),
            )
            .unwrap_err();
        assert!(matches!(
            &error,
            CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE
        ));
    }
    assert_eq!(core.settings().drawing_unit, "drawing");

    // The accepted boundaries really are accepted.
    core.apply_setting(CadSettingKey::UnitPrecision, CadSettingValue::Number(8))
        .unwrap();
    core.apply_setting(
        CadSettingKey::RecoveryInterval,
        CadSettingValue::Number(3600),
    )
    .unwrap();
    assert_eq!(core.settings().unit_precision, 8);
    assert_eq!(core.settings().recovery_interval, 3600);
}

/// The units are document-header data: the snapshot the save path encodes
/// carries them, and installing that snapshot (the reopen path) keeps them.
#[test]
fn saved_units_survive_a_save_and_reopen() {
    let (core, _config) = settings_core();
    core.apply_setting(
        CadSettingKey::DrawingUnit,
        CadSettingValue::Text("in".into()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::DisplayUnit,
        CadSettingValue::Text("mm".into()),
    )
    .unwrap();
    core.apply_setting(CadSettingKey::UnitPrecision, CadSettingValue::Number(4))
        .unwrap();

    // The save path's own encode.
    let snapshot = core
        .controller
        .borrow()
        .snapshot_document()
        .expect("snapshot");
    assert_eq!(snapshot.header.units.source, cad_domain::Unit::Inch);
    assert_eq!(snapshot.header.units.display, cad_domain::Unit::Millimeter);
    assert_eq!(snapshot.header.units.display_per_source, Some(25.4));
    assert_eq!(snapshot.header.units.decimal_places, 4);
    let revision = snapshot.revision;

    // A fresh session installing that snapshot is the reopen.
    let mut reopened =
        cad_app::host::HostController::with_blank_document([800.0, 600.0]).expect("blank session");
    reopened
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::imported(),
            Some("plan.ymodel".into()),
        )
        .expect("install");
    assert_eq!(
        reopened.drawing().expect("reopened drawing").revision(),
        revision
    );
    let units = {
        let document = reopened
            .application
            .workspace
            .documents
            .get(&reopened.document_id)
            .expect("document");
        document.units.clone()
    };
    assert_eq!(units.source, cad_domain::Unit::Inch);
    assert_eq!(units.display, cad_domain::Unit::Millimeter);
    assert_eq!(units.display_per_source, Some(25.4));
    assert_eq!(units.decimal_places, 4);
}

/// A units write with no open document is refused by name rather than persisted
/// into a state nothing honours.
#[test]
fn a_units_write_without_a_document_is_refused() {
    let (core, config) = settings_core();
    core.controller.borrow_mut().close_document().unwrap();
    let error = core
        .apply_setting(
            CadSettingKey::DrawingUnit,
            CadSettingValue::Text("mm".into()),
        )
        .unwrap_err();
    assert!(
        matches!(&error, CadError::InvalidInput(reason) if reason.contains("no document")),
        "expected the explicit refusal, got {error:?}"
    );
    assert!(persisted(&config).is_none());
}

/// The snapshot reports the units the open document really carries.
///
/// A `.ymodel` open installs the archive's header, which can differ from the
/// preference; the dialog must show what the coordinates mean, and the
/// preference stays the value a later write applies.
#[test]
fn the_snapshot_reports_the_documents_real_units() {
    let units = UnitsConfig {
        drawing_unit: cad_app::viewer_config::UnitToken::Mm,
        display_unit: cad_app::viewer_config::UnitToken::Cm,
        decimal_places: 4,
    };
    let mut source =
        cad_app::host::HostController::with_blank_document([800.0, 600.0]).expect("blank");
    source
        .set_units(units.unit_context().expect("a legal pair"))
        .expect("set units");
    let snapshot = source.snapshot_document().expect("snapshot");

    let (core, _config) = settings_core();
    core.controller
        .borrow_mut()
        .install_snapshot(
            &snapshot,
            cad_app::PersistenceState::imported(),
            Some("plan.ymodel".into()),
        )
        .expect("install");
    let settings = core.settings();
    assert_eq!(settings.drawing_unit, "mm");
    assert_eq!(settings.display_unit, "cm");
    assert_eq!(settings.unit_precision, 4);
    // The preference itself is untouched: it is not a projection of the header.
    assert_eq!(
        core.config_snapshot().effective.ui.units,
        UnitsConfig::default()
    );
    assert_eq!(settings.recovery_interval, 60);
}
