//! Browser host support: backend probing/selection and logging (wasm32 only).
//!
//! Spec v2.0 §6: `Auto` must not merely check `navigator.gpu`; it performs a
//! real WebGPU adapter request and falls back to WebGL2 when that fails. The
//! chosen backend is reported to the UI, and forcing a backend records a
//! reason when it cannot be satisfied.

use cad_domain::{CadError, CadResult};
use cad_render_wgpu::wgpu;
use wasm_bindgen::JsCast;

pub use cad_render_wgpu::{ActiveBackend, BackendPreference};

/// Storage key for the user's CAD backend preference.
pub const BACKEND_STORAGE_KEY: &str = "yacr.cad.backend";

fn parse_preference(value: &str) -> Option<BackendPreference> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Some(BackendPreference::Auto),
        "webgpu" => Some(BackendPreference::WebGpu),
        "webgl2" => Some(BackendPreference::WebGl2),
        _ => None,
    }
}

fn preference_name(preference: BackendPreference) -> &'static str {
    match preference {
        BackendPreference::Auto => "auto",
        BackendPreference::WebGpu => "webgpu",
        BackendPreference::WebGl2 => "webgl2",
    }
}

/// Backend preference from `?backend=` or localStorage, defaulting to Auto.
pub fn stored_preference() -> BackendPreference {
    if let Some(search) = web_sys::window().and_then(|w| w.location().search().ok()) {
        for pair in search.trim_start_matches('?').split('&') {
            let mut parts = pair.splitn(2, '=');
            if let (Some("backend"), Some(value)) = (parts.next(), parts.next()) {
                if let Some(preference) = parse_preference(&percent_decoded(value)) {
                    return preference;
                }
            }
        }
    }
    if let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) {
        if let Ok(Some(value)) = storage.get_item(BACKEND_STORAGE_KEY) {
            if let Some(preference) = parse_preference(&value) {
                return preference;
            }
        }
    }
    BackendPreference::Auto
}

/// Persist a preference and reload so the render session is rebuilt (spec §6
/// allows a restart instead of in-place hot switching).
pub fn store_preference_and_reload(preference: BackendPreference) -> CadResult<()> {
    let window = web_sys::window().ok_or_else(|| CadError::Invariant("no window".into()))?;
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(BACKEND_STORAGE_KEY, preference_name(preference));
    }
    let location = window.location();
    location
        .reload()
        .map_err(|_| CadError::Invariant("reload failed".into()))?;
    Ok(())
}

fn percent_decoded(value: &str) -> String {
    // Minimal %XX decoder for query values; malformed input falls through.
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Ask WGPU for a WebGPU adapter (a real probe, not just API presence).
///
/// `navigator.gpu` existing is not evidence: the browser may expose the object
/// and still refuse an adapter. This performs the actual request.
pub async fn webgpu_available() -> bool {
    if !webgpu_api_present() {
        return false;
    }
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(descriptor);
    instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .is_ok()
}

/// Whether `navigator.gpu` exists, without claiming an adapter is available.
pub fn webgpu_api_present() -> bool {
    web_sys::window()
        .map(|window| js_sys::Reflect::has(&window.navigator(), &"gpu".into()).unwrap_or(false))
        .unwrap_or(false)
}

/// Whether the page can create a WebGL2 context (the base renderer tier).
pub fn webgl2_available() -> bool {
    if let Some(canvas) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("canvas"))
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    {
        if let Ok(context) = canvas.get_context("webgl2") {
            if context.is_some() {
                return true;
            }
        }
    }
    // Fall back to a probe canvas so hosts that install the canvas later still
    // get an answer.
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.create_element("canvas").ok())
        .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .and_then(|canvas| canvas.get_context("webgl2").ok().flatten())
        .is_some()
}

/// Resolve a preference and select Slint's wgpu renderer with those backends.
///
/// Returns the backend actually requested; the renderer reports the concrete
/// adapter through its capability snapshot once the first frame is set up.
pub async fn select_backend(preference: BackendPreference) -> CadResult<BackendPreference> {
    let chosen = match preference {
        BackendPreference::Auto => {
            if webgpu_available().await {
                BackendPreference::WebGpu
            } else if webgl2_available() {
                BackendPreference::WebGl2
            } else {
                return Err(CadError::GpuFailure(
                    "neither WebGPU nor WebGL2 is available in this browser".into(),
                ));
            }
        }
        forced => forced,
    };
    if chosen == BackendPreference::WebGpu && !webgpu_api_present() {
        return Err(CadError::GpuFailure(
            "WebGPU was requested but navigator.gpu is missing".into(),
        ));
    }
    if chosen == BackendPreference::WebGl2 && !webgl2_available() {
        return Err(CadError::GpuFailure(
            "WebGL2 was requested but the browser rejected a webgl2 context".into(),
        ));
    }
    let backends = match chosen {
        BackendPreference::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
        BackendPreference::WebGl2 => wgpu::Backends::GL,
        BackendPreference::Auto => wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
    };
    let mut settings = slint::wgpu_30::WGPUSettings::default();
    settings.backends = backends;
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(settings))
        .select()
        .map_err(|e| CadError::GpuFailure(format!("slint wgpu backend ({chosen:?}): {e}")))?;
    Ok(chosen)
}

/// Install a console panic hook so panics are visible in the browser console.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!("yacr panic: {info}");
        web_sys::console::error_1(&message.into());
    }));
}

/// Log to the browser console.
pub fn console_log(message: &str) {
    web_sys::console::log_1(&message.into());
}

/// Log an error to the browser console.
pub fn console_error(message: &str) {
    web_sys::console::error_1(&message.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_round_trip() {
        for preference in [
            BackendPreference::Auto,
            BackendPreference::WebGpu,
            BackendPreference::WebGl2,
        ] {
            assert_eq!(
                parse_preference(preference_name(preference)),
                Some(preference)
            );
        }
        assert_eq!(parse_preference("nonsense"), None);
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(percent_decoded("webgpu"), "webgpu");
        assert_eq!(percent_decoded("web%67pu"), "webgpu");
        assert_eq!(percent_decoded("a+b"), "a b");
    }
}
