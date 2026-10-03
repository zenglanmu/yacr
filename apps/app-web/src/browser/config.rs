//! Browser ViewerConfig host wiring (data-only; no script in the config).
//!
//! The wasm exports hand host JSON to the shared `ViewerConfigStore` through the
//! `UiHandle`, so the Slint shell and the store cannot disagree. User preferences
//! are persisted to `localStorage`, but the Rust store projects them onto
//! `ui.userCustomization.allowedPaths` first; a path the host has since revoked
//! cannot be resurrected on a later load. JSON stays a string across this
//! boundary (`app-web` has no direct serde dependency).

use cad_domain::{CadError, CadResult};
use cad_ui_slint::UiHandle;

use super::current_handle;

/// Browser CustomEvent name emitted after a successful config change.
pub const CONFIG_EVENT_NAME: &str = cad_ui_slint::web::CONFIG_EVENT_NAME;

/// Carry the exact `path`/`reason` to the JS result object as `path\u{1}reason`.
fn config_error(path: &str, reason: &str) -> CadError {
    CadError::InvalidInput(format!("{path}\u{1}{reason}"))
}

fn handle() -> CadResult<UiHandle> {
    current_handle().ok_or(CadError::Cancelled)
}

/// Persist only the currently-allowed user preference leaves.
///
/// The projection comes from the store, so a stale path is dropped rather than
/// written back. A `localStorage` failure is returned, never hidden.
pub fn persist_user_preference(handle: &UiHandle) -> CadResult<()> {
    let text = handle.projected_user_preference_json();
    cad_ui_slint::web::store_config_preference(&text)
}

/// Read the persisted user preference patch (a raw JSON string, unvalidated).
pub fn stored_user_preference() -> Option<String> {
    cad_ui_slint::web::stored_config_preference()
}

/// Install the host config (JSON) and emit a change event on success.
pub fn set_config_json(text: &str) -> CadResult<()> {
    let handle = handle()?;
    handle.set_config_json(text).map_err(config_from_error)?;
    cad_ui_slint::web::emit_config_changed(
        handle.config_revision(),
        &handle.effective_config_json(),
    );
    Ok(())
}

/// Structured merge into the host config; emits a change event on success.
pub fn update_config_json(text: &str) -> CadResult<()> {
    let handle = handle()?;
    handle.update_config_json(text).map_err(config_from_error)?;
    cad_ui_slint::web::emit_config_changed(
        handle.config_revision(),
        &handle.effective_config_json(),
    );
    Ok(())
}

/// Merge a user preference (validated against `allowedPaths`), persist the
/// allowed projection, then emit a change event.
pub fn apply_user_preference_json(text: &str) -> CadResult<()> {
    let handle = handle()?;
    handle
        .apply_user_preference_json(text)
        .map_err(config_from_error)?;
    let _ = persist_user_preference(&handle);
    cad_ui_slint::web::emit_config_changed(
        handle.config_revision(),
        &handle.effective_config_json(),
    );
    Ok(())
}

/// Clear the stored user preference (in-memory and durable) and emit an event.
pub fn clear_user_preference() -> CadResult<()> {
    let handle = handle()?;
    handle.clear_user_preference().map_err(config_from_error)?;
    cad_ui_slint::web::clear_config_preference();
    cad_ui_slint::web::emit_config_changed(
        handle.config_revision(),
        &handle.effective_config_json(),
    );
    Ok(())
}

/// The current effective config JSON, or `"null"` before the host starts.
pub fn effective_config_json() -> String {
    handle()
        .map(|h| h.effective_config_json())
        .unwrap_or_else(|_| "null".to_string())
}

/// The effective `interaction` object as compact JSON, or `"null"` before start.
///
/// The keys are spelled from the typed `InteractionConfig` fields rather than by
/// re-slicing `effective_config_json`, so they cannot drift from the `camelCase`
/// serde form. The JS touch router reads the `touch` leaf to skip capturing or
/// forwarding while the authoritative `touch_*` exports stay the backstop.
pub fn interaction_json() -> String {
    handle()
        .map(|h| {
            let interaction = h.effective_config().interaction;
            format!(
                "{{\"pointer\":{},\"touch\":{},\"keyboardShortcuts\":{}}}",
                interaction.pointer, interaction.touch, interaction.keyboard_shortcuts
            )
        })
        .unwrap_or_else(|_| "null".to_string())
}

fn config_from_error(error: cad_app::viewer_config::ConfigError) -> CadError {
    config_error(&error.path, &error.reason)
}
