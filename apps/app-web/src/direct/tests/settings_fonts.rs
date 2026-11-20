//! The browser carrier's font-directory boundary.
//!
//! The `dom-canvas` session has no local filesystem: a font package arrives as
//! bytes from the page's same-origin `fonts/`. So the two font-directory fields
//! are **omitted** from `cad_settings` (a disabled row, not a fabricated path)
//! and the setting write is refused by name instead of silently succeeding.

use cad_ui_core::session::wire::AdmissionTarget;
use cad_ui_core::session::{CadSettingKey, CadSettingValue};

use super::*;

#[test]
fn the_web_settings_snapshot_omits_the_font_directory_fields() {
    let session = session();
    let direct = DirectReads::new(&session);
    let settings = wire::settings(&direct).expect("a live session answers");
    assert!(settings.fonts_dir.is_none());
    assert!(settings.fonts_dir_override.is_none());
    assert!(
        !settings.fonts_dir_missing,
        "a carrier with no saved directory has nothing missing"
    );

    let value = serde_json::to_value(&settings).expect("settings serialize");
    assert!(
        value.get("fontsDir").is_none(),
        "the key is omitted, never null or fabricated: {value}"
    );
    assert!(
        value.get("fontsDirOverride").is_none(),
        "the key is omitted, never null or fabricated: {value}"
    );
}

#[test]
fn the_web_carrier_refuses_the_font_directory_write_by_name() {
    let session = session();
    let mut sink = DirectSink::new(&session);

    let error = sink
        .setting(
            CadSettingKey::FontsDir,
            CadSettingValue::Text("/usr/share/fonts".to_string()),
        )
        .expect_err("the browser carrier has no local font directory");
    assert_eq!(error, crate::platform::UNSUPPORTED_FONTS_DIR);
    assert!(
        error.starts_with("unsupported-runtime: "),
        "the shared carrier marker leads: {error}"
    );
    // Nothing was applied: the session's own snapshot still has no choice.
    assert_eq!(session.settings().fonts_dir_override, None);

    // The same refusal comes back through the wire command, so the React adapter
    // sees one string whichever entry point it uses.
    let direct = DirectReads::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;
    let error = dispatch(
        "cad_set_setting",
        r#"{"key":"fontsDir","value":"/usr/share/fonts"}"#,
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect_err("the wire write is refused too");
    assert_eq!(error, crate::platform::UNSUPPORTED_FONTS_DIR);
    assert_eq!(session.settings().fonts_dir_override, None);
}
