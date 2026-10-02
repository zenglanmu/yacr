//! Browser downloads, file dialogs and durable recovery storage.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::host_files::load_recovery;
use cad_domain::{CadError, DocumentId};
use cad_platform::{HostFuture, Persistence};
use cad_ui_slint::{CadView, UiHandle};
use wasm_bindgen::JsCast;

use super::state_push;

/// Only a successfully initiated download may confirm the exported revision.
pub(super) fn download_text(filename: &str, text: &str) -> bool {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    let parts = js_sys::Array::new();
    parts.push(&wasm_bindgen::JsValue::from_str(text));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&parts) else {
        return false;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return false;
    };
    let mut clicked = false;
    if let Ok(element) = document.create_element("a") {
        if let Ok(anchor) = element.dyn_into::<web_sys::HtmlAnchorElement>() {
            anchor.set_href(&url);
            anchor.set_download(filename);
            anchor.click();
            clicked = true;
        }
    }
    let _ = web_sys::Url::revoke_object_url(&url);
    clicked
}

pub(super) fn open_dialog(id: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(element) = document.get_element_by_id(id) {
            if let Ok(input) = element.dyn_into::<web_sys::HtmlInputElement>() {
                input.click();
            }
        }
    }
}

/// Single recovery slot: document identity lives inside the encoded snapshot.
/// Failed writes propagate so backend switching cannot lose unsaved work.
pub(super) struct WebPersistence;

impl Persistence for WebPersistence {
    fn save_recovery(&self, _document: DocumentId, bytes: Arc<[u8]>) -> HostFuture<'_, ()> {
        Box::pin(async move {
            let text = std::str::from_utf8(&bytes).map_err(|e| {
                CadError::CorruptData(format!("recovery snapshot is not UTF-8: {e}"))
            })?;
            cad_ui_slint::web::store_recovery_snapshot(text)
        })
    }

    fn load_recovery(&self, _document: DocumentId) -> HostFuture<'_, Option<Arc<[u8]>>> {
        Box::pin(async move {
            Ok(cad_ui_slint::web::peek_recovery_snapshot()
                .map(|text| Arc::from(text.into_bytes().into_boxed_slice())))
        })
    }

    fn discard_recovery_after_confirmation(&self, _document: DocumentId) -> HostFuture<'_, ()> {
        Box::pin(async move {
            cad_ui_slint::web::clear_recovery_snapshot();
            Ok(())
        })
    }
}

/// Startup restores only a matching snapshot; mismatches remain recoverable.
///
/// Any applied restore is pushed to the panels through the single funnel so the
/// annotation list and undo availability are correct before the first frame.
pub(super) fn restore_startup_recovery(
    controller: &Rc<RefCell<HostController>>,
    handle: &UiHandle,
    view: &Rc<RefCell<Option<CadView>>>,
) {
    let persistence = WebPersistence;
    let document = controller.borrow().document_id;
    match cad_platform::block_on(load_recovery(&persistence, document)) {
        Ok(Some(snapshot)) => match controller.borrow_mut().restore_recovery_snapshot(&snapshot) {
            Ok(count) => {
                state_push::push_panel_state(controller, handle, view);
                let _ = handle.set_status(format!("已从恢复快照恢复 {count} 条批注"));
            }
            Err(e) => {
                let _ = handle.set_status(format!("检测到恢复快照但未应用（{e}）；可恢复或丢弃"));
            }
        },
        Ok(None) => {}
        Err(e) => {
            let _ = handle.set_status(format!("恢复快照损坏，未应用：{e}"));
        }
    }
}
