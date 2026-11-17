//! The Save-As export-version settings (`acisVersion`, `dxfVersion`): defaults,
//! the closed key table, validation, persistence and the real save path.
//!
//! Writer + our own reader agreement is **internal consistency**, not a
//! third-party compatibility claim: no vendor reader has opened the ASM 21800
//! payload or the AC1027/AC1032 `ACDSDATA` container these tests produce.
use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// A core whose platform records written bytes **and** whose config store records
/// every persisted projection, so one test can assert both the setting leaf and
/// the bytes the save funnel produced from it.
fn export_core() -> (HostCore, Rc<SyncSaveFiles>, Rc<SettingsConfig>) {
    let files = Rc::new(SyncSaveFiles::default());
    let config = Rc::new(SettingsConfig::default());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: config.clone(),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap();
    (core, files, config)
}

/// Open the committed exact-body fixture so the save path really writes a
/// `3DSOLID`.
fn open_acis_cube(core: &HostCore) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/dxf/synthetic-acis-cube.dxf");
    let bytes = std::fs::read(&fixture).expect("committed ACIS DXF fixture");
    core.controller
        .borrow_mut()
        .open_bytes(Arc::from(bytes), "synthetic-acis-cube.dxf")
        .expect("the ACIS fixture opens");
}

/// The SAB/ASM header version of the `310` chunks written into a DXF's
/// `ACDSDATA` section.
fn acds_sab_version(bytes: &[u8]) -> u32 {
    let text = String::from_utf8_lossy(bytes);
    assert!(text.contains("ACDSDATA"), "the file carries ACDSDATA");
    let mut blob = Vec::new();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut index = 0;
    while index + 1 < lines.len() {
        if lines[index] == "310" {
            let hex = lines[index + 1];
            for pair in hex.as_bytes().as_chunks::<2>().0 {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    other => panic!("not a hex digit: {}", other as char),
                };
                blob.push((digit(pair[0]) << 4) | digit(pair[1]));
            }
        }
        index += 1;
    }
    assert!(
        blob.starts_with(b"ACIS BinaryFile"),
        "the ACDSDATA payload is a SAB/ASM blob"
    );
    u32::from_le_bytes(blob[15..19].try_into().expect("the version field fits"))
}

#[test]
fn the_export_version_keys_are_in_the_closed_table_with_their_defaults() {
    assert_eq!(
        CadSettingKey::ALL.len(),
        26,
        "one writable key per contract"
    );
    for (key, wire) in [
        (CadSettingKey::AcisVersion, "acisVersion"),
        (CadSettingKey::DxfVersion, "dxfVersion"),
    ] {
        assert_eq!(
            CadSettingKey::ALL
                .into_iter()
                .filter(|candidate| *candidate == key)
                .count(),
            1,
            "{key:?} is declared exactly once"
        );
        assert_eq!(key.key(), wire);
        assert_eq!(CadSettingKey::parse(wire), Some(key));
        assert_eq!(
            CadSettingKey::parse(&format!("ui.export.{wire}")),
            None,
            "the wire key is the leaf, not the dotted path"
        );
    }

    let core = core();
    let settings = core.settings();
    assert_eq!(settings.acis_version, "7.0", "the documented default");
    assert_eq!(settings.dxf_version, "2010", "the documented default");
    let effective = core.config_snapshot().effective;
    assert_eq!(
        settings.acis_version,
        effective.ui.export.acis_version.label()
    );
    assert_eq!(
        settings.dxf_version,
        effective.ui.export.dxf_version.label()
    );
    assert_eq!(
        core.export_options(),
        cad_app::host::ExportOptions::default(),
        "the default export request is the pre-selector combination"
    );
}

#[test]
fn an_unknown_or_wrong_shaped_export_version_is_refused_not_defaulted() {
    let (core, config) = settings_core();
    for value in [
        CadSettingValue::Text("8.0".to_string()),
        CadSettingValue::Text("21800".to_string()),
        CadSettingValue::Number(21),
        CadSettingValue::Bool(true),
    ] {
        let error = core
            .apply_setting(CadSettingKey::AcisVersion, value)
            .expect_err("an unknown acisVersion token is refused");
        assert_eq!(
            error,
            CadError::InvalidInput(SETTINGS_INVALID_VALUE.to_string())
        );
    }
    for value in [
        CadSettingValue::Text("2020".to_string()),
        CadSettingValue::Text("2013 ".to_string()),
        CadSettingValue::Number(2013),
        CadSettingValue::Bool(false),
    ] {
        let error = core
            .apply_setting(CadSettingKey::DxfVersion, value)
            .expect_err("an unknown dxfVersion token is refused");
        assert_eq!(
            error,
            CadError::InvalidInput(SETTINGS_INVALID_VALUE.to_string())
        );
    }
    // A refused value changes neither the snapshot nor the durable document.
    assert_eq!(core.settings().acis_version, "7.0");
    assert_eq!(core.settings().dxf_version, "2010");
    assert!(
        persisted(&config).is_none(),
        "a refused value is never written: {:?}",
        persisted(&config)
    );
}

#[test]
fn both_keys_persist_and_reload_through_the_real_parser() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::AcisVersion,
        CadSettingValue::Text("21.0".to_string()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::DxfVersion,
        CadSettingValue::Text("2013".to_string()),
    )
    .unwrap();

    assert_eq!(core.settings().acis_version, "21.0");
    assert_eq!(core.settings().dxf_version, "2013");
    let persisted = persisted(&config).expect("a durable projection");
    assert!(
        persisted.contains(r#""acisVersion":"21.0""#),
        "the ACIS payload version is persisted: {persisted}"
    );
    assert!(
        persisted.contains(r#""dxfVersion":"2013""#),
        "the DXF container version is persisted: {persisted}"
    );

    // The typed options the save funnel installs follow the leaves.
    let options = core.export_options();
    assert_eq!(options.acis_version, cad_app::host::AcisVersion::V21_0);
    assert_eq!(options.dxf_target, cad_app::host::DxfTarget::R2013);

    // A fresh core reading the same preference sees the same values (the store's
    // real parser accepted the projected document).
    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&persisted)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings().acis_version, "21.0");
    assert_eq!(reloaded.settings().dxf_version, "2013");
}

#[test]
fn the_save_path_writes_the_configured_dxf_container_and_payload() {
    let (core, files, _config) = export_core();
    open_acis_cube(&core);

    // R2013 + ASM 21800: the payload moves into ACDSDATA as an ASM blob.
    core.apply_setting(
        CadSettingKey::DxfVersion,
        CadSettingValue::Text("2013".to_string()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::AcisVersion,
        CadSettingValue::Text("21.0".to_string()),
    )
    .unwrap();
    core.request_save().unwrap();
    {
        let written = files.written.borrow();
        assert_eq!(written.len(), 1, "exactly one file is written");
        let text = String::from_utf8_lossy(&written[0].1);
        assert!(
            text.contains("AC1027"),
            "the configured R2013 container is written, not the R2010 default"
        );
        assert!(
            !text.contains("AC1024"),
            "the R2010 default must not leak into a R2013 save"
        );
        assert_eq!(
            acds_sab_version(&written[0].1),
            21800,
            "the ACDSDATA payload is the configured ASM 21.0"
        );
    }
    let status = core.last_status().expect("the save grades a status");
    let (key, _) = catalog(&status);
    assert!(
        key.starts_with("save.exported_acis_sab_dxf"),
        "the status names the ACDSDATA payload: {key}"
    );

    // R2018 + ACIS 7.0: the same container family, the historical payload.
    core.apply_setting(
        CadSettingKey::DxfVersion,
        CadSettingValue::Text("2018".to_string()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::AcisVersion,
        CadSettingValue::Text("7.0".to_string()),
    )
    .unwrap();
    core.request_save().unwrap();
    {
        let written = files.written.borrow();
        assert_eq!(written.len(), 2, "a second save writes a second file");
        let text = String::from_utf8_lossy(&written[1].1);
        assert!(text.contains("AC1032"), "the R2018 container is written");
        assert_eq!(acds_sab_version(&written[1].1), 700);
    }

    // Back to the documented default: R2010 inline SAT text, no ACDSDATA.
    core.apply_setting(
        CadSettingKey::DxfVersion,
        CadSettingValue::Text("2010".to_string()),
    )
    .unwrap();
    core.request_save().unwrap();
    {
        let written = files.written.borrow();
        assert_eq!(written.len(), 3, "a third save writes a third file");
        let text = String::from_utf8_lossy(&written[2].1);
        assert!(text.contains("AC1024"));
        assert!(!text.contains("ACDSDATA"));
    }
    let status = core.last_status().expect("the save grades a status");
    let (key, _) = catalog(&status);
    assert!(
        key.starts_with("save.exported_acis_sat"),
        "the status names the inline SAT text choice: {key}"
    );
}

#[test]
fn a_save_with_an_unrepresentable_combination_fails_and_writes_nothing() {
    let (core, files, _config) = export_core();
    open_acis_cube(&core);
    // ASM 21800 cannot be inline SAT text, and R2010 is inline-only: the save is
    // refused by name instead of degrading to POLYFACE or another version.
    core.apply_setting(
        CadSettingKey::AcisVersion,
        CadSettingValue::Text("21.0".to_string()),
    )
    .unwrap();
    core.request_save().unwrap();

    assert!(
        files.written.borrow().is_empty(),
        "a refused combination writes no file"
    );
    let status = core.last_status().expect("the save grades a status");
    let (key, args) = catalog(&status);
    assert_eq!(key, "save.export_failed");
    let reason = args.get("reason").expect("the failure names its reason");
    assert!(
        reason.contains("export.acis_asm_requires_r2013"),
        "the status carries the exporter's stable code: {reason}"
    );
}

#[test]
fn a_dwg_export_keeps_ac1032_whatever_dxf_version_is_configured() {
    let (core, _files, _config) = export_core();
    open_acis_cube(&core);
    core.apply_setting(
        CadSettingKey::AcisVersion,
        CadSettingValue::Text("21.0".to_string()),
    )
    .unwrap();
    // A DWG's container is fixed; the configured DXF version is a DXF-only
    // choice, so the controller names the DWG's own container instead of asking
    // the writer to ignore the setting.
    core.controller
        .borrow_mut()
        .set_export_options(core.export_options());
    let (bytes, report) = core.controller.borrow().export_dwg().unwrap();
    assert!(bytes.starts_with(b"AC10"), "the export is a DWG document");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sab_dwg" && note.message.contains("21.0")),
        "the DWG note names the ASM payload: {:?}",
        report.notes
    );
    // The configured `dxfVersion` is still the documented 2010 default here, yet
    // the DWG request was accepted: the controller names the DWG's own AC1032
    // container, and the exporter's matrix would have refused anything else.
    assert_eq!(core.settings().dxf_version, "2010");
    assert!(
        core.controller.borrow().export_options().acis_version == cad_app::host::AcisVersion::V21_0
    );
    assert!(
        report.counts.bytes == bytes.len(),
        "the report counts the bytes the platform would write"
    );
}
