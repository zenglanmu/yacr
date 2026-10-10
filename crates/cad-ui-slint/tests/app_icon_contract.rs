//! Contract: the desktop shell advertises the shared application icon.
//!
//! The window icon (title bar / task bar / dock on supporting window managers)
//! must come from the single source of truth in the repository-root `assets/`
//! directory, not a crate-local copy, so every app surface agrees on one icon.
use std::path::PathBuf;

/// Repository root, two levels above this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn shell_declares_the_shared_app_icon() {
    let definition = cad_ui_slint::UI_DEFINITION;
    assert!(
        definition.contains(r#"icon: @image-url("../../../assets/yacr-icon.svg")"#),
        "YacrWindow must bind `icon` to the shared assets/yacr-icon.svg"
    );
}

#[test]
fn shared_app_icon_exists() {
    let icon = repo_root().join("assets/yacr-icon.svg");
    assert!(
        icon.is_file(),
        "expected the shared app icon at {}",
        icon.display()
    );
    let svg = std::fs::read_to_string(&icon).expect("read assets/yacr-icon.svg");
    assert!(svg.starts_with("<svg"), "shared app icon must be an SVG");
}
