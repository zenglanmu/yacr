//! Browser host assembly (spec v2.0 §5.3, §6, §9.2).
//!
//! The web host is a thin shell: the Slint canvas is the single presentation
//! coordinator, the shared `UiAdapter` drives the same commands as Android, and
//! the CAD renderer receives Slint's wgpu device through the same bridge. File
//! opening uses the user-authorized File API; export uses a download. No CAD
//! logic is duplicated here.
//!
//! Compiled for wasm32 the crate exports `start_web`, `open_document_bytes`,
//! `annotation_export_json` and `annotation_import_json` for the minimal JS
//! host in `web/main.js`.

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
pub use browser::{font_report, open_document, renderer_report, start};

/// The stable dot-path used by the web host to persist user preferences.
#[cfg(target_arch = "wasm32")]
pub use browser::config::CONFIG_EVENT_NAME;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Entry point for `web/main.js`; resolves once the Slint/wgpu session is up.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web() -> Result<(), JsValue> {
    start().await.map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The JS host performs a bounded device probe before choosing this backend.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web_with_backend(backend: &str) -> Result<(), JsValue> {
    let preference = match backend {
        "webgpu" => cad_ui_slint::web::BackendPreference::WebGpu,
        "webgl2" => cad_ui_slint::web::BackendPreference::WebGl2,
        _ => return Err(JsValue::from_str("unknown renderer backend")),
    };
    browser::start_with_preference(preference)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn web_resize(width: f64, height: f64, scale: f64) -> Result<(), JsValue> {
    browser::resize(width, height, scale).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// CSS-pixel CAD hit rectangle followed by phone/tools/ribbon/command state.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn shell_geometry() -> Result<Vec<f64>, JsValue> {
    browser::shell::geometry().map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn canvas_hit_test(x: f64, y: f64) -> Result<bool, JsValue> {
    browser::shell::canvas_hit_test(x, y).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Whether the browser host's touch router may act, per the effective config.
///
/// The wasm boundary is the authoritative gate: the JS router has no cheap read
/// of the live effective config, so it always forwards and these exports decide.
/// A host that has not started (`None`) keeps the previous short-circuit/cancel
/// behavior instead of silently succeeding as if touch were enabled.
#[cfg(target_arch = "wasm32")]
fn touch_enabled() -> bool {
    browser::current_handle()
        .map(|handle| handle.effective_config().interaction.touch)
        .unwrap_or(true)
}

/// Native browser touch navigation, never mutates a GPU camera directly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn touch_navigate(dx: f64, dy: f64, zoom: f64) -> Result<(), JsValue> {
    if !touch_enabled() {
        return Ok(());
    }
    browser::shell::navigate(dx, dy, zoom).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn touch_pick(x: f64, y: f64) -> Result<(), JsValue> {
    if !touch_enabled() {
        return Ok(());
    }
    browser::shell::pick(x, y).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// A second finger or cancelled touch abandons unconfirmed drawing capture.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn touch_cancel_draw() -> Result<(), JsValue> {
    if !touch_enabled() {
        return Ok(());
    }
    browser::shell::cancel_draw_capture().map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Open a drawing from bytes read by the JS File API host.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes(name: String, bytes: Vec<u8>) -> Result<(), JsValue> {
    open_document(&name, bytes).map_err(|e| JsValue::from_str(&e))
}

/// Poll the asynchronous / cancellable open and push the progress panel.
///
/// The JS heartbeat calls this. On a worker-capable host it publishes a current
/// background result exactly once (the manager's stamp guard discards
/// superseded/cancelled results) and installs the document through the shared
/// `install_opened` path; in the browser it re-reads the synchronous fallback's
/// real terminal snapshot. Returns the stable panel JSON (see
/// `browser::async_open::snapshot_json`). New export; existing exports are
/// unchanged.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn async_open_poll_json() -> String {
    browser::async_open_poll()
}

/// Whether this host can run the background import worker at all.
///
/// `false` in the browser: `cad_app::tasks` uses `std::thread` and
/// `wasm32-unknown-unknown` has no threads. Exposed so the JS host can state the
/// limitation honestly instead of implying a live progress bar.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn async_open_worker_available() -> bool {
    browser::async_open_worker_available()
}

/// Serializable renderer/state report for the diagnostics panel and tests.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn renderer_state_report() -> String {
    renderer_report()
}

/// Redacted diagnostics model JSON for the diagnostics drawer and headless tests.
///
/// Encodes every import reason through the shared `cad_diagnostics` model; an
/// empty model (no rows) is returned before any import report rather than
/// fabricated entries.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn diagnostics_report_json() -> String {
    browser::diagnostics_report()
}

/// Load the fonts the current drawing references; resolves to a summary.
///
/// Exposed so the JS host (and headless verification) can await the real
/// catalog fetch, font fetches and registration instead of guessing.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn load_web_fonts() -> Result<String, JsValue> {
    browser::load_current_fonts()
        .await
        .map(|report| report.summary())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The last font-loading report (catalog/plan/registered/failed).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn font_load_report() -> String {
    font_report()
}

/// Switch the UI language at runtime and persist the choice (N01 host sync).
///
/// Returns the resolved stable tag (`zh-CN` or `en`). The switch only re-applies
/// catalog-driven chrome; it never rebuilds the document, camera, annotations or
/// undo history. The choice is stored under the shared
/// `cad_ui_slint::web::LOCALE_STORAGE_KEY` so `start()` restores it next load.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn web_set_locale(tag: &str) -> Result<String, JsValue> {
    // The browser host applies the handle chrome switch and re-pushes the
    // catalog-derived panel labels together; the source of truth is one call.
    let resolution = browser::apply_locale(tag).map_err(|e| JsValue::from_str(&e.to_string()))?;
    cad_ui_slint::web::store_locale(resolution.locale.tag())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(resolution.locale.tag().to_string())
}

/// Replace the whole host ViewerConfig from JSON. Full validation happens in
/// `cad-app`; the result is `{ ok: true, config, revision }` or
/// `{ ok: false, path, reason }` (never a false success).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_config_json(config_json: &str) -> JsValue {
    config_result(browser::config::set_config_json(config_json))
}

/// Structured merge into the host ViewerConfig from JSON (arrays replace,
/// explicit `false` survives). Same result shape as `viewer_set_config_json`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_update_config_json(patch_json: &str) -> JsValue {
    config_result(browser::config::update_config_json(patch_json))
}

/// Merge a host-allowed user preference from JSON and persist the allowed
/// projection to localStorage.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_apply_user_preference_json(patch_json: &str) -> JsValue {
    config_result(browser::config::apply_user_preference_json(patch_json))
}

/// Clear the in-memory and persisted user preference.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_clear_user_preference() -> JsValue {
    config_result(browser::config::clear_user_preference())
}

/// The current effective config JSON (for host queries/automation), or `null`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_config_json() -> String {
    browser::config::effective_config_json()
}

/// The effective `interaction` object as compact JSON, or `"null"` before start.
///
/// Read-only companion to `viewer_config_json` for the JS touch router, which
/// uses it to stop capturing/forwarding touch while `interaction.touch` is off.
/// It never mutates config and mirrors `viewer_config_json`'s `"null"` when no
/// host has started. Because absent/malformed input on the JS side must fall
/// back to the same enabled default as `touch_enabled()`'s `unwrap_or(true)`,
/// this export is an optimization only: the `touch_*` exports stay the
/// authoritative gate.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_interaction_json() -> String {
    browser::config::interaction_json()
}

/// Build the stable `{ ok, ... }` result object for a config wasm export.
#[cfg(target_arch = "wasm32")]
fn config_result(result: cad_domain::CadResult<()>) -> JsValue {
    let out = js_sys::Object::new();
    match result {
        Ok(()) => {
            let _ = js_sys::Reflect::set(&out, &"ok".into(), &JsValue::TRUE);
            if let Some(handle) = browser::current_handle() {
                if let Ok(config) = js_sys::JSON::parse(&handle.effective_config_json()) {
                    let _ = js_sys::Reflect::set(&out, &"config".into(), &config);
                }
                let _ = js_sys::Reflect::set(
                    &out,
                    &"revision".into(),
                    &JsValue::from_f64(handle.config_revision() as f64),
                );
            }
        }
        Err(cad_domain::CadError::InvalidInput(detail)) => {
            let _ = js_sys::Reflect::set(&out, &"ok".into(), &JsValue::FALSE);
            let (path, reason) = split_config_error(&detail);
            let _ = js_sys::Reflect::set(&out, &"path".into(), &JsValue::from_str(&path));
            let _ = js_sys::Reflect::set(&out, &"reason".into(), &JsValue::from_str(&reason));
        }
        Err(other) => {
            let _ = js_sys::Reflect::set(&out, &"ok".into(), &JsValue::FALSE);
            let _ = js_sys::Reflect::set(&out, &"path".into(), &JsValue::from_str("config"));
            let _ = js_sys::Reflect::set(
                &out,
                &"reason".into(),
                &JsValue::from_str(&other.to_string()),
            );
        }
    }
    out.into()
}

/// Split the `path\u{1}reason` encoding produced by `browser::config`.
#[cfg(target_arch = "wasm32")]
fn split_config_error(detail: &str) -> (String, String) {
    match detail.split_once('\u{1}') {
        Some((path, reason)) => (path.to_string(), reason.to_string()),
        None => ("config".to_string(), detail.to_string()),
    }
}

/// Native builds cannot run the browser host; this is a platform constraint,
/// not a placeholder. Use `wasm32-unknown-unknown` (see `docs/build.md`).
#[cfg(not(target_arch = "wasm32"))]
pub fn start() -> cad_domain::CadResult<()> {
    Err(cad_domain::CadError::Unsupported(
        "app-web is a wasm32 host; build with --target wasm32-unknown-unknown".into(),
    ))
}

/// Native builds of the browser host have no browser File API.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_document(_name: &str, _bytes: Vec<u8>) -> Result<(), String> {
    Err("app-web is a wasm32 host".into())
}
