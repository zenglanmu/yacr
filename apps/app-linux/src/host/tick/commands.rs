//! One tick's CAD command/input batch: queued commands, ordered inputs, typed
//! writes and the single owner-thread session tick.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use cad_desktop_shell::commands::{drain_orbits, drain_standard_views};
use cad_ui_core::shell::{clear_host_error, note_host_error, HostErrorKind};

use super::super::layers;
use super::super::pinch_intake;
use super::super::{apply_setting_write, lock, Shared, CAD};

/// Drain this tick's queued CAD work onto the session and run one owner tick.
///
/// Returns the batch's latest input sequence and its IPC-to-drain age (both
/// `None` when nothing was queued), for the frame evidence line.
pub(super) fn apply_batch(
    shared: &Arc<Mutex<Shared>>,
    cad_mode: bool,
    cad_log: bool,
) -> (Option<u64>, Option<u128>) {
    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    // Telemetry: this batch's latest input sequence and its IPC-to-drain age.
    let mut last_input_seq: Option<u64> = None;
    let mut input_age_us: Option<u128> = None;
    if cad_mode {
        let (
            commands,
            mut standard_views,
            mut orbits,
            setting_writes,
            layer_writes,
            tool_parameters,
            tool_texts,
            selected_texts,
            selected_properties,
            selection_requests,
            inputs,
        ) = {
            let mut state = lock(shared);
            pinch_intake::append_native_pinches(&mut state); // DOM first
            (
                std::mem::take(&mut state.shell.cad_commands),
                std::mem::take(&mut state.cad_standard_views),
                std::mem::take(&mut state.cad_orbits),
                std::mem::take(&mut state.cad_setting_writes),
                std::mem::take(&mut state.cad_layer_writes),
                std::mem::take(&mut state.cad_tool_parameters),
                std::mem::take(&mut state.cad_tool_texts),
                std::mem::take(&mut state.cad_selected_texts),
                std::mem::take(&mut state.cad_selected_properties),
                std::mem::take(&mut state.cad_selection_requests),
                std::mem::take(&mut state.shell.cad_inputs),
            )
        };
        let queued = commands.len()
            + standard_views.len()
            + orbits.len()
            + setting_writes.len()
            + layer_writes.len()
            + tool_parameters.len()
            + tool_texts.len()
            + selected_texts.len()
            + selected_properties.len()
            + selection_requests.len()
            + inputs.len();
        last_input_seq = inputs.last().map(|input| input.sequence);
        input_age_us = last_input_seq.and_then(|sequence| {
            let state = lock(shared);
            state
                .shell
                .cad_input_recv
                .iter()
                .rev()
                .find(|(seq, _)| *seq == sequence)
                .map(|(_, received)| received.elapsed().as_micros())
        });
        // The save dialog's self-containment choice (ADR-0040 D3) is applied
        // **before** this batch's commands: the dialog applies the choice and then
        // dispatches SAVE, and both can land in one tick. Draining it after the
        // commands would save with the previous policy and silently ignore the
        // user's answer. Last wins (the field is one scalar), so a double toggle
        // ends at the newest value.
        CAD.with(|cell| {
            cad_desktop_shell::state::drain_embed_resources(shared, cell.borrow().as_ref());
            cad_desktop_shell::state::drain_application_domains(shared, cell.borrow().as_ref());
        });
        let inputs_started = Instant::now();
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    // Settings writes apply first: an Options change in this
                    // batch is visible to the commands/inputs that follow. The
                    // session validates, persists and applies each one (and a
                    // GPU-budget change is installed on the live renderer); a
                    // refusal is its real error (surfaced through cad_status).
                    for (key, value) in setting_writes {
                        apply_setting_write(session, key, value);
                    }
                    // Typed layer writes (the React layer panel's three
                    // mutations): the session resolves each name against the
                    // drawing's own layer table and dispatches the ordinary layer
                    // command; a refusal is the session's real error.
                    layers::apply_writes(session, layer_writes);
                    for command in &commands {
                        // Admission already returned `{accepted:true}`. The real
                        // business outcome is the CadSession snapshot error; it
                        // is not duplicated into the host transient error.
                        if let Err(error) = session.command(command) {
                            eprintln!("[probe] cad command rejected: {error}");
                        }
                    }
                    // Typed standard-view requests: drained in admission order (the
                    // last one wins); a real refusal is the session's own error.
                    for error in drain_standard_views(&mut standard_views, |view| {
                        session.standard_view(view)
                    }) {
                        eprintln!("[probe] cad standard view rejected: {error}");
                    }
                    // Typed orbit deltas (the ViewCube's `cad_orbit`), in admission order.
                    for error in drain_orbits(&mut orbits, |d| session.orbit(d.yaw, d.pitch)) {
                        eprintln!("[probe] cad orbit rejected: {error}");
                    }
                    // Typed tool parameters: applied before this batch's inputs so a
                    // stepper change takes effect in the same batch; a refusal is its
                    // own error.
                    for (parameter, value) in &tool_parameters {
                        if let Err(error) = session.set_tool_parameter(*parameter, *value) {
                            eprintln!("[probe] cad tool parameter rejected: {error}");
                        }
                    }
                    // One render publication for the whole tick: every input is
                    // still admitted/executed in order, but the derived render
                    // state syncs once instead of once per event.
                    if let Err(error) = session.handle_inputs(&inputs) {
                        eprintln!("[probe] cad input rejected: {error}");
                    }
                    // Terminal tool texts (ADR 0019 decision A.1): applied after
                    // this batch's inputs, because the last point of a text tool
                    // may arrive in the same tick and the text is only valid once
                    // the point complement is complete. The text completes the
                    // command on its own (no second confirm); a refusal is named
                    // and surfaces through `cad_status.error`.
                    for text in &tool_texts {
                        if let Err(error) = session.set_tool_text(text) {
                            eprintln!("[probe] cad tool text rejected: {error}");
                        }
                    }
                    // Panel text edits (ADR 0020 amendment B): each one targets
                    // the session's **current** selection, resolved on the owner
                    // thread at apply time (no id travelled). A refusal is named
                    // and surfaces through `cad_status.error`.
                    for text in &selected_texts {
                        if let Err(error) = session.set_selected_text(text) {
                            eprintln!("[probe] cad selected text rejected: {error}");
                        }
                    }
                    // Panel property writes (the docked palette's
                    // `SETENTITYPROPERTY` channel): each pair targets the
                    // session's **current** selection, resolved on the owner
                    // thread at apply time (no id travelled). The value is
                    // validated against the drawing's own tables there, so a
                    // refusal is named and surfaces through `cad_status.error`.
                    for (key, value) in &selected_properties {
                        if let Err(error) = session.set_selected_property(key, value) {
                            eprintln!("[probe] cad selected property rejected: {error}");
                        }
                    }
                    // BIM-panel selection requests (`cad_select_entities`): every
                    // id is validated against the open document and the whole
                    // request is refused on an unknown id, so a stale row can never
                    // half-select. A refusal is named and surfaces through
                    // `cad_status.error`.
                    for ids in &selection_requests {
                        if let Err(error) = session.select_entities(ids) {
                            eprintln!("[probe] cad selection rejected: {error}");
                        }
                    }
                }
            });
        }
        // Plugin-API submissions and prompt answers are drained on this tick (the
        // IPC thread cannot touch the `!Send` session); answers wait for the poll.
        CAD.with(|cell| {
            cad_desktop_shell::state::drain_api_calls(shared, cell.borrow().as_ref());
        });
        // The Options > Files folder chooser publishes one terminal from its
        // worker thread; the tick hands it to the waiting `cad_pick_directory`
        // call. It needs no session (a pick can happen with no document open), so
        // it runs outside the `CAD.with` block.
        cad_desktop_shell::state::drain_directory_pick(shared);

        let inputs_us = inputs_started.elapsed().as_micros();
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        // A tick failure is a host transient error, cleared only by a later
        // successful tick.
        let tick_started = Instant::now();
        let tick = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.tick()));
        let tick_us = tick_started.elapsed().as_micros();
        match tick {
            Some(Ok(())) => clear_host_error(&mut lock(shared).shell, HostErrorKind::Tick),
            Some(Err(error)) => note_host_error(
                &mut lock(shared).shell,
                HostErrorKind::Tick,
                format!("cad tick failed: {error}"),
            ),
            None => {}
        }
        if cad_log && queued > 0 {
            eprintln!("[probe] cad-stage queued={queued} input_us={inputs_us} tick_us={tick_us}");
        }
    }
    (last_input_seq, input_age_us)
}
