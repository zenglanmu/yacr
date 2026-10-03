//! Browser font fetching, registration and load diagnostics.

use std::cell::RefCell;
use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_platform::fonts::{load_font_engine, requested_fonts, FontLoadReport};
use cad_platform::FontLoader;
use cad_resources::DEFAULT_FONT_BASE_URL;
use cad_ui_slint::UiHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use super::with_runtime;

// Last font-loading report, surfaced for diagnostics and headless tests.
thread_local! {
    static FONT_REPORT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_font_status(handle: &UiHandle, text: String) {
    let _ = handle.set_status(text.clone());
    FONT_REPORT.with(|slot| *slot.borrow_mut() = Some(text));
}

/// Human-readable font report (catalog/plan/registered/failed counts).
pub fn font_report() -> String {
    FONT_REPORT
        .with(|slot| slot.borrow().clone())
        .unwrap_or_else(|| "字体：未加载".to_string())
}

/// Same-origin for packaged builds; plain cargo builds retain the shared CDN.
const WEB_FONT_BASE_URL: &str = match option_env!("YACR_FONT_BASE_URL") {
    Some(base) => base,
    None => DEFAULT_FONT_BASE_URL,
};

/// Fetches font bytes through the browser Fetch API.
struct WebFontLoader;

async fn fetch_bytes(url: &str) -> CadResult<Arc<[u8]>> {
    let window = web_sys::window()
        .ok_or_else(|| CadError::Invariant("font fetch needs a browser window".into()))?;
    let response_value = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| CadError::ResourceMissing(format!("font fetch failed for {url}: {e:?}")))?;
    let response: web_sys::Response = response_value
        .dyn_into()
        .map_err(|_| CadError::Invariant(format!("font fetch {url} did not return a Response")))?;
    if !response.ok() {
        return Err(CadError::ResourceMissing(format!(
            "font fetch {url} returned HTTP {}",
            response.status()
        )));
    }
    let buffer = response
        .array_buffer()
        .map_err(|e| CadError::Invariant(format!("font arrayBuffer failed for {url}: {e:?}")))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|e| CadError::ResourceMissing(format!("font body failed for {url}: {e:?}")))?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    Ok(Arc::from(bytes.into_boxed_slice()))
}

impl FontLoader for WebFontLoader {
    fn load_font(&self, url: &str) -> cad_platform::HostFuture<'_, Arc<[u8]>> {
        let url = url.to_string();
        Box::pin(async move { fetch_bytes(&url).await })
    }
}

/// Load and install the shaping fonts the current drawing references.
/// Stale loads are discarded rather than applied to a replacement drawing.
pub async fn load_current_fonts() -> CadResult<FontLoadReport> {
    let (controller, handle, view, incoming) = with_runtime(|rt| {
        (
            rt.controller.clone(),
            rt.handle.clone(),
            rt.view.clone(),
            rt.incoming.clone(),
        )
    })
    .ok_or_else(|| CadError::Invariant("浏览器宿主尚未启动".into()))?;

    let (requested, identity) = {
        let controller = controller.borrow();
        match controller.drawing() {
            Some(drawing) => (
                requested_fonts(drawing.as_ref()),
                Some(drawing.scene_identity()),
            ),
            None => (Vec::new(), None),
        }
    };

    if requested.is_empty() {
        view.clear_fonts();
        let report = FontLoadReport::default();
        set_font_status(&handle, "字体：图纸未引用 CAD 文本字体".into());
        return Ok(report);
    }

    let (engine, report) = load_font_engine(&WebFontLoader, &requested, WEB_FONT_BASE_URL).await?;
    let current = incoming.borrow().as_ref().map(|d| d.scene_identity());
    if current != identity {
        return Err(CadError::StaleResult);
    }

    if report.registered.is_empty() {
        view.clear_fonts();
    } else {
        view.set_fonts(engine);
    }
    let text = super::messages::current_messages().text(
        "fonts.loaded",
        &[
            ("registered", &report.registered.len().to_string()),
            ("failed", &report.failed.len().to_string()),
            ("unresolved", &report.unresolved.len().to_string()),
        ],
    );
    set_font_status(&handle, text);
    Ok(report)
}

/// Best-effort load after opening: failures never replace the open document.
pub(super) fn spawn_font_load() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = load_current_fonts().await {
            if let Some(handle) = with_runtime(|rt| rt.handle.clone()) {
                let _ = handle.set_status(
                    super::messages::current_messages()
                        .text("fonts.failed", &[("error", &e.to_string())]),
                );
            }
        }
    });
}
