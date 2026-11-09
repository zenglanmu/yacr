//! Contract for the shared `cad_app_version` answer.
//!
//! The version is a compile-time property of the crate that serves the command, so
//! this test lives next to the crate itself: it pins the registry row and the
//! `CARGO_PKG_VERSION` value the About dialog and the frontend both read.

#[test]
fn the_running_crate_version_is_the_registered_wire_answer() {
    use cad_ui_core::session::{
        SHARED_WIRE_COMMAND_NAMES, TAURI_ONLY_WIRE_COMMAND_NAMES, WIRE_COMMAND_NAMES,
    };

    assert!(
        WIRE_COMMAND_NAMES.contains(&"cad_app_version"),
        "the version command must be registered"
    );
    assert!(
        SHARED_WIRE_COMMAND_NAMES.contains(&"cad_app_version"),
        "both transports serve the version command"
    );
    assert!(
        !TAURI_ONLY_WIRE_COMMAND_NAMES.contains(&"cad_app_version"),
        "the version command is not desktop-only"
    );
    assert_eq!(cad_ui_core::app_version(), env!("CARGO_PKG_VERSION"));
    assert!(!cad_ui_core::app_version().is_empty());
}
