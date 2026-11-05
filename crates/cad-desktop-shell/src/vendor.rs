//! Vendor transport for every **native** Tauri carrier.
//!
//! The carrier-neutral state machine (tokens, ABI v1 frames, `applied` rules and
//! the `vendor.*` refusals) lives in
//! [`cad_ui_core::session::vendor`]. This module only adds the two things a
//! Tauri host owns:
//!
//! * the three `#[tauri::command]` wrappers over [`Shared::vendor`];
//! * the owner-tick staging helper and the composed registration list.
//!
//! # Which carriers use it
//!
//! * `app-linux` (`native-below`) routes the names here; its `dom-canvas` branch
//!   refuses them explicitly because the wasm session owns the document there.
//! * `app-windows`, `app-macos` and `app-android` are always `native-below`, so
//!   they register [`register_commands_with_vendor`] and stage from their tick.
//! * `dom-canvas` (X11/XWayland, Web) does **not** use this module; it serves the
//!   same three names from the wasm session (`apps/app-web`) and the shared
//!   state machine, so the protocol cannot drift.
//!
//! A host that never registers these commands must never enable the store: an
//! enabled store emits `vendor-pending`, and the frontend would then call a
//! command that does not exist.

use std::sync::{Arc, Mutex};

use cad_app::vendor::VendorRepresentationStore;
use cad_app::VendorRecord;
use cad_ui_core::session::CadSession;
use tauri::ipc::Invoke;

use crate::state::{lock, ProbeShared, Shared};

pub use cad_ui_core::session::vendor::*;

/// Fetch the binary vendor record frame for one pass.
pub const CAD_VENDOR_RECORDS_COMMAND: &str = "cad_vendor_records";

/// Settle one pass with the plugin's binary result frame (base64 in JSON).
pub const CAD_VENDOR_APPLY_COMMAND: &str = "cad_vendor_apply";

/// Recover the active pass after a late frontend connection.
pub const CAD_VENDOR_STATUS_COMMAND: &str = "cad_vendor_status";

/// One settled derived-representation snapshot awaiting the owner tick.
#[derive(Debug, Clone)]
pub struct PendingVendorRepresentation {
    pub token: u64,
    pub generation: u64,
    pub store: Arc<VendorRepresentationStore>,
}

/// Whether `name` is one of this module's three commands.
pub fn is_vendor_command(name: &str) -> bool {
    matches!(
        name,
        CAD_VENDOR_RECORDS_COMMAND | CAD_VENDOR_APPLY_COMMAND | CAD_VENDOR_STATUS_COMMAND
    )
}

/// Stage the records the owner tick took from the last import.
///
/// `Some([])` is a **new document with no vendor objects**: the previous pass is
/// cleared, so an old token can never settle into the replacement document. A
/// staging failure is reported through the next `vendor-settled` event instead of
/// being dropped.
pub fn stage_from_tick(shared: &Arc<Mutex<Shared>>, records: Option<Vec<VendorRecord>>) {
    let Some(records) = records else {
        return;
    };
    let mut state = lock(shared);
    // A published document starts with no derived geometry. The old snapshot is
    // cleared on the next tick even when the new document has vendor records;
    // the plugin's apply installs the replacement later.
    state.vendor_representations_clear = true;
    state.vendor_pending_representations = None;
    if records.is_empty() {
        state.vendor.clear();
        return;
    }
    if let Err(reason) = state.vendor.stage(records) {
        state.vendor.report_failure(reason);
    }
}

/// Apply the snapshots the owner tick owes to the render session.
///
/// The command runs on a worker thread and only stores the snapshot; this runs
/// on the owner thread, where `CadSession` lives. A snapshot whose pass is no
/// longer active (the user opened another document) is dropped.
pub fn apply_pending_representations(shared: &Arc<Mutex<Shared>>, session: &CadSession) {
    let (clear, pending, active) = {
        let mut state = lock(shared);
        (
            std::mem::take(&mut state.vendor_representations_clear),
            state.vendor_pending_representations.take(),
            state.vendor.active(),
        )
    };
    if clear {
        session.set_vendor_representations(None);
    }
    let Some(pending) = pending else {
        return;
    };
    if active == Some((pending.token, pending.generation)) {
        session.set_vendor_representations(Some(pending.store));
    }
}

/// `cad_vendor_records`: return the ABI v1 records frame as raw bytes.
///
/// The response is a `tauri::ipc::Response`, so the frontend receives an
/// `ArrayBuffer` instead of a JSON number array (see the Tauri v2 raw-response
/// path). A stale token is rejected with a stable `vendor.*` reason.
#[tauri::command]
pub fn cad_vendor_records(
    shared: tauri::State<'_, ProbeShared>,
    token: u64,
) -> Result<tauri::ipc::Response, String> {
    let frame = lock(&shared.0).vendor.records_frame(token)?;
    Ok(tauri::ipc::Response::new(frame))
}

/// `cad_vendor_apply`: validate and settle one pass.
///
/// `payload` is the base64 of the binary result frame. Base64 is used because
/// Tauri's raw **request** body is unavailable on Android; the response direction
/// still uses `ArrayBuffer`.
#[tauri::command]
pub fn cad_vendor_apply(
    shared: tauri::State<'_, ProbeShared>,
    token: u64,
    generation: u64,
    payload: String,
) -> Result<VendorAck, String> {
    let mut state = lock(&shared.0);
    let settlement = state
        .vendor
        .settle_base64_full(token, generation, &payload)?;
    let store = Arc::new(VendorRepresentationStore::new(token, settlement.primitives));
    state.vendor_pending_representations = Some(PendingVendorRepresentation {
        token,
        generation,
        store,
    });
    Ok(settlement.ack)
}

/// `cad_vendor_status`: the active pass descriptor, or explicit nulls.
#[tauri::command]
pub fn cad_vendor_status(shared: tauri::State<'_, ProbeShared>) -> Result<VendorStatus, String> {
    Ok(lock(&shared.0).vendor.status())
}

/// The three vendor commands as one typed handler.
///
/// Separate so a host that owns a larger dispatch table (Linux, Android) can
/// route to it without a second `generate_handler!` list.
pub fn register_vendor_commands<R: tauri::Runtime>(
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![cad_vendor_records, cad_vendor_apply, cad_vendor_status]
}

/// The shared command set **plus** the vendor transport.
///
/// This is what a host whose every carrier is `native-below` registers. The
/// command name is read before `invoke` is moved, so the vendor handler only ever
/// sees its own three names and the shared handler stays byte-for-byte unchanged.
pub fn register_commands_with_vendor<R: tauri::Runtime>(
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let shared = crate::commands::register_commands::<R>();
    let vendor = register_vendor_commands::<R>();
    move |invoke: Invoke<R>| {
        let command = invoke.message.command().to_string();
        if is_vendor_command(&command) {
            vendor(invoke)
        } else {
            shared(invoke)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_names_are_vendor_commands() {
        assert!(is_vendor_command(CAD_VENDOR_RECORDS_COMMAND));
        assert!(is_vendor_command(CAD_VENDOR_APPLY_COMMAND));
        assert!(is_vendor_command(CAD_VENDOR_STATUS_COMMAND));
        assert!(!is_vendor_command("cad_status"));
        assert!(!is_vendor_command("cad_command"));
        assert!(!is_vendor_command(""));
    }

    fn record() -> VendorRecord {
        VendorRecord {
            class_name: "TCH_WALL".to_string(),
            handle: 0x2c,
            dwg_version: 1027,
            source: cad_app::VendorSource::UnknownEntity,
            main: vec![0x06],
            main_bits: 8,
            text: Vec::new(),
            text_bits: 0,
            handle_stream: Vec::new(),
            handle_bits: 0,
            main_skip_bits: 0,
            text_skip_bits: 0,
            handle_skip_bits: 0,
            references: Vec::new(),
        }
    }

    #[test]
    fn a_new_document_without_vendor_objects_clears_the_previous_pass() {
        let shared = Arc::new(Mutex::new(Shared::new(
            cad_ui_core::launch::LaunchOptions::default(),
        )));
        lock(&shared).vendor.enable();
        stage_from_tick(&shared, Some(vec![record()]));
        assert!(lock(&shared).vendor.active().is_some());
        // `Some([])` is a real new-document answer: the old token must not
        // survive into the replacement.
        stage_from_tick(&shared, Some(Vec::new()));
        assert_eq!(lock(&shared).vendor.active(), None);
        // `None` is "nothing new", not a clear.
        stage_from_tick(&shared, None);
        assert_eq!(lock(&shared).vendor.active(), None);
    }
}
