use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

// --- renderer backend preference (`wgpu_backend`) ---------------------------

/// A store whose persistence always fails, so a write's rollback and error
/// reporting are observable without a real filesystem.
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

/// The closed token set is exactly `auto|vulkan|dx12`. Every other string — case
/// variants, other graphics APIs, trailing whitespace — and every non-string is
/// refused with the stable code, never coerced to the default.
#[test]
fn wgpu_backend_accepts_only_the_three_wire_tokens() {
    let key = CadSettingKey::WgpuBackend;
    assert_eq!(
        key.key(),
        "wgpu_backend",
        "the wire key, the `cad_set_setting` key and the persisted leaf are identical"
    );
    assert_eq!(WGPU_BACKEND_PREFERENCE_KEY, "wgpu_backend");
    assert_eq!(
        WGPU_BACKEND_VALUES,
        ["auto", "vulkan", "dx12"],
        "`auto` stays first: it is the default"
    );
    for token in WGPU_BACKEND_VALUES {
        assert_eq!(
            key.validate(&CadSettingValue::Text(token.to_string())),
            Ok(()),
            "{token} is a wire value"
        );
    }
    for token in [
        "", "Auto", "AUTO", "Vulkan", "DX12", "gl", "metal", "webgpu", "vulkan ", " cpu",
    ] {
        assert_eq!(
            key.validate(&CadSettingValue::Text(token.to_string())),
            Err(SETTINGS_INVALID_VALUE),
            "{token:?} must be refused, not normalized"
        );
    }
    for value in [CadSettingValue::Bool(true), CadSettingValue::Number(1)] {
        assert_eq!(key.validate(&value), Err(SETTINGS_INVALID_VALUE));
    }
    // `parse` learns the key from the same spelling.
    assert_eq!(
        CadSettingKey::parse("wgpu_backend"),
        Some(CadSettingKey::WgpuBackend)
    );
    assert_eq!(CadSettingKey::parse("wgpuBackend"), None);
}

/// With nothing persisted the snapshot reports the `auto` default, and a rejected
/// value never changes it.
#[test]
fn wgpu_backend_defaults_to_auto_when_unset() {
    let (core, config) = settings_core();
    assert_eq!(core.settings().wgpu_backend, "auto");
    assert!(persisted(&config).is_none(), "reading does not persist");

    let error = core
        .apply_setting(
            CadSettingKey::WgpuBackend,
            CadSettingValue::Text("gl".to_string()),
        )
        .expect_err("an unknown token is refused");
    assert!(
        matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
        "the refusal is the stable code, not a coercion: {error:?}"
    );
    assert_eq!(core.settings().wgpu_backend, "auto");
    assert!(persisted(&config).is_none(), "a refusal writes nothing");
}

/// A write lands as the **root** `wgpu_backend` leaf of the same preference
/// document the hosts' presenters read, and the snapshot plus a reload report the
/// very same token.
#[test]
fn wgpu_backend_write_roundtrips_through_the_preference_store() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::WgpuBackend,
        CadSettingValue::Text("dx12".to_string()),
    )
    .expect("a wire token is accepted");
    assert_eq!(core.settings().wgpu_backend, "dx12");

    let written = persisted(&config).expect("the write persists");
    let document: serde_json::Value = serde_json::from_str(&written).expect("persisted JSON");
    assert_eq!(
        document.get("wgpu_backend"),
        Some(&serde_json::Value::String("dx12".to_string())),
        "the leaf is root-level and spelled like the readers' key: {written}"
    );

    // The presenters read the file, so a reload must report the same value — and
    // the leaf must not make the rest of the document "rejected".
    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert!(
        reloaded.startup_diagnostics.borrow().is_empty(),
        "the backend leaf is stripped before the strict allowed-path check: {:?}",
        reloaded.startup_diagnostics.borrow()
    );
    assert_eq!(reloaded.settings().wgpu_backend, "dx12");
}

/// An unrelated setting write re-persists the allowed-leaf projection; the
/// backend leaf is merged back into it, so it is never dropped.
#[test]
fn unrelated_preference_writes_keep_the_backend_leaf() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::WgpuBackend,
        CadSettingValue::Text("vulkan".to_string()),
    )
    .unwrap();
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();

    let written = persisted(&config).expect("the grid write persists");
    let document: serde_json::Value = serde_json::from_str(&written).expect("persisted JSON");
    assert_eq!(
        document.get("wgpu_backend"),
        Some(&serde_json::Value::String("vulkan".to_string())),
        "the backend outlives an unrelated write: {written}"
    );

    // A restart keeps both the backend and the unrelated setting.
    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings().wgpu_backend, "vulkan");
    assert!(reloaded.settings().grid);
}

/// A preference document that also carries the backend leaf loads normally: the
/// leaf is captured and stripped before the strict allowed-path validation, so
/// the other leaves still apply.
#[test]
fn startup_captures_the_backend_leaf_and_still_applies_the_other_leaves() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"wgpu_backend":"dx12","view":{"overlays":{"grid":true}}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    assert!(
        core.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        core.startup_diagnostics.borrow()
    );
    assert_eq!(core.settings().wgpu_backend, "dx12");
    assert!(core.settings().grid, "the other leaves are still applied");
}

/// An unknown persisted token is reported as `auto` — the honest default, not an
/// invented value — and the document is not rejected because of it.
#[test]
fn unknown_persisted_backend_reports_auto_without_rejecting_the_file() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"wgpu_backend":"gl","view":{"overlays":{"grid":true}}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    assert!(
        core.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        core.startup_diagnostics.borrow()
    );
    assert_eq!(core.settings().wgpu_backend, "auto");
    assert!(core.settings().grid);
}

/// Clearing the user preference is a real clear: the rendered value returns to
/// `auto` and the durable leaf is removed.
#[test]
fn clearing_the_user_preference_resets_the_backend_choice() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::WgpuBackend,
        CadSettingValue::Text("dx12".to_string()),
    )
    .unwrap();
    core.clear_user_preference().expect("the clear persists");
    assert_eq!(core.settings().wgpu_backend, "auto");
    let written = persisted(&config).expect("the clear persists a document");
    let document: serde_json::Value = serde_json::from_str(&written).expect("persisted JSON");
    assert_eq!(document.get("wgpu_backend"), None, "{written}");
}

/// A failed persistence is an explicit error and the in-memory choice rolls
/// back: the snapshot never claims a value that was not written.
#[test]
fn a_failed_backend_write_rolls_back_and_reports_the_error() {
    let core = core_with_config(Rc::new(RejectingPreferenceStore));
    let error = core
        .apply_setting(
            CadSettingKey::WgpuBackend,
            CadSettingValue::Text("vulkan".to_string()),
        )
        .expect_err("the store refuses the write");
    assert!(
        matches!(&error, CadError::InvalidInput(reason) if reason == "preference store is read-only"),
        "{error:?}"
    );
    assert_eq!(
        core.settings().wgpu_backend,
        "auto",
        "the rejected write left no trace"
    );
}
