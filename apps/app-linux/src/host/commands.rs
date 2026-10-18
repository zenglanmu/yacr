//! Tauri IPC commands: probe status, CAD command admission, CAD status and the
//! sequence-gated input funnel.

use cad_render_wgpu::PulledState;
use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::CadStatus;
use cad_ui_core::shell::{admit_command, projected_status, queue_cad_input};
use cad_ui_core::unsupported_status;

use super::postload::PostloadPanPhase;
use super::{lock, ProbeShared};

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    lock(&shared.0).shell.status.clone()
}

/// Admission result of `cad_command`: queued, **not** completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

/// `cad_command` command: admit a known CAD command into the bounded main-thread
/// queue.
///
/// An unknown command is rejected immediately (never queued). `{accepted:true}`
/// means the command was **queued**, not that it completed; the real outcome
/// surfaces through `cad_status.error`/`loading`.
#[tauri::command]
pub fn cad_command(
    shared: tauri::State<'_, ProbeShared>,
    command: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    admit_command(&mut state.shell, &command)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> CadStatus {
    let status = {
        let state = lock(&shared.0);
        let host_error = state
            .shell
            .cad_host_error
            .as_ref()
            .map(|error| error.text.as_str());
        match state.shell.cad_status.clone() {
            Some(snapshot) => projected_status(snapshot, host_error),
            None => unsupported_status(None),
        }
    };
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    // The pulled organisation state is appended as a separate token (the JSON is
    // unchanged and stays first) so a skipped accelerated organisation is visible
    // next to the status payload.
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        let pulled = lock(&shared.0)
            .last_pulled_state
            .as_ref()
            .map(PulledState::to_string)
            .unwrap_or_else(|| "-".to_string());
        match serde_json::to_string(&status) {
            Ok(json) => eprintln!("[probe] cad_status {json} pulled={pulled}"),
            Err(error) => eprintln!("[probe] cad_status serialize failed: {error} pulled={pulled}"),
        }
    }
    status
}

/// `probe_input` command: sequence-gate one frontend input and route it.
///
/// In CAD mode a pointer/wheel input is queued for the main thread only when the
/// CAD session is ready; before that it is refused with an explicit `cad_waiting`
/// reason (never a pretend success), and a full queue is refused explicitly.
/// `resize` is applied by the tick through the CAD rect path.
#[tauri::command]
pub fn probe_input(shared: tauri::State<'_, ProbeShared>, input: ProbeInput) -> ProbeInputAck {
    let kind = input.kind;
    let sequence = input.sequence;
    let ack = lock(&shared.0).shell.input.apply(&input);
    // Evidence line: shows the real IPC path (invoke -> command -> tracker).
    // x/y/dx/dy/width are logged so the accepted input can be tied to real OS
    // coordinates (diagnosis only; identifiers stay English).
    eprintln!(
        "[probe] input kind={} seq={} accepted={} marker={:.3} terminal={} reason={:?} x={:?} y={:?} dx={:?} dy={:?} width={:?}",
        kind.as_str(),
        sequence,
        ack.accepted,
        ack.marker_offset,
        ack.terminal,
        ack.reason,
        input.x,
        input.y,
        input.dx,
        input.dy,
        input.width,
    );
    if !ack.accepted {
        return ack;
    }
    // Post-load pan driver accounting: count host-admitted gesture inputs, and
    // flag a concurrent resize (which would make any view change unattributable
    // to the pan).
    {
        let mut state = lock(&shared.0);
        if let Some(pan) = state.postload_pan.as_mut() {
            match kind {
                InputKind::PointerMove => pan.admitted_moves += 1,
                InputKind::PointerUp => pan.admitted_ups += 1,
                InputKind::Resize => {
                    if matches!(
                        pan.phase,
                        PostloadPanPhase::Gesture | PostloadPanPhase::AwaitingFinal
                    ) {
                        pan.concurrent_resize = true;
                    }
                }
                _ => {}
            }
        }
    }
    if matches!(kind, InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(&shared.0);
        queue_cad_input(&mut state.shell, &input)
    };
    match queued {
        Ok(()) => ack,
        Err(reason) => ProbeInputAck {
            accepted: false,
            sequence,
            marker_offset: ack.marker_offset,
            terminal: ack.terminal,
            reason: Some(reason),
        },
    }
}
