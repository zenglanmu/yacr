//! Authoritative drawing replacement through the host's real open pipeline.
//!
//! The bytes arrive from the JS File API host and are queued as a
//! [`cad_platform::FileEvent`]; this module drains that queue and hands each
//! `Opened` event straight to `HostCore::deliver_external_open`, which reuses the
//! shared open pipeline (sync/worker choice, the independent document epoch,
//! resource publication). The browser never mutates the controller document, the
//! session `IncomingDocument` slot or the draw capture itself — the host owns
//! those, so the epoch is never bypassed (P3 platform gate).
//!
//! The browser has no worker thread on wasm, so `deliver_external_open` installs
//! synchronously; `Ok(())` still only means the document was delivered, never a
//! fabricated "opened/worker started" claim.

use cad_platform::{FileEvent, PlatformFiles};

use super::fonts::spawn_font_load_if_identity_changed;
use super::{with_runtime, HostRuntime};

/// Open a drawing from the File API / File System Access path.
pub fn open_document(name: &str, bytes: Vec<u8>) -> Result<(), String> {
    let rt = with_runtime(|rt| rt.clone()).ok_or_else(|| "浏览器宿主尚未启动".to_string())?;
    // Queue the bytes as a platform file event and drain it through the host
    // open pipeline: the `PlatformFiles` contract is event-based, and the
    // browser read is synchronous so the event is available immediately.
    rt.files.push_opened(name.to_string(), bytes);
    drain_open_events(&rt)
}

/// Drain the web file queue, delivering every opened drawing through the host.
///
/// An `Opened` event is delivered with `HostCore::deliver_external_open` (queued
/// old input is allowed; active work is refused with a real error and no
/// mutation). Any other event (a save target, for example) is put back for the
/// core's own save funnel, which drains the same queue in its gesture, so no save
/// is dropped.
pub(super) fn drain_open_events(rt: &HostRuntime) -> Result<(), String> {
    let mut result = Ok(());
    for event in rt.files.poll() {
        match event {
            FileEvent::Opened { target, bytes } => {
                match rt.core.deliver_external_open(target, bytes) {
                    Ok(()) => {
                        // The host published the document (sync install on wasm,
                        // or an import started on a worker host); start the web
                        // font load only once the published identity changed.
                        spawn_font_load_if_identity_changed(rt);
                        rt.flush_updates().ok();
                    }
                    Err(error) => {
                        let _ = rt
                            .core
                            .set_catalog_status("command.failed", &[("error", &error.to_string())]);
                        rt.flush_updates().ok();
                        result = Err(error.to_string());
                    }
                }
            }
            other => rt.files.requeue(other),
        }
    }
    result
}
