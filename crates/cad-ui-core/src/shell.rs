//! Generic host shell state and CAD admission (UI-framework-free).
//!
//! A native UI host shares a single GPU-free state object between its main
//! thread and any IPC workers: status, the input tracker, the bounded CAD
//! command/input queues and the host-side error model. This module owns that
//! state and the admission helpers so every host applies the **same** bounded
//! queue semantics and the **same** error projection.
//!
//! It carries no windowing toolkit and no GPU: the concrete host wraps
//! [`ShellState`] and adds its own toolkit-specific fields (windows, redraw
//! pumps, telemetry). A host that keeps its probe/Tauri fields in the same
//! struct can deref to [`ShellState`] so its existing field accesses resolve
//! here.

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); `web-time` re-exports std elsewhere, so this
// is a no-op off wasm.
use web_time::Instant;

use crate::protocol::{InputTracker, ProbeInput, ProbeStatus};
use crate::session::{
    canonical_command_name, unsupported_command_reason, unsupported_status, CadPopup, CadStatus,
};

/// Bounded main-thread CAD command queue capacity (admission only, not completion).
pub const CAD_COMMAND_CAP: usize = 256;
/// Bounded ordered main-thread CAD input queue capacity.
pub const CAD_INPUT_CAP: usize = 512;

/// Stable refusal reason while a modal popup holds the host input gate
/// (ADR-0021 D2). The gate is host-owned: it is **not** the UI's
/// `suspendPointer`, so an external click that releases that suspension can
/// never un-gate CAD input.
pub const POPUP_MODAL_OPEN: &str = "popup.modal_open";

/// The kind of a host-side (transport) CAD error.
///
/// A good event clears only its **own** kind, so a successful present never
/// hides an open failure and a successful bind never hides a present error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostErrorKind {
    /// `set_render_session` (device attach) failed.
    Bind,
    /// A CAD frame present/render failed.
    Present,
    /// The owner-thread CAD tick failed.
    Tick,
    /// The webview renderer process failed (the UI can no longer render).
    ///
    /// Recorded by trigger ③ of the popup lifecycle on the way to clearing the
    /// modal gate, so a crash is visible in `cad_status.error` instead of only
    /// releasing input silently.
    Renderer,
}

/// A host-side CAD error (technical caption + kind). Never UI prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub kind: HostErrorKind,
    pub text: String,
}

/// GPU-free state shared with a host's IPC commands.
///
/// Deliberately carries **no** toolkit or GPU object: only status, the input
/// tracker, the bounded CAD queues and the transport error cross threads.
#[derive(Debug)]
pub struct ShellState {
    /// The probe status returned to the frontend.
    pub status: ProbeStatus,
    /// Input/sequence tracker (marker offset and interaction state).
    pub input: InputTracker,
    /// Set by destroy/exit handling; the tick performs the stop and release.
    pub stop_requested: bool,
    /// Whether the presenter has been stopped and released.
    pub stopped: bool,
    /// Whether the last tick paused (minimized / zero-size) the probe.
    pub paused: bool,
    /// Count of ticks that passed the frame-rate throttle (diagnostic).
    pub ticks: u64,
    /// Chosen present mode label (evidence).
    pub present_mode: Option<String>,
    /// Whether the CAD session path is enabled (false in `--probe` mode).
    pub cad_mode: bool,
    /// Whether the main-thread CAD session has been created.
    pub cad_ready: bool,
    /// Bounded main-thread CAD command queue (admission only; not completion).
    pub cad_commands: Vec<String>,
    /// Bounded ordered main-thread CAD input queue.
    pub cad_inputs: Vec<ProbeInput>,
    /// Bounded per-input IPC receive time (`seq -> Instant`), for the env-gated
    /// drag-latency telemetry. Kept separate from the input queue (no queue
    /// change) and capped at [`CAD_INPUT_CAP`].
    pub cad_input_recv: Vec<(u64, Instant)>,
    /// Latest `cad_status` snapshot (main-thread derived; cloned for IPC).
    pub cad_status: Option<CadStatus>,
    /// Last real **host-side** (transport) CAD error (bind/present/tick). CAD
    /// business/open failures live in the CadSession snapshot instead.
    pub cad_host_error: Option<HostError>,
    /// The presenter generation whose bind attempt failed. A bind is attempted
    /// **once** per generation: until the generation changes (a new device) no
    /// further attempt is made, so there is no per-tick initialization loop.
    pub cad_bind_failed_generation: Option<u64>,
    /// The open modal popup, or `None` while none is open (ADR-0021 D1/D2).
    ///
    /// This is the host's **input gate** as well as the value projected onto
    /// `cad_status.popup`: while it is `Some`, [`admit_command`] and
    /// [`queue_cad_input`] refuse with [`POPUP_MODAL_OPEN`]. It is set only
    /// after a fully validated open request and cleared by exactly four
    /// triggers: an explicit close/Esc, a UI reload, a renderer process failure
    /// and process exit. It is therefore independent of any UI-side pointer
    /// suspension, which a blur can release at any time.
    pub popup: Option<CadPopup>,
}

impl ShellState {
    /// A fresh shell state for a host whose CAD session path is `cad_mode`.
    ///
    /// When the CAD session is disabled (probe mode) the initial status is an
    /// explicit "CAD is disabled" snapshot, never a fabricated document.
    pub fn new(cad_mode: bool) -> Self {
        Self {
            status: ProbeStatus::default(),
            input: InputTracker::new(),
            stop_requested: false,
            stopped: false,
            paused: false,
            ticks: 0,
            present_mode: None,
            cad_mode,
            cad_ready: false,
            cad_commands: Vec::new(),
            cad_inputs: Vec::new(),
            cad_input_recv: Vec::new(),
            cad_status: if cad_mode {
                None
            } else {
                Some(unsupported_status(Some(
                    "CAD is disabled in probe mode".into(),
                )))
            },
            cad_host_error: None,
            cad_bind_failed_generation: None,
            popup: None,
        }
    }
}

/// Record a host-side error of `kind` (replacing any previous host error).
pub fn note_host_error(state: &mut ShellState, kind: HostErrorKind, text: String) {
    state.cad_host_error = Some(HostError { kind, text });
}

/// Clear the host error **only** if it is of `kind` (never a different kind, and
/// never a CadSession business error, which lives in the snapshot).
pub fn clear_host_error(state: &mut ShellState, kind: HostErrorKind) {
    if state
        .cad_host_error
        .as_ref()
        .is_some_and(|error| error.kind == kind)
    {
        state.cad_host_error = None;
    }
}

/// Project the CadSession snapshot plus the host-side error into the status the
/// frontend reads.
///
/// No false clear: when the host error is absent the snapshot's own (business /
/// open) error is preserved; when both exist they are both shown.
pub fn projected_status(mut snapshot: CadStatus, host_error: Option<&str>) -> CadStatus {
    snapshot.error = match (host_error, snapshot.error.take()) {
        (Some(host), Some(business)) => Some(format!("{host}; {business}")),
        (Some(host), None) => Some(host.to_string()),
        (None, business) => business,
    };
    snapshot
}

/// Project the host's live modal-popup state onto a status snapshot.
///
/// The session never owns a window, so `popup` is `None` in every snapshot it
/// assembles; the host applies its real value here when it answers
/// `cad_status` (ADR-0021 D1/D2). Passing `None` clears the block, so a closed
/// popup is never reported as still open.
pub fn projected_popup(mut snapshot: CadStatus, popup: Option<&CadPopup>) -> CadStatus {
    snapshot.popup = popup.cloned();
    snapshot
}

/// Whether the host's modal input gate is set (ADR-0021 D2).
///
/// Independent from any UI-side pointer suspension: it is cleared only by the
/// popup lifecycle (explicit close, reload, renderer crash, process exit),
/// never by a DOM `blur`.
pub fn modal_gate_open(state: &ShellState) -> bool {
    state.popup.is_some()
}

/// Whether a bind may be attempted: never once ready, and never again for the
/// generation that already failed (fail-stop until the generation changes).
pub fn bind_allowed(ready: bool, failed_generation: Option<u64>, owner_generation: u64) -> bool {
    !ready && failed_generation != Some(owner_generation)
}

/// Admit a command into the bounded main-thread queue (unknown refused first).
///
/// A name outside the command vocabulary is refused with a stable reason before
/// the queue is touched; a disabled CAD session and a full queue are refused
/// explicitly. The canonical selector is stored, so the queue never carries an
/// alias.
pub fn admit_command(state: &mut ShellState, command: &str) -> Result<(), String> {
    // The modal gate comes first: while a popup owns input, *every* CAD command
    // is refused by name (even one the router would otherwise reject), and the
    // refusal never depends on the UI having stopped forwarding.
    if modal_gate_open(state) {
        return Err(POPUP_MODAL_OPEN.into());
    }
    let Some(normalized) = canonical_command_name(command) else {
        return Err(unsupported_command_reason(command));
    };
    if !state.cad_mode {
        return Err("CAD is disabled in probe mode".into());
    }
    if state.cad_commands.len() >= CAD_COMMAND_CAP {
        return Err("CAD command queue is full".into());
    }
    state.cad_commands.push(normalized.to_string());
    Ok(())
}

/// Queue one ordered CAD input, or refuse explicitly (not ready / full).
pub fn queue_cad_input(state: &mut ShellState, input: &ProbeInput) -> Result<(), String> {
    // Host-side second refusal while a modal popup owns input (ADR-0021 D2):
    // this does not rely on the UI's own suspend switch, which a blur can
    // release at any time.
    if modal_gate_open(state) {
        return Err(POPUP_MODAL_OPEN.into());
    }
    if !state.cad_mode {
        // Probe marker path keeps the tracker semantics.
        return Ok(());
    }
    if !state.cad_ready {
        return Err("cad_waiting".into());
    }
    if state.cad_inputs.len() >= CAD_INPUT_CAP {
        return Err("input_queue_full".into());
    }
    state.cad_inputs.push(input.clone());
    // Record the IPC receive time for the env-gated drag-latency telemetry.
    state.cad_input_recv.push((input.sequence, Instant::now()));
    if state.cad_input_recv.len() > CAD_INPUT_CAP {
        state.cad_input_recv.remove(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InputKind;
    use crate::session::CadSelection;

    fn input(kind: InputKind) -> ProbeInput {
        ProbeInput {
            sequence: 1,
            kind,
            x: Some(1.0),
            y: Some(2.0),
            dx: None,
            dy: None,
            width: None,
            height: None,
            dpr: None,
            button: None,
            shift: None,
        }
    }

    fn sample_status(error: Option<&str>) -> CadStatus {
        CadStatus {
            document_label: Some("demo".into()),
            entity_count: Some(1),
            loading: false,
            import_progress: None,
            error: error.map(str::to_string),
            can_new: true,
            can_open: true,
            can_save: true,
            can_plot: true,
            can_fit: true,
            backend: Some("vulkan".into()),
            adapter: Some("P600".into()),
            camera: None,
            selection: CadSelection::empty(),
            tool: None,
            hover_preview: false,
            snap_enabled: false,
            grid_enabled: false,
            ortho_enabled: false,
            snap_kind: None,
            crosshair: false,
            import_count: 0,
            frame_count: 0,
            command_log: Vec::new(),
            popup: None,
        }
    }

    #[test]
    fn unsupported_status_is_explicit_and_not_fake() {
        let status = unsupported_status(None);
        assert_eq!(status.document_label, None);
        assert_eq!(status.entity_count, None);
        assert_eq!(status.camera, None);
        assert!(!status.can_open && !status.can_fit && !status.loading);
        assert!(
            !status.can_new && !status.can_save && !status.can_plot,
            "a snapshot with no session advertises no NEW/SAVE/PLOT either"
        );
        assert_eq!(status.import_count, 0);
        assert_eq!(status.frame_count, 0);
        assert!(
            !status.snap_enabled,
            "no session means no installed switch to advertise"
        );
        assert!(
            !status.grid_enabled,
            "no session means no installed display-grid switch either"
        );
        assert!(
            !status.ortho_enabled,
            "no session means no installed ORTHO switch either"
        );
        // The selection block is present and honest: nothing can be selected
        // when no CAD session exists.
        assert_eq!(status.selection, CadSelection::empty());
        assert_eq!(status.selection.count, 0);
        assert!(status.selection.empty);
        let with_error = unsupported_status(Some("boom".into()));
        assert_eq!(with_error.error.as_deref(), Some("boom"));
    }

    #[test]
    fn command_admission_normalizes_and_is_bounded() {
        assert_eq!(canonical_command_name(" open "), Some("OPEN"));
        assert_eq!(canonical_command_name("FIT"), Some("FIT"));
        assert_eq!(canonical_command_name("line"), Some("LINE"));
        for rejected in ["", "BOGUS", "OPEN FIT", "OPENX"] {
            assert_eq!(canonical_command_name(rejected), None, "{rejected:?}");
        }

        let mut state = ShellState::new(true);
        assert!(state.cad_mode && !state.cad_ready);
        assert!(admit_command(&mut state, "BOGUS").is_err());
        assert!(admit_command(&mut state, "OPEN").is_ok());
        assert_eq!(state.cad_commands, vec!["OPEN".to_string()]);
        for _ in 1..CAD_COMMAND_CAP {
            admit_command(&mut state, "FIT").unwrap();
        }
        assert_eq!(state.cad_commands.len(), CAD_COMMAND_CAP);
        assert!(
            admit_command(&mut state, "FIT").is_err(),
            "the command queue is bounded at {CAD_COMMAND_CAP}"
        );
    }

    #[allow(deprecated)] // the frozen `cad_status` wire label
    fn about_popup() -> CadPopup {
        CadPopup {
            kind: "about".into(),
            label: "popup-1".into(),
            modal: true,
        }
    }

    #[test]
    fn modal_popup_refuses_commands_and_input_until_cleared() {
        let mut state = ShellState::new(true);
        state.cad_ready = true;
        // No popup: the normal admission path applies.
        assert!(admit_command(&mut state, "OPEN").is_ok());
        assert!(queue_cad_input(&mut state, &input(InputKind::PointerDown)).is_ok());
        assert!(!modal_gate_open(&state));

        state.popup = Some(about_popup());
        assert!(modal_gate_open(&state));
        // Every command/input is refused by the named modal reason — including
        // an unknown command, which never reaches the router behind the gate.
        assert_eq!(
            admit_command(&mut state, "BOGUS").unwrap_err(),
            POPUP_MODAL_OPEN
        );
        assert_eq!(
            admit_command(&mut state, "OPEN").unwrap_err(),
            POPUP_MODAL_OPEN
        );
        assert_eq!(
            queue_cad_input(&mut state, &input(InputKind::PointerDown)).unwrap_err(),
            POPUP_MODAL_OPEN
        );
        assert_eq!(state.cad_commands, vec!["OPEN".to_string()]);
        assert_eq!(state.cad_inputs.len(), 1);

        // Clearing the host gate (an explicit close) admits both paths again
        // without any UI-side action.
        state.popup = None;
        assert!(!modal_gate_open(&state));
        assert!(admit_command(&mut state, "FIT").is_ok());
        assert!(queue_cad_input(&mut state, &input(InputKind::PointerDown)).is_ok());
    }

    #[test]
    fn projected_popup_reports_the_real_state_and_clears_it() {
        let status = projected_popup(unsupported_status(None), None);
        assert_eq!(status.popup, None);

        let popup = about_popup();
        let status = projected_popup(unsupported_status(None), Some(&popup));
        assert_eq!(status.popup, Some(popup.clone()));

        // After a close the block is `null` again, never a stale popup.
        let status = projected_popup(status, None);
        assert_eq!(status.popup, None);
    }

    #[test]
    fn input_queue_waits_for_cad_then_bounds() {
        let mut state = ShellState::new(true);
        // Before the CAD session exists an input is refused explicitly.
        assert_eq!(
            queue_cad_input(&mut state, &input(InputKind::PointerDown)).unwrap_err(),
            "cad_waiting"
        );
        state.cad_ready = true;
        for _ in 0..CAD_INPUT_CAP {
            queue_cad_input(&mut state, &input(InputKind::PointerMove)).unwrap();
        }
        assert_eq!(state.cad_inputs.len(), CAD_INPUT_CAP);
        assert_eq!(
            queue_cad_input(&mut state, &input(InputKind::PointerMove)).unwrap_err(),
            "input_queue_full"
        );
    }

    #[test]
    fn probe_mode_reports_cad_unsupported() {
        let state = ShellState::new(false);
        assert!(!state.cad_mode);
        let status = state.cad_status.clone().expect("probe status present");
        assert!(status.error.is_some());
        assert!(!status.can_open && !status.can_fit);
        assert_eq!(status.import_count, 0);
    }

    #[test]
    fn bind_error_is_visible_and_never_marks_ready() {
        let mut state = ShellState::new(true);
        assert!(!state.cad_ready);
        note_host_error(
            &mut state,
            HostErrorKind::Bind,
            "cad bind failed: attach".into(),
        );
        // A bind failure never marks the session ready.
        assert!(!state.cad_ready);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad bind failed: attach"));
        assert!(
            projected.can_fit,
            "the session document capability is preserved"
        );
        assert!(projected.can_open);
    }

    #[test]
    fn present_failure_is_visible_without_business_error_then_clears() {
        let mut state = ShellState::new(true);
        note_host_error(
            &mut state,
            HostErrorKind::Present,
            "cad present failed: gpu".into(),
        );
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad present failed: gpu"));
        clear_host_error(&mut state, HostErrorKind::Present);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error, None);
    }

    #[test]
    fn open_failure_persists_despite_present_success() {
        let mut state = ShellState::new(true);
        // While both exist they are both shown.
        note_host_error(
            &mut state,
            HostErrorKind::Present,
            "cad present failed: gpu".into(),
        );
        let projected = projected_status(
            sample_status(Some("read failed")),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(
            projected.error.as_deref(),
            Some("cad present failed: gpu; read failed")
        );
        // A successful present clears only the present error; the open failure stays.
        clear_host_error(&mut state, HostErrorKind::Present);
        let projected = projected_status(
            sample_status(Some("read failed")),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("read failed"));
    }

    #[test]
    fn host_error_clear_is_kind_specific() {
        let mut state = ShellState::new(true);
        note_host_error(&mut state, HostErrorKind::Bind, "cad bind failed: x".into());
        clear_host_error(&mut state, HostErrorKind::Present);
        assert!(
            state.cad_host_error.is_some(),
            "a present clear never clears a bind error"
        );
        clear_host_error(&mut state, HostErrorKind::Bind);
        assert!(state.cad_host_error.is_none());
    }

    #[test]
    fn previous_host_error_does_not_stick_once_corrected() {
        let mut state = ShellState::new(true);
        note_host_error(&mut state, HostErrorKind::Tick, "cad tick failed: y".into());
        clear_host_error(&mut state, HostErrorKind::Tick);
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error, None);
    }

    #[test]
    fn bind_attempts_are_bounded_per_generation() {
        // Once a generation has failed, 100 further ticks never re-attempt it.
        let failed = Some(3u64);
        for _ in 0..100 {
            assert!(
                !bind_allowed(false, failed, 3),
                "no repeated bind in the same generation"
            );
        }
        // A new presenter generation allows exactly one fresh attempt.
        assert!(bind_allowed(false, failed, 4));
        // A generation 0 failure is also fail-stopped until the generation changes.
        assert!(!bind_allowed(false, Some(0), 0));
        assert!(bind_allowed(false, Some(0), 1));
        // Ready never attempts; no failure yet is allowed.
        assert!(!bind_allowed(true, None, 4));
        assert!(bind_allowed(false, None, 0));
    }

    #[test]
    fn bind_error_survives_a_waiting_present() {
        let mut state = ShellState::new(true);
        note_host_error(&mut state, HostErrorKind::Bind, "cad bind failed: x".into());
        // An explicit-waiting present (`Ok(None)`) clears nothing, so the bind
        // error stays visible.
        clear_host_error(&mut state, HostErrorKind::Present);
        assert!(state.cad_host_error.is_some());
        let projected = projected_status(
            sample_status(None),
            state.cad_host_error.as_ref().map(|e| e.text.as_str()),
        );
        assert_eq!(projected.error.as_deref(), Some("cad bind failed: x"));
    }
}
