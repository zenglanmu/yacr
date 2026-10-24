//! The `dom-canvas` **session descriptor relay** (ADR 0024 D2 / iron rule 8).
//!
//! On the `dom-canvas` carrier the wasm CAD session inside the WebView is the only
//! `cad-db` authority, so the native shell owns **no** document state: its own
//! `cad_status` honestly reports `documentLabel: null` (`unsupported_status`).
//! The chrome still has to *name* the open drawing, so the session reports an
//! opaque descriptor to this module ([`CANVAS_SESSION_STATUS_COMMAND`]); the host
//! stores it verbatim and relays it back as `cad_status.canvasSession`
//! (`crate::host::dispatch`).
//!
//! # Why this does not violate iron rule 8
//!
//! The stored value is **the session's own report, relayed unchanged**. The host
//! never opens the drawing, parses it, derives anything from it, validates
//! against it, or makes a decision with it — the value is display data with a
//! fixed shape (`label`, `batches`, `draws`), not a `cad-db`, not a scene and not
//! derived geometry. It is **bounded on arrival** ([`sanitize_label`],
//! [`saturate`]) so a buggy or hostile producer cannot make the host hold
//! unbounded memory, and it is forgotten on every event that already ends the
//! session ([`on_session_reset`]). There is nowhere in the shape to put file
//! paths or drawing content, and nothing here reads a file or a database.
//!
//! # One descriptor, no history
//!
//! The newest report replaces the previous one. A `dom-canvas` host keeps a single
//! descriptor because there is a single wasm session; a report from a session that
//! has been replaced must not be able to resurrect an old name (hence the reset
//! seam).
//!
//! # Wire
//!
//! `canvas_session_status` takes a flat JSON object
//! `{ label: string, batches: number, draws: number }`. Unknown extra keys are
//! ignored on purpose ([`CanvasSessionReport`] has no `deny_unknown_fields`), so
//! the wasm side can grow its report without breaking this host; missing keys
//! default to the honest zero/empty value instead of failing a display label.
//! On `native-below` the report is refused with the stable
//! `unsupported_on_surface_mode: …` reason: that carrier has a real session whose
//! own `cad_status` is the only status path, so a second one must not appear.

use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::backend::{surface_mode, SurfaceMode};

/// The command the wasm session calls to report its own descriptor.
pub const CANVAS_SESSION_STATUS_COMMAND: &str = "canvas_session_status";

/// Upper bound of a relayed label, in **characters** (not bytes).
///
/// Chosen to cover a full path-less file name plus a suffix without letting a
/// report grow host memory: 260 characters is the conventional maximum for a
/// display name, and the clamp is applied before anything is stored.
pub const MAX_LABEL_CHARS: usize = 260;

/// Explicit marker appended when a label had to be truncated.
///
/// Truncation is marked rather than rejected: a display string is not a
/// correctness-critical input, and a rejected report would leave the chrome
/// nameless. The marker makes the shortening visible instead of silent.
pub const TRUNCATION_MARKER: &str = "[truncated]";

/// The relayed descriptor: exactly the three fields the report may carry.
///
/// This is a **reported descriptor**, never document state: no entity count, no
/// path, no selection, no geometry, no camera. The shape is asserted by
/// `the_host_stores_only_the_descriptor_shape`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CanvasSessionDescriptor {
    /// The session's own display name for the open drawing, bounded by
    /// [`sanitize_label`].
    pub label: String,
    /// Batches the session has submitted, saturated to `u32`.
    pub batches: u32,
    /// Draws the session has issued, saturated to `u32`.
    pub draws: u32,
}

/// The wire body of a report, as the wasm session sends it.
///
/// Deliberately **not** `deny_unknown_fields`: the session owns this vocabulary
/// and may add fields; the host reads the three it understands and ignores the
/// rest. Counts are parsed as `u64` so a wider value from a growing session is
/// saturated by [`saturate`] instead of failing deserialization.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct CanvasSessionReport {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub batches: u64,
    #[serde(default)]
    pub draws: u64,
}

impl CanvasSessionReport {
    /// The bounded descriptor this report describes. Pure.
    pub fn descriptor(&self) -> CanvasSessionDescriptor {
        CanvasSessionDescriptor {
            label: sanitize_label(&self.label),
            batches: saturate(self.batches),
            draws: saturate(self.draws),
        }
    }
}

/// The acknowledgement returned to the reporting session.
///
/// `accepted` is what the caller checks; the echoed descriptor shows exactly what
/// the host stored after clamping, so a shortening is visible to the producer
/// instead of being guessed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CanvasSessionAck {
    pub accepted: bool,
    #[serde(flatten)]
    pub descriptor: CanvasSessionDescriptor,
}

/// The process-global descriptor slot.
///
/// A `Mutex<Option<_>>` is sufficient and intentional: this is one reported
/// display value, not a session, not a queue, not a document. It lives outside
/// `Shared` because it is host transport state (like the carrier itself) rather
/// than `cad-db` state, and because the reset seam is reached from the GTK/webview
/// hooks that do not all carry the shared-state handle.
#[derive(Debug, Default)]
pub struct CanvasSessionStore {
    slot: Mutex<Option<CanvasSessionDescriptor>>,
}

impl CanvasSessionStore {
    /// An empty store (no descriptor reported yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Store `descriptor` and return the stored value.
    pub fn report(&self, descriptor: CanvasSessionDescriptor) -> CanvasSessionDescriptor {
        let mut slot = self.lock();
        *slot = Some(descriptor.clone());
        descriptor
    }

    /// The last reported descriptor, if any.
    pub fn current(&self) -> Option<CanvasSessionDescriptor> {
        self.lock().clone()
    }

    /// Forget the descriptor (idempotent).
    pub fn clear(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> MutexGuard<'_, Option<CanvasSessionDescriptor>> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

static STORE: OnceLock<CanvasSessionStore> = OnceLock::new();

/// The process-global store the command and `cad_status` share.
pub fn global_store() -> &'static CanvasSessionStore {
    STORE.get_or_init(CanvasSessionStore::new)
}

/// Bound a reported label: drop control characters, then truncate with
/// [`TRUNCATION_MARKER`].
///
/// Control characters are removed rather than rejected because this is a display
/// string: a newline or an escape sequence must not reach the chrome, but it also
/// must not cost the user the name. Truncation counts **characters**, so a
/// multi-byte label is cut at a character boundary and stays valid UTF-8. An
/// empty label stays empty (the drawing is simply unnamed); nothing is invented.
pub fn sanitize_label(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if cleaned.chars().count() <= MAX_LABEL_CHARS {
        return cleaned;
    }
    let keep = MAX_LABEL_CHARS.saturating_sub(TRUNCATION_MARKER.chars().count());
    let mut clamped: String = cleaned.chars().take(keep).collect();
    clamped.push_str(TRUNCATION_MARKER);
    clamped
}

/// Saturate a reported count into the `u32` the descriptor promises.
pub fn saturate(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The explicit refusal for a report on a carrier that has no wasm session.
///
/// Pure and stable, mirroring `dispatch::native_session_refusal`: it names the
/// command, the carrier the report belongs to and the carrier the host runs, so a
/// caller never has to guess why nothing was stored.
pub fn report_refusal(mode: SurfaceMode) -> Option<String> {
    if mode == SurfaceMode::DomCanvas {
        return None;
    }
    Some(format!(
        "unsupported_on_surface_mode: {CANVAS_SESSION_STATUS_COMMAND} is a surfaceMode={} report, \
         and this host runs {} (its native session reports the document through the one cad_status \
         path, so no second status path is kept)",
        SurfaceMode::DomCanvas.wire_name(),
        mode.wire_name()
    ))
}

/// Store a report on `mode`'s behalf, or refuse it explicitly.
///
/// Mode and store are parameters (not globals) so the round-trip and the refusal
/// are unit-tested without installing a process-wide carrier.
pub fn report_for(
    mode: SurfaceMode,
    store: &CanvasSessionStore,
    wire: &CanvasSessionReport,
) -> Result<CanvasSessionAck, String> {
    if let Some(reason) = report_refusal(mode) {
        // No silent success: a refused report stores nothing.
        return Err(reason);
    }
    let descriptor = store.report(wire.descriptor());
    Ok(CanvasSessionAck {
        accepted: true,
        descriptor,
    })
}

/// Store a report on the installed carrier.
pub fn report(wire: &CanvasSessionReport) -> Result<CanvasSessionAck, String> {
    report_for(surface_mode(), global_store(), wire)
}

/// The descriptor `cad_status` relays, or `None` before the first report.
pub fn current_descriptor() -> Option<CanvasSessionDescriptor> {
    global_store().current()
}

/// The reset seam: forget the descriptor on an event that ends the session.
///
/// Called from the **same** hooks that already reset the shell state (page reload,
/// renderer crash, process exit), so a stale drawing name cannot outlive the
/// session that reported it. Idempotent; the trigger is logged for evidence.
pub fn on_session_reset(trigger: &str) {
    global_store().clear();
    eprintln!("[probe] canvas session descriptor cleared trigger={trigger}");
}

/// `canvas_session_status` command body (the `#[tauri::command]` wrapper lives in
/// [`super::dispatch`], with the other host commands).
///
/// Takes the raw invoke body so the **flat** object
/// `{ label, batches, draws }` is parsed by [`CanvasSessionReport`], which ignores
/// unknown keys (see the module docs). Tauri's per-argument deserialization cannot
/// express that tolerance for a flat object, and nesting the report under an extra
/// key would change the wire the session sends.
///
/// On `native-below` the promise is rejected with [`report_refusal`]'s reason;
/// nothing is stored.
pub fn report_from_body(body: &tauri::ipc::InvokeBody) -> Result<CanvasSessionAck, String> {
    let wire: CanvasSessionReport = match body {
        tauri::ipc::InvokeBody::Json(value) => {
            serde_json::from_value(value.clone()).map_err(|error| {
                format!("{CANVAS_SESSION_STATUS_COMMAND}: malformed report: {error}")
            })?
        }
        _ => {
            return Err(format!(
                "{CANVAS_SESSION_STATUS_COMMAND}: expected a JSON report object"
            ))
        }
    };
    report(&wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(label: &str, batches: u64, draws: u64) -> CanvasSessionReport {
        CanvasSessionReport {
            label: label.to_string(),
            batches,
            draws,
        }
    }

    #[test]
    fn a_report_round_trips_on_dom_canvas() {
        let store = CanvasSessionStore::new();
        let ack = report_for(SurfaceMode::DomCanvas, &store, &wire("plan.dwg", 3, 7))
            .expect("a dom-canvas report is accepted");

        assert!(ack.accepted);
        let stored = store.current().expect("the descriptor is stored");
        assert_eq!(stored.label, "plan.dwg");
        assert_eq!(stored.batches, 3);
        assert_eq!(stored.draws, 7);
        // The ack echoes exactly what was stored (after clamping).
        assert_eq!(ack.descriptor, stored);

        // The wire shape is the frozen three-field descriptor.
        let serialized = serde_json::to_value(&stored).expect("descriptor serializes");
        assert_eq!(serialized["label"], "plan.dwg");
        assert_eq!(serialized["batches"], 3);
        assert_eq!(serialized["draws"], 7);

        // The newest report wins: one descriptor, no history.
        report_for(SurfaceMode::DomCanvas, &store, &wire("second.dwg", 1, 2)).expect("accepted");
        assert_eq!(
            store.current().map(|descriptor| descriptor.label),
            Some("second.dwg".to_string())
        );
    }

    #[test]
    fn a_report_is_refused_on_native_below_and_stores_nothing() {
        let store = CanvasSessionStore::new();
        let error = report_for(SurfaceMode::NativeBelow, &store, &wire("plan.dwg", 1, 1))
            .expect_err("native-below has no wasm session to report on");

        assert!(
            error.starts_with("unsupported_on_surface_mode"),
            "the reason must stay machine-readable: {error}"
        );
        assert!(error.contains(CANVAS_SESSION_STATUS_COMMAND), "{error}");
        assert!(error.contains("dom-canvas"), "{error}");
        assert!(error.contains("native-below"), "{error}");
        // No silent success: the refusal did not store anything.
        assert_eq!(store.current(), None);
        // The pure table says the same.
        assert_eq!(report_refusal(SurfaceMode::DomCanvas), None);
        assert!(report_refusal(SurfaceMode::NativeBelow).is_some());
    }

    #[test]
    fn the_command_path_refuses_on_the_installed_native_below_carrier() {
        // Unit tests run before any `install_surface_mode` call, so the installed
        // carrier is the documented `native-below` default. This exercises the
        // real command entry (`report`), not just the pure table.
        assert_eq!(
            surface_mode(),
            SurfaceMode::NativeBelow,
            "this test pins the documented default carrier"
        );
        let error = report(&wire("plan.dwg", 1, 1)).expect_err("refused on native-below");
        assert!(error.starts_with("unsupported_on_surface_mode"), "{error}");
    }

    #[test]
    fn the_reset_seam_forgets_the_process_global_descriptor() {
        // The only test that writes the process-global store: report a descriptor
        // into it directly (mode-independent), then clear through the seam the
        // reload/crash/exit hooks call.
        let descriptor = report_for(
            SurfaceMode::DomCanvas,
            global_store(),
            &wire("global.dwg", 2, 2),
        )
        .expect("accepted")
        .descriptor;
        assert_eq!(current_descriptor(), Some(descriptor));

        on_session_reset("reload");
        assert_eq!(current_descriptor(), None, "a reload must forget the name");

        // Idempotent: a second reset is a no-op, not an error.
        on_session_reset("reload");
        assert_eq!(current_descriptor(), None);

        // A store-level clear behaves the same (the seam is a thin wrapper).
        let store = CanvasSessionStore::new();
        report_for(SurfaceMode::DomCanvas, &store, &wire("x", 1, 1)).expect("accepted");
        store.clear();
        assert_eq!(store.current(), None);
    }

    #[test]
    fn every_existing_reset_trigger_also_clears_the_descriptor() {
        // Source-structure contract: the three hooks that already reset the shell
        // state (page reload, renderer crash, process exit) must also forget the
        // descriptor, so no stale drawing name can outlive its session.
        let init = include_str!("init.rs");
        assert_eq!(
            init.matches("canvas_session::on_session_reset(\"reload\")")
                .count(),
            1,
            "init.rs must clear the descriptor on the page-load/reload trigger"
        );

        let popup = include_str!("popup.rs");
        assert_eq!(
            popup
                .matches("canvas_session::on_session_reset(\"renderer-crash\")")
                .count(),
            1,
            "popup.rs must clear the descriptor on the renderer-crash trigger"
        );

        let entry = include_str!("../entry.rs");
        assert_eq!(
            entry
                .matches("canvas_session::on_session_reset(\"exit\")")
                .count(),
            2,
            "entry.rs must clear the descriptor on ExitRequested and Exit"
        );
    }

    #[test]
    fn labels_are_bounded_and_control_characters_are_dropped() {
        assert_eq!(sanitize_label("plain.dwg"), "plain.dwg");
        // Control characters never reach the chrome, and they do not cost the
        // rest of the name.
        assert_eq!(sanitize_label("a\nb\tc\u{0}d\u{7}e\u{1b}[31m"), "abcde[31m");
        assert_eq!(sanitize_label(""), "");

        // Exactly at the bound: unchanged, no marker.
        let exact = "x".repeat(MAX_LABEL_CHARS);
        assert_eq!(sanitize_label(&exact), exact);

        // One over: truncated **with an explicit marker**, still at the bound.
        let clamped = sanitize_label(&"y".repeat(MAX_LABEL_CHARS + 1));
        assert!(clamped.ends_with(TRUNCATION_MARKER), "{clamped}");
        assert_eq!(clamped.chars().count(), MAX_LABEL_CHARS);
        assert!(clamped.chars().filter(|c| *c == 'y').count() < MAX_LABEL_CHARS);

        // Counted in characters, not bytes: a multi-byte label is cut on a
        // character boundary and stays valid UTF-8.
        let cjk = sanitize_label(&"図".repeat(MAX_LABEL_CHARS + 5));
        assert_eq!(cjk.chars().count(), MAX_LABEL_CHARS);
        assert!(cjk.ends_with(TRUNCATION_MARKER));
        assert_eq!(cjk, sanitize_label(&cjk), "clamping is stable");

        // A label that is only control characters becomes empty, never a panic.
        assert_eq!(sanitize_label("\u{0}\u{1}\u{7f}"), "");
    }

    #[test]
    fn counts_saturate_instead_of_overflowing() {
        assert_eq!(saturate(0), 0);
        assert_eq!(saturate(42), 42);
        assert_eq!(saturate(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(saturate(u64::MAX), u32::MAX);

        let descriptor = wire("plan.dwg", u64::MAX, 7).descriptor();
        assert_eq!(descriptor.batches, u32::MAX);
        assert_eq!(descriptor.draws, 7);
    }

    #[test]
    fn unknown_report_fields_are_ignored_so_the_session_can_grow() {
        let body = serde_json::json!({
            "label": "plan.dwg",
            "batches": 2,
            "draws": 4,
            "futureField": { "nested": [1, 2, 3] },
            "anotherOne": "ignored",
        });
        let parsed: CanvasSessionReport =
            serde_json::from_value(body).expect("unknown fields are ignored");
        let descriptor = parsed.descriptor();
        assert_eq!(descriptor.label, "plan.dwg");
        assert_eq!(descriptor.batches, 2);
        assert_eq!(descriptor.draws, 4);

        // A missing field is a default, not a hard failure of a display string.
        let sparse: CanvasSessionReport =
            serde_json::from_value(serde_json::json!({ "label": "plan.dwg" }))
                .expect("missing counts default");
        assert_eq!(sparse.descriptor().batches, 0);
        assert_eq!(sparse.descriptor().draws, 0);
    }

    #[test]
    fn the_host_stores_only_the_descriptor_shape() {
        // Wire-shape guard: exactly the three relayed fields. A future change
        // cannot smuggle document state (entity counts, paths, geometry,
        // selection) into the descriptor without failing this test.
        let serialized =
            serde_json::to_value(wire("plan.dwg", 1, 2).descriptor()).expect("serializes");
        let mut keys: Vec<&str> = serialized
            .as_object()
            .expect("descriptor is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["batches", "draws", "label"]);

        // Structural guard: the relay never touches a document authority. The
        // forbidden names are assembled at runtime so this test's own source is
        // not a false positive.
        let source = include_str!("canvas_session.rs");
        for forbidden in [
            format!("Cad{}", "Session"),
            format!("cad_{}", "db"),
            format!("std::{}", "fs"),
            format!("Path{}", "Buf"),
            format!("entity_{}", "count"),
        ] {
            assert!(
                !source.contains(&forbidden),
                "the descriptor relay must not reference {forbidden}"
            );
        }
    }
}
