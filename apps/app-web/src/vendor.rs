//! The **`dom-canvas` end of the vendor pass**: the carrier-neutral
//! [`VendorStore`] behind the three `cad_vendor_*` commands.
//!
//! # Why this module is thin
//!
//! The state machine (tokens, ABI v1 framing, `applied` rules, `vendor.*`
//! refusals) lives in [`cad_ui_core::session::vendor`], the same type the native
//! `app-linux`/`app-windows`/`app-macos`/`app-android` hosts drive through
//! `cad-desktop-shell`. This module only adds the two things a wasm session owns:
//!
//! * the **thread-local store** (one wasm instance, one owner thread);
//! * the three exports the browser transport forwards (`vendor_records`,
//!   `vendor_apply`, `vendor_status`), plus the staging call the open path makes.
//!
//! No rule is re-implemented here, so a token or framing decision cannot drift
//! between a native host and the page carrier.
//!
//! # The pass on this carrier
//!
//! Unlike a native host there is no separate owner tick: the document opens
//! **inside** the same call that flushes the event batch. So
//! [`Live::install`](crate::live::Live::install) and
//! [`Live::replace_document`](crate::live::Live::replace_document) stage the
//! records right after the import succeeds, and the export's own
//! [`flush_live`](crate::events::flush_live) emits `vendor-pending` in that same
//! batch. The settled pass is reported by the next flush, which is the
//! `cad_vendor_apply` call itself.
//!
//! # Enabled, deliberately
//!
//! The store starts [`enabled`](VendorStore::enable) because this carrier really
//! serves all three commands. A host that does **not** register them must never
//! enable the store (the frontend would call a command that is not there); here
//! the exports are compiled in, so the announcement points at a real command.
//!
//! # wasm-bindgen and the pass numbers
//!
//! `token`/`generation` are `u64`. wasm-bindgen maps a `u64` **parameter** to a
//! JS `bigint` (the WebAssembly `i64` rule), which is why the browser transport
//! converts the plain JSON numbers from `vendor-pending` with `BigInt(...)` before
//! calling these exports. The wire events keep numbers; only the call boundary is
//! `bigint`.

use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use cad_app::vendor::VendorRepresentationStore;
use cad_app::VendorRecord;
use cad_ui_core::session::events::CadEventKind;
use cad_ui_core::session::vendor::{VendorAck, VendorStatus, VendorStore};

thread_local! {
    /// The one pass this wasm instance owns (enabled: the exports exist).
    static STORE: RefCell<VendorStore> = RefCell::new(enabled_store());
}

/// A store that serves the vendor commands (see the module docs).
fn enabled_store() -> VendorStore {
    let mut store = VendorStore::default();
    store.enable();
    store
}

/// Run `f` over the this-instance store.
fn with_store<T>(f: impl FnOnce(&mut VendorStore) -> T) -> T {
    STORE.with(|store| f(&mut store.borrow_mut()))
}

/// Stage the records copied by the last successful import.
///
/// `None` is "nothing new" (the import produced no records for this open) and is
/// left alone. `Some([])` is a **new document with no vendor objects**: the
/// previous pass is cleared, so an old token can never settle into the replacement
/// document. A non-empty batch is staged, and a staging refusal is reported
/// through the next `vendor-settled` event instead of being dropped.
pub(crate) fn stage(records: Option<Vec<VendorRecord>>) {
    let Some(records) = records else {
        return;
    };
    if records.is_empty() {
        with_store(|store| store.clear());
        return;
    }
    let outcome = with_store(|store| store.stage(records));
    if let Err(reason) = outcome {
        with_store(|store| store.report_failure(reason));
    }
}

/// Take the vendor events the next flush owes, in their frozen order.
///
/// `vendor-pending` first, then `vendor-settled`: each is reported at most once
/// per pass, so a flush that runs twice cannot duplicate either.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn take_event_kinds() -> Vec<CadEventKind> {
    with_store(|store| {
        let mut kinds = Vec::new();
        if let Some((token, generation, entities)) = store.announce_pending() {
            kinds.push(CadEventKind::VendorPending {
                token,
                generation,
                entities,
            });
        }
        if let Some((token, ack)) = store.take_unreported() {
            kinds.push(CadEventKind::VendorSettled {
                token,
                applied: ack.applied,
                diagnostics: ack.diagnostics,
            });
        }
        kinds
    })
}

/// The ABI v1 records frame for `token`, or the store's stable refusal.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn records_frame(token: u64) -> Result<Vec<u8>, String> {
    with_store(|store| store.records_frame(token))
}

/// Settle the active pass from a base64-encoded result frame, discarding the
/// primitives (tests/back-compat; the export uses the full settlement).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn settle_base64(
    token: u64,
    generation: u64,
    payload: &str,
) -> Result<VendorAck, String> {
    settle_base64_full(token, generation, payload).map(|settlement| settlement.ack)
}

/// Settle the active pass from a base64-encoded result frame.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn settle_base64_full(
    token: u64,
    generation: u64,
    payload: &str,
) -> Result<cad_ui_core::session::vendor::VendorSettlement, String> {
    with_store(|store| store.settle_base64_full(token, generation, payload))
}

/// The active pass descriptor as JSON (explicit `null`s when nothing is armed).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn status_json() -> String {
    let status: VendorStatus = with_store(|store| store.status());
    serde_json::to_string(&status).expect("VendorStatus is a plain JSON value")
}

/// `cad_vendor_records`: the ABI v1 record frame, or a stable `vendor.*` reason.
///
/// Answers a `Uint8Array` on the JS side (wasm-bindgen's mapping of `Vec<u8>`),
/// which the browser transport converts to an `ArrayBuffer` for the plugin. It
/// never answers an empty buffer for a refused/stale token: that would look like
/// "a document with no vendor objects".
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn vendor_records(token: u64) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    records_frame(token).map_err(|message| wasm_bindgen::JsValue::from_str(&message))
}

/// `cad_vendor_apply`: validate and settle one pass, answering the JSON ack.
///
/// `payload` is the base64 of the plugin's binary result frame (base64 because
/// the browser transport already carries the apply body as JSON text). A stale
/// token/generation or a corrupt frame is refused with the store's stable
/// `vendor.*` reason and does **not** settle the pass. A valid frame installs its
/// primitives as the session's derived display snapshot (never `cad-db`).
///
/// A **settled** pass is flushed here, before the answer returns: unlike a native
/// host there is no owner tick to emit `vendor-settled` later, and waiting for the
/// next `frame()` would leave the chrome's pass pending for one more animation
/// frame. A refused frame settles nothing, so nothing is flushed (the diagnostic
/// is the rejection itself). The records/status exports stay side-effect free.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn vendor_apply(
    token: u64,
    generation: u64,
    payload: &str,
) -> Result<String, wasm_bindgen::JsValue> {
    let settlement = settle_base64_full(token, generation, payload)
        .map_err(|message| wasm_bindgen::JsValue::from_str(&message))?;
    let store = Arc::new(VendorRepresentationStore::new(token, settlement.primitives));
    crate::page::apply_vendor_representations(store);
    crate::events::flush_live();
    Ok(serde_json::to_string(&settlement.ack).expect("VendorAck is a plain JSON value"))
}

/// `cad_vendor_status`: the active pass descriptor, for a chrome that connected
/// after the `vendor-pending` frame.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn vendor_status() -> String {
    status_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::VendorSource;
    use cad_ui_core::session::vendor::{RESULT_MAGIC, VENDOR_ABI_VERSION};

    /// A store that serves the commands, fresh for this test.
    ///
    /// The store is a thread-local and the workspace runs tests with
    /// `--test-threads=1` in the full gate, so two tests can share a thread:
    /// resetting is what keeps them independent instead of relying on scheduler
    /// luck.
    fn reset() {
        STORE.with(|store| *store.borrow_mut() = enabled_store());
    }

    fn record(handle: u64) -> VendorRecord {
        VendorRecord {
            class_name: "TCH_WALL".to_string(),
            handle,
            dwg_version: 1027,
            source: VendorSource::UnknownEntity,
            main: vec![0x06, 0b1010_0000],
            main_bits: 10,
            text: vec![0b1100_0000],
            text_bits: 2,
            handle_stream: vec![0b0100_0000],
            handle_bits: 2,
            main_skip_bits: 3,
            text_skip_bits: 0,
            handle_skip_bits: 0,
            references: vec![0x2c],
        }
    }

    fn result_frame(token: u64, generation: u64) -> Vec<u8> {
        let mut frame = Vec::new();
        let diagnostic = b"vendor.unsupported";
        frame.extend_from_slice(&RESULT_MAGIC.to_le_bytes());
        frame.extend_from_slice(&VENDOR_ABI_VERSION.to_le_bytes());
        frame.extend_from_slice(&token.to_le_bytes());
        frame.extend_from_slice(&generation.to_le_bytes());
        // status 1 (unsupported), applied 0, one diagnostic, no entity groups.
        frame.extend_from_slice(&1u32.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.extend_from_slice(&1u32.to_le_bytes());
        frame.extend_from_slice(&(diagnostic.len() as u16).to_le_bytes());
        frame.extend_from_slice(diagnostic);
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = String::new();
        for chunk in bytes.chunks(3) {
            let mut buffer = 0u32;
            for (index, byte) in chunk.iter().enumerate() {
                buffer |= (*byte as u32) << (16 - index * 8);
            }
            let chars = match chunk.len() {
                1 => 2,
                2 => 3,
                _ => 4,
            };
            for index in 0..4 {
                if index < chars {
                    let shift = 18 - index * 6;
                    output.push(TABLE[((buffer >> shift) & 0x3f) as usize] as char);
                } else {
                    output.push('=');
                }
            }
        }
        output
    }

    /// The kind name of a vendor event (`vendor-pending` / `vendor-settled`).
    fn kind_name(kind: &CadEventKind) -> &'static str {
        match kind {
            CadEventKind::VendorPending { .. } => "vendor-pending",
            CadEventKind::VendorSettled { .. } => "vendor-settled",
            _ => "other",
        }
    }

    #[test]
    fn an_empty_batch_clears_the_active_pass() {
        reset();
        stage(Some(vec![record(0x2c)]));
        let (token, _) = with_store(|store| store.active()).expect("a pass is armed");
        assert!(records_frame(token).is_ok());

        // A new document with no vendor objects: the old pass must not survive.
        stage(Some(Vec::new()));
        assert_eq!(with_store(|store| store.active()), None);
        assert_eq!(
            records_frame(token),
            Err("vendor.no_pending_pass".to_string()),
            "an old token can never settle into the replacement document"
        );
        assert!(
            take_event_kinds().is_empty(),
            "a cleared pass announces nothing"
        );
        assert_eq!(
            status_json(),
            r#"{"token":null,"generation":null,"entities":0}"#
        );
    }

    #[test]
    fn nothing_new_stages_nothing() {
        reset();
        stage(Some(vec![record(1)]));
        let armed = with_store(|store| store.active()).expect("a pass is armed");
        stage(None);
        assert_eq!(with_store(|store| store.active()), Some(armed));
        let kinds = take_event_kinds();
        assert_eq!(kinds.len(), 1);
        assert_eq!(kind_name(&kinds[0]), "vendor-pending");
    }

    #[test]
    fn a_staged_pass_announces_once_and_settles_once() {
        reset();
        stage(Some(vec![record(0x2c), record(0x2d)]));
        let (token, generation) = with_store(|store| store.active()).expect("a pass is armed");

        // The first flush owes `vendor-pending`, the second owes nothing new.
        let pending = take_event_kinds();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0],
            CadEventKind::VendorPending {
                token,
                generation,
                entities: 2,
            }
        );
        assert!(take_event_kinds().is_empty(), "announced exactly once");

        // The records frame is the active pass's own frame (ABI v1 header).
        let frame = records_frame(token).expect("the active token serves its frame");
        assert_eq!(&frame[0..4], &0x5941_4e44u32.to_le_bytes());
        assert_eq!(&frame[4..8], &VENDOR_ABI_VERSION.to_le_bytes());

        let payload = encode_base64(&result_frame(token, generation));
        let ack =
            settle_base64(token, generation, &payload).expect("an unsupported result settles");
        assert!(!ack.accepted);
        assert_eq!(ack.applied, 0);
        assert_eq!(ack.diagnostics, ["vendor.unsupported"]);

        let settled = take_event_kinds();
        assert_eq!(settled.len(), 1);
        assert_eq!(
            settled[0],
            CadEventKind::VendorSettled {
                token,
                applied: 0,
                diagnostics: vec!["vendor.unsupported".to_string()],
            }
        );
        assert!(take_event_kinds().is_empty(), "reported exactly once");

        // Settled means the frame is no longer served, and the descriptor still
        // names the pass (a late chrome can recover it).
        assert_eq!(
            records_frame(token),
            Err("vendor.already_settled".to_string())
        );
        let status: serde_json::Value =
            serde_json::from_str(&status_json()).expect("valid status JSON");
        assert_eq!(status["token"], serde_json::json!(token));
        assert_eq!(status["generation"], serde_json::json!(generation));
        assert_eq!(status["entities"], serde_json::json!(2));
    }

    #[test]
    fn records_frame_rejects_stale_tokens() {
        reset();
        assert_eq!(
            records_frame(1),
            Err("vendor.no_pending_pass".to_string()),
            "nothing armed"
        );
        stage(Some(vec![record(1)]));
        let (token, _) = with_store(|store| store.active()).expect("a pass is armed");
        assert_eq!(
            records_frame(token + 1),
            Err("vendor.stale_token".to_string())
        );
        assert_eq!(
            settle_base64(
                token + 1,
                token + 1,
                &encode_base64(&result_frame(token, token))
            ),
            Err("vendor.stale_token".to_string())
        );
        assert_eq!(
            settle_base64(
                token,
                token + 1,
                &encode_base64(&result_frame(token, token))
            ),
            Err("vendor.stale_generation".to_string())
        );
    }

    #[test]
    fn a_refused_apply_leaves_nothing_for_the_export_flush() {
        reset();
        stage(Some(vec![record(1)]));
        let (token, generation) = with_store(|store| store.active()).expect("a pass is armed");
        let pending = take_event_kinds();
        assert_eq!(pending.len(), 1);
        assert_eq!(kind_name(&pending[0]), "vendor-pending");

        // A stale generation is refused and must not settle: the export flushes
        // right after a successful settle, so a refusal has nothing to emit and
        // the chrome only sees the rejection.
        assert_eq!(
            settle_base64(
                token,
                generation + 1,
                &encode_base64(&result_frame(token, generation + 1)),
            ),
            Err("vendor.stale_generation".to_string())
        );
        assert!(
            take_event_kinds().is_empty(),
            "a refused frame settles nothing, so the flush after it emits no vendor-settled"
        );

        // The pass is still armed and still settleable with the right frame.
        let payload = encode_base64(&result_frame(token, generation));
        assert!(settle_base64(token, generation, &payload).is_ok());
        let settled = take_event_kinds();
        assert_eq!(settled.len(), 1);
        assert_eq!(kind_name(&settled[0]), "vendor-settled");
    }

    #[test]
    fn a_staging_failure_is_reported_once() {
        reset();
        // `TCH_` is a valid class name, so use a record the frame cannot carry:
        // a class name longer than the u16 length prefix.
        let mut oversized = record(1);
        oversized.class_name = "TCH_".to_string() + &"X".repeat(u16::MAX as usize);
        stage(Some(vec![oversized]));

        let kinds = take_event_kinds();
        assert_eq!(kinds.len(), 1);
        match &kinds[0] {
            CadEventKind::VendorSettled {
                diagnostics,
                applied,
                ..
            } => {
                assert_eq!(*applied, 0);
                assert_eq!(diagnostics, &vec!["vendor.class_name_too_long".to_string()]);
            }
            other => panic!("expected vendor-settled, got {other:?}"),
        }
        assert!(take_event_kinds().is_empty(), "a failure is not replayed");
    }
}
