//! The `interaction.orbitSensitivityPercent` 3D orbit preference.
//!
//! Like the 2D `interaction.leftDragPans` scheme this is a plain preference: the
//! orbit gesture reads the effective config on every pointer event, so the
//! durable write *is* the live application and there is no session state to seed.
//! The contracts below cover the closed key table, the persisted leaf, the range
//! refusals, the startup/replace path, the write-failure rollback — and, because
//! the rate itself is the user-visible behaviour, the **measured** camera
//! rotation a real primary drag produces at 50/100/200 %.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// The base rate at 100 %: radians per logical pixel of orbit drag.
const BASE_RADIANS_PER_PIXEL: f64 = 0.0035;

// --- a core whose preference writes always fail -----------------------------

/// A store that reads nothing and refuses every write, so the rollback path of a
/// settings write is observable without a real filesystem.
struct RejectingPreferenceStore;

impl ConfigStore for RejectingPreferenceStore {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn persist_preference(&self, _text: &str) -> CadResult<()> {
        Err(CadError::InvalidInput(
            "preference store is read-only".into(),
        ))
    }
}

// --- the real gesture -------------------------------------------------------

fn set_three_d(core: &HostCore) {
    let (viewport_id, camera) = {
        let c = core.controller.borrow();
        (
            c.viewport_id,
            c.application.workspace.viewports[&c.viewport_id].camera,
        )
    };
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .view_mode = cad_app::ViewMode2d3d::ThreeD { saved_2d: camera };
}

fn pointer_request(
    core: &HostCore,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(1),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn primary(core: &HostCore, phase: cad_app::interaction::PointerPhase, position: [f64; 2]) {
    dispatch_pointer(core, pointer_request(core, phase, position));
}

/// Tilt the 3D camera off the top-down axis (`SWITCH2D3D` preserves the 2D view
/// direction, so the default 3D eye sits on `+Z` where a yaw about world `+Z` is
/// a no-op). An isometric-ish eye makes the azimuth a real, measurable yaw.
fn tilt_view(core: &HostCore) {
    let mut controller = core.controller.borrow_mut();
    let viewport_id = controller.viewport_id;
    let viewport = controller
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .expect("the active viewport exists");
    let target = viewport.camera.target;
    let distance = viewport.camera.distance().max(1.0);
    viewport.camera.eye = Point3 {
        x: target.x + distance,
        y: target.y - distance,
        z: target.z + distance,
    };
}

/// The camera's world-`+Z` azimuth (`atan2(offset.y, offset.x)`).
fn azimuth(core: &HostCore) -> f64 {
    let camera = active_camera(core);
    let offset = [
        camera.eye.x - camera.target.x,
        camera.eye.y - camera.target.y,
    ];
    offset[1].atan2(offset[0])
}

/// Yaw measured from a real 3D primary drag of `dx` logical pixels (pure
/// horizontal, so the pitch stays 0 and the azimuth delta is exactly the yaw).
fn measured_yaw(percent: f64, dx: f64) -> f64 {
    let (core, _config) = settings_core();
    core.apply_setting(
        CadSettingKey::OrbitSensitivityPercent,
        CadSettingValue::Real(percent),
    )
    .expect("an in-range sensitivity is accepted");
    set_three_d(&core);
    tilt_view(&core);
    let before = azimuth(&core);
    primary(
        &core,
        cad_app::interaction::PointerPhase::Down,
        [100.0, 100.0],
    );
    primary(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [100.0 + dx, 100.0],
    );
    primary(
        &core,
        cad_app::interaction::PointerPhase::Up,
        [100.0 + dx, 100.0],
    );
    let after = azimuth(&core);
    // The azimuth wraps at +/- pi; a 100 px drag is far below a half turn.
    let mut delta = after - before;
    while delta > std::f64::consts::PI {
        delta -= std::f64::consts::TAU;
    }
    while delta < -std::f64::consts::PI {
        delta += std::f64::consts::TAU;
    }
    delta
}

// --- the closed table -------------------------------------------------------

/// The key is in the one closed table, its wire spelling is the camelCase leaf,
/// and the leaf is host-writable (otherwise the write path would reject it).
#[test]
fn the_orbit_sensitivity_key_is_in_the_closed_table() {
    let key = CadSettingKey::OrbitSensitivityPercent;
    assert_eq!(
        CadSettingKey::ALL.len(),
        33,
        "one writable key per contract"
    );
    assert_eq!(
        CadSettingKey::ALL
            .into_iter()
            .filter(|candidate| *candidate == key)
            .count(),
        1,
        "the key is declared exactly once"
    );
    assert_eq!(key.key(), "orbitSensitivityPercent");
    assert_eq!(CadSettingKey::parse("orbitSensitivityPercent"), Some(key));
    assert_eq!(
        CadSettingKey::parse("interaction.orbitSensitivityPercent"),
        None,
        "the wire key is the leaf, not the dotted path"
    );
    assert_eq!(
        CadSettingKey::parse("orbit_sensitivity_percent"),
        None,
        "the wire spelling is camelCase, like the sibling interaction keys"
    );
    assert!(
        HOST_ALLOWED_PATHS_JSON.contains(r#""interaction.orbitSensitivityPercent""#),
        "the leaf must be listed by name in the host's closed allowed-path policy"
    );

    // The default is the effective config's value (100 %), never a local guess.
    let core = core();
    assert_eq!(core.settings().orbit_sensitivity_percent, 100.0);
    assert_eq!(core.orbit_sensitivity_percent(), 100.0);
    assert_eq!(
        core.config_snapshot()
            .effective
            .interaction
            .orbit_sensitivity_percent,
        100.0,
        "an absent preference is the schema's `100` default"
    );
}

/// A write persists `interaction.orbitSensitivityPercent`, the snapshot and the
/// gesture reader report it, and a fresh startup reads the same value back.
#[test]
fn an_orbit_sensitivity_write_persists_and_reloads() {
    let (core, config) = settings_core();

    core.apply_setting(
        CadSettingKey::OrbitSensitivityPercent,
        CadSettingValue::Real(150.0),
    )
    .unwrap();
    assert_eq!(core.settings().orbit_sensitivity_percent, 150.0);
    assert_eq!(
        core.orbit_sensitivity_percent(),
        150.0,
        "the gesture reader reads the effective config"
    );
    let written = persisted(&config).expect("the write persists");
    let document: serde_json::Value =
        serde_json::from_str(&written).expect("the persisted preference is JSON");
    assert_eq!(
        document["interaction"]["orbitSensitivityPercent"].as_f64(),
        Some(150.0),
        "the percentage is persisted under interaction.orbitSensitivityPercent: {written}"
    );

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert!(
        reloaded.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        reloaded.startup_diagnostics.borrow()
    );
    assert_eq!(reloaded.settings(), core.settings());
    assert_eq!(reloaded.orbit_sensitivity_percent(), 150.0);

    // An integer wire value writes the same leaf (one entry, not a second one).
    core.apply_setting(
        CadSettingKey::OrbitSensitivityPercent,
        CadSettingValue::Number(50),
    )
    .unwrap();
    assert_eq!(core.settings().orbit_sensitivity_percent, 50.0);
    let written = persisted(&config).expect("the second write persists");
    assert!(
        written.contains(r#""orbitSensitivityPercent":50"#),
        "{written}"
    );
    assert_eq!(
        written.matches("\"orbitSensitivityPercent\"").count(),
        1,
        "{written}"
    );
}

/// The schema range is inclusive at both ends and a value outside it (or a
/// non-finite one) is refused with the stable code without touching the file.
#[test]
fn orbit_sensitivity_accepts_25_to_200_and_refuses_everything_else() {
    let (core, _config) = settings_core();

    for accepted in [
        CadSettingValue::Real(25.0),
        CadSettingValue::Real(200.0),
        CadSettingValue::Number(25),
        CadSettingValue::Number(200),
    ] {
        core.apply_setting(CadSettingKey::OrbitSensitivityPercent, accepted.clone())
            .unwrap_or_else(|error| panic!("{accepted:?} is in range: {error:?}"));
    }
    assert_eq!(core.settings().orbit_sensitivity_percent, 200.0);

    let before = core.settings();
    for refused in [
        CadSettingValue::Real(24.9),
        CadSettingValue::Real(200.1),
        CadSettingValue::Real(f64::NAN),
        CadSettingValue::Real(f64::INFINITY),
        CadSettingValue::Number(24),
        CadSettingValue::Number(201),
        CadSettingValue::Bool(true),
        CadSettingValue::Text("100".to_string()),
    ] {
        let error = core
            .apply_setting(CadSettingKey::OrbitSensitivityPercent, refused.clone())
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{refused:?} must be refused with the stable code: {error:?}"
        );
    }
    assert_eq!(
        core.settings(),
        before,
        "a refused write changes nothing that is visible"
    );
}

/// A startup preference reaches the gesture reader with no seeding step, and it
/// survives a document replacement (which rebuilds `SessionState`).
#[test]
fn a_startup_orbit_sensitivity_survives_without_seeding() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"orbitSensitivityPercent":50}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();

    assert!(
        core.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        core.startup_diagnostics.borrow()
    );
    assert_eq!(core.settings().orbit_sensitivity_percent, 50.0);
    assert_eq!(core.orbit_sensitivity_percent(), 50.0);

    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert_eq!(
        core.orbit_sensitivity_percent(),
        50.0,
        "the persisted sensitivity survives open/new"
    );

    // An absent preference keeps the documented base rate.
    let defaulted = core_with_settings(Rc::new(SettingsConfig::default()));
    defaulted.load_startup();
    assert_eq!(defaulted.orbit_sensitivity_percent(), 100.0);
}

/// A durable document that was edited past validation (out of the accepted
/// range) is refused at load and the effective rate stays the 100 % base rate —
/// it is never reported as an effective sensitivity the orbit path would not use.
#[test]
fn an_invalid_persisted_value_falls_back_to_the_base_rate() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"orbitSensitivityPercent":900}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();

    assert!(
        !core.startup_diagnostics.borrow().is_empty(),
        "an out-of-range persisted value is reported, not silently accepted"
    );
    assert_eq!(
        core.orbit_sensitivity_percent(),
        100.0,
        "the invalid leaf never becomes an effective rate"
    );
    assert_eq!(core.settings().orbit_sensitivity_percent, 100.0);
}

/// A write that cannot be made durable is rolled back in memory and reported as
/// `settings.not_persisted`, so the read-back never shows a value that would
/// disappear on the next start.
#[test]
fn a_failed_orbit_sensitivity_write_rolls_back() {
    let core = core_with_config(Rc::new(RejectingPreferenceStore));
    assert_eq!(core.orbit_sensitivity_percent(), 100.0);

    let error = core
        .apply_setting(
            CadSettingKey::OrbitSensitivityPercent,
            CadSettingValue::Real(200.0),
        )
        .expect_err("the store refuses the write");
    assert!(
        matches!(&error, CadError::Unsupported(reason) if reason.starts_with(SETTINGS_NOT_PERSISTED)),
        "{error:?}"
    );
    assert_eq!(
        core.orbit_sensitivity_percent(),
        100.0,
        "the rejected write left no trace"
    );
    assert_eq!(core.settings().orbit_sensitivity_percent, 100.0);
}

// --- the actual rate --------------------------------------------------------

/// `percent = 100` is the base rate: a 100 px horizontal drag yaws by exactly
/// `0.35` rad (`0.0035 * 100`), as the real orbit path emits it.
#[test]
fn a_hundred_pixel_drag_at_100_percent_yaws_the_base_rate() {
    let yaw = measured_yaw(100.0, 100.0);
    let expected = -(100.0 * BASE_RADIANS_PER_PIXEL);
    assert!(
        (yaw - expected).abs() < 1e-9,
        "100 px right at 100 % must yaw {expected} rad, measured {yaw}"
    );
}

/// `50` % halves and `200` % doubles the same drag, and the base rate lies
/// between them — the percentage is a real multiplier, not a reported number.
#[test]
fn the_percentage_scales_the_measured_rotation_linearly() {
    let half = measured_yaw(50.0, 100.0);
    let base = measured_yaw(100.0, 100.0);
    let double = measured_yaw(200.0, 100.0);
    let base_expected = -(100.0 * BASE_RADIANS_PER_PIXEL);

    assert!(
        (base - base_expected).abs() < 1e-9,
        "100 % must be the base rate: measured {base}, expected {base_expected}"
    );
    assert!(
        (half - base / 2.0).abs() < 1e-9,
        "50 % must halve the base rotation: measured {half}, base {base}"
    );
    assert!(
        (double - base * 2.0).abs() < 1e-9,
        "200 % must double the base rotation: measured {double}, base {base}"
    );
    assert!(
        (double - -(200.0 * BASE_RADIANS_PER_PIXEL)).abs() < 1e-9,
        "200 % of a 100 px drag is 0.70 rad: measured {double}"
    );
}
