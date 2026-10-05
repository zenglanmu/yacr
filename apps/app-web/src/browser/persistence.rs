//! Browser file dialogs.

use wasm_bindgen::JsCast;

pub(super) fn open_dialog(id: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(element) = document.get_element_by_id(id) {
            if let Ok(input) = element.dyn_into::<web_sys::HtmlInputElement>() {
                input.click();
            }
        }
    }
}
