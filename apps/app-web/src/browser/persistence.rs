//! Browser file dialogs and downloads.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use super::messages::current_messages;

pub(super) fn open_dialog(id: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(element) = document.get_element_by_id(id) {
            if let Ok(input) = element.dyn_into::<web_sys::HtmlInputElement>() {
                input.click();
            }
        }
    }
}

/// Trigger a browser download of `bytes` under `name`.
///
/// The bytes are wrapped in an `application/octet-stream` `Blob`, exposed through
/// an object URL, and clicked via an `<a download="name">` element. The
/// `download` attribute is always set, so the browser saves the file instead of
/// navigating to it (the reason an octet-stream type is used even for SVG). The
/// anchor is appended to the document before the click (Safari requires it) and
/// removed after; the object URL is revoked on a `setTimeout(0)` so the download
/// can start first. Every DOM failure is a real error, never a silent success.
pub(super) fn download(name: &str, bytes: &[u8]) -> Result<(), cad_domain::CadError> {
    let window = web_sys::window().ok_or_else(no_window)?;
    let document = window.document().ok_or_else(no_window)?;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).into());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|error| blob_error(&error))?;
    let url =
        web_sys::Url::create_object_url_with_blob(&blob).map_err(|error| blob_error(&error))?;

    let anchor = document
        .create_element("a")
        .map_err(|error| blob_error(&error))?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| {
            cad_domain::CadError::Unsupported(current_messages().text("web.download_anchor", &[]))
        })?;
    anchor.set_href(&url);
    anchor.set_download(name);
    // Safari only honours a programmatic click on an anchor that is in the DOM.
    // `Element`/`Node` methods are reached through explicit upcasts because
    // web-sys types do not deref to their parent interface. A document with no
    // body cannot host the anchor, so a detached click (which some browsers
    // ignore) is refused rather than reported as a successful download.
    let body = document.body().ok_or_else(no_body)?;
    let anchor_node: web_sys::Node = anchor.clone().unchecked_into();
    let body_node: web_sys::Node = body.unchecked_into();
    body_node
        .append_child(anchor_node.as_ref())
        .map_err(|error| blob_error(&error))?;
    anchor.click();
    if anchor_node.parent_node().is_some() {
        let anchor_element: web_sys::Element = anchor.clone().unchecked_into();
        anchor_element.remove();
    }

    // Revoke after the current task so the download has begun. `once_into_js`
    // hands the closure to JS, which drops the captured Rust closure after the
    // single call — a self-destroying one-shot that does not leak (unlike
    // `Closure::once(...).forget()`).
    let revoke_url = url.clone();
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&revoke_url);
    });
    if window
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 0)
        .is_err()
    {
        // A host without timers: revoke now rather than leak the object URL.
        let _ = web_sys::Url::revoke_object_url(&url);
    }
    Ok(())
}

fn no_window() -> cad_domain::CadError {
    cad_domain::CadError::Unsupported(current_messages().text("web.download_no_window", &[]))
}

fn no_body() -> cad_domain::CadError {
    cad_domain::CadError::Unsupported(current_messages().text("web.download_no_body", &[]))
}

fn blob_error(error: &JsValue) -> cad_domain::CadError {
    cad_domain::CadError::Unsupported(
        current_messages().text("web.download_failed", &[("error", &format!("{error:?}"))]),
    )
}
