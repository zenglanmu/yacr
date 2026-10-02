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
pub use browser::{
    confirm_annotation_export, export_annotations_json, font_report, import_annotations_json,
    open_document, open_document_decided, open_needs_decision, pending_recovery_is_valid,
    pending_recovery_snapshot, renderer_report, start,
};

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

/// Native browser touch navigation, never mutates a GPU camera directly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn touch_navigate(dx: f64, dy: f64, zoom: f64) -> Result<(), JsValue> {
    browser::shell::navigate(dx, dy, zoom).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn touch_pick(x: f64, y: f64) -> Result<(), JsValue> {
    browser::shell::pick(x, y).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Open a drawing from bytes read by the JS File API host.
///
/// Refuses to replace a document with unsaved annotations; the JS host must
/// call `open_needs_decision`, prompt, then `open_document_bytes_decided`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes(name: String, bytes: Vec<u8>) -> Result<(), JsValue> {
    open_document(&name, bytes).map_err(|e| JsValue::from_str(&e))
}

/// Whether the current document has unsaved annotations that need a decision.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_requires_decision() -> bool {
    open_needs_decision()
}

/// Open a drawing after the JS host supplied an explicit unsaved-work decision.
///
/// `decision` is `save`, `recovery`, `discard` or `cancel`. Returns a status
/// string; `cancel` and a failed write are errors and never replace the
/// document.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn open_document_bytes_decided(
    name: String,
    bytes: Vec<u8>,
    decision: String,
) -> Result<String, JsValue> {
    open_document_decided(&name, bytes, &decision).map_err(|e| JsValue::from_str(&e))
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

/// Annotation JSON export (caller downloads the returned text). The revision
/// must be passed back to `annotation_confirm_export` after a successful write.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_export_json() -> Result<JsValue, JsValue> {
    let (json, revision) = export_annotations_json().map_err(|e| JsValue::from_str(&e))?;
    let out = js_sys::Object::new();
    js_sys::Reflect::set(&out, &"json".into(), &JsValue::from_str(&json))
        .map_err(|_| JsValue::from_str("failed to build export object"))?;
    js_sys::Reflect::set(
        &out,
        &"revision".into(),
        &JsValue::from_f64(revision as f64),
    )
    .map_err(|_| JsValue::from_str("failed to build export object"))?;
    Ok(out.into())
}

/// Confirm that the export at `revision` was durably written.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_confirm_export(revision: f64) -> Result<(), JsValue> {
    confirm_annotation_export(revision as u64).map_err(|e| JsValue::from_str(&e))
}

/// Annotation JSON import from user-chosen text.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn annotation_import_json(text: &str) -> Result<usize, JsValue> {
    import_annotations_json(text).map_err(|e| JsValue::from_str(&e))
}

/// Whether a backend-switch recovery snapshot is waiting (audit B06).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn has_recovery_snapshot() -> bool {
    cad_ui_slint::web::has_recovery_snapshot()
}

/// Restore the pending recovery snapshot (returns the annotation count).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn restore_recovery_snapshot() -> Result<usize, JsValue> {
    browser::restore_pending_recovery_snapshot().map_err(|e| JsValue::from_str(&e))
}

/// Discard the pending recovery snapshot explicitly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn discard_recovery_snapshot() {
    browser::drop_pending_recovery_snapshot()
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
