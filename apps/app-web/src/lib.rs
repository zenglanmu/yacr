//! Browser host — RETIRED: an explicit NOT IMPLEMENTED placeholder.
//!
//! The wasm UI has been retired. The Slint canvas host, the browser platform
//! adapters, the open/backend-switch/request-queue wiring and the CAD render
//! bridge are all gone, so this crate no longer presents a CAD surface in the
//! browser.
//!
//! What remains is an explicit refusal: the wasm entry points ([`start_web`],
//! [`start_web_with_backend`]) reject with a clear diagnostic instead of a blank
//! canvas or an empty success. The JS shell under `apps/app-web/web/` was
//! removed with the rest of the retired web tooling, and no live build or CI
//! path publishes a web bundle (`scripts/build-web.sh` fails with NOT
//! IMPLEMENTED).

/// The stable retirement diagnostic returned to the browser caller.
pub const RETIRED_DIAGNOSTIC: &str = "Web UI retired — NOT IMPLEMENTED";

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Entry point for the retired web host. The UI is retired, so this rejects.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web() -> Result<(), JsValue> {
    Err(startup_refusal())
}

/// A caller may probe a renderer backend before choosing this entry point; the
/// retired host refuses regardless of the choice.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn start_web_with_backend(_backend: &str) -> Result<(), JsValue> {
    Err(startup_refusal())
}

/// Report the refusal on the browser console as well as through the returned
/// rejection, so a caller that ignores the rejected promise still sees why the
/// host is gone.
#[cfg(target_arch = "wasm32")]
fn startup_refusal() -> JsValue {
    let message = JsValue::from_str(RETIRED_DIAGNOSTIC);
    web_sys::console::error_1(&message);
    message
}

/// Native builds cannot run the browser host; this is a platform constraint, not
/// a placeholder. Use `wasm32-unknown-unknown` (see `docs/build.md`).
#[cfg(not(target_arch = "wasm32"))]
pub fn start() -> Result<(), String> {
    Err(RETIRED_DIAGNOSTIC.to_string())
}
