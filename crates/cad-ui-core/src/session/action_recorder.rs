//! The session half of the action recorder (ADR-0056).
//!
//! `ACTRECORD` starts a recording buffer, every **successfully executed** typed
//! command is appended by [`CadSession::command`]'s wrapper, and `ACTSTOP`
//! writes the buffer through the host's named-file bridge. `ACTMANAGER` lists,
//! plays and deletes the session-visible macros.
//!
//! # Boundaries
//!
//! * The recorder lives in the session because that is where the typed command
//!   line is parsed and where the file strategy is reachable. It never writes
//!   `cad-db` directly: playback re-enters [`CadSession::command`], so every
//!   replayed step keeps the ordinary authorization, transaction, revision and
//!   undo boundaries.
//! * A runtime with no named-file bridge (the browser/wasm carrier) refuses
//!   start/stop/list/play with [`ACTION_RECORDER_UNSUPPORTED`]; it never
//!   fabricates an in-memory macro that cannot be saved.
//! * Recorder control commands are never recorded and cannot appear in a macro
//!   file, so playback cannot recurse.
//! * A playback refusal names the one-based step and stops. Already executed
//!   steps are not rolled back: the user's ordinary `UNDO` owns that history.

use std::path::Path;

use cad_platform::NamedFileError;

use crate::action_recorder::{
    decode_action_macro, encode_action_macro, ActionMacro, ActionMacroSummary,
    ActionPlaybackSnapshot, ActionRecorderSnapshot, ActionRecordingSnapshot, MacroStep,
    ACTION_MACRO_MAX_STEPS, ACTION_RECORDER_UNSUPPORTED, RECORDER_CONTROL_NAMES,
};

use super::CadSession;

/// `ACTRECORD [<name-or-path>]`.
pub const ACTRECORD_SELECTOR: &str = "ACTRECORD";
/// `ACTSTOP [<path.json>]`.
pub const ACTSTOP_SELECTOR: &str = "ACTSTOP";
/// `-ACTSTOP [<path.json>]`.
pub const DASH_ACTSTOP_SELECTOR: &str = "-ACTSTOP";
/// `ACTUSERINPUT [<message>]`.
pub const ACTUSERINPUT_SELECTOR: &str = "ACTUSERINPUT";
/// `ACTUSERMESSAGE <text>`.
pub const ACTUSERMESSAGE_SELECTOR: &str = "ACTUSERMESSAGE";
/// `-ACTUSERMESSAGE <text>`.
pub const DASH_ACTUSERMESSAGE_SELECTOR: &str = "-ACTUSERMESSAGE";
/// `ACTMANAGER [LIST|PLAY <name>|DELETE <name>|CONTINUE]`.
pub const ACTMANAGER_SELECTOR: &str = "ACTMANAGER";

/// The default macro name when `ACTRECORD` has no argument.
const DEFAULT_MACRO_NAME: &str = "macro";

/// The live recording buffer.
#[derive(Debug, Clone, Default)]
pub(crate) struct Recording {
    pub name: String,
    /// Whether the user typed the name (and it must win over a later STOP path).
    pub name_explicit: bool,
    pub path: Option<String>,
    pub steps: Vec<MacroStep>,
    /// Set once the step cap is reached: further successful commands are not
    /// captured, and the status says so on stop. The recording is not aborted,
    /// because the commands already ran.
    pub truncated: bool,
}

/// One macro saved (or loaded) in this session.
#[derive(Debug, Clone)]
pub(crate) struct SavedMacro {
    pub name: String,
    pub path: Option<String>,
    pub steps: Vec<MacroStep>,
}

/// A playback that is paused on an input step, or between steps.
#[derive(Debug, Clone)]
pub(crate) struct Playback {
    pub name: String,
    pub steps: Vec<MacroStep>,
    /// The index of the next step to run.
    pub next: usize,
}

/// The session's whole recorder state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ActionRecorderState {
    pub recording: Option<Recording>,
    pub macros: Vec<SavedMacro>,
    pub last_macro: Option<String>,
    pub playback: Option<Playback>,
    /// The last recorder refusal (published in the host-read snapshot).
    pub last_error: Option<String>,
}

impl ActionRecorderState {
    fn snapshot(&self) -> ActionRecorderSnapshot {
        ActionRecorderSnapshot {
            recording: self
                .recording
                .as_ref()
                .map(|recording| ActionRecordingSnapshot {
                    name: recording.name.clone(),
                    path: recording.path.clone(),
                    steps: recording.steps.len(),
                }),
            macros: self
                .macros
                .iter()
                .map(|saved| ActionMacroSummary {
                    name: saved.name.clone(),
                    path: saved.path.clone(),
                    steps: saved.steps.len(),
                })
                .collect(),
            last_macro: self.last_macro.clone(),
            playback: self
                .playback
                .as_ref()
                .map(|playback| ActionPlaybackSnapshot {
                    name: playback.name.clone(),
                    next_step: playback.next + 1,
                    total: playback.steps.len(),
                }),
            last_error: self.last_error.clone(),
        }
    }

    fn upsert(&mut self, saved: SavedMacro) {
        if let Some(existing) = self.macros.iter_mut().find(|row| row.name == saved.name) {
            *existing = saved;
        } else {
            self.macros.push(saved);
        }
    }
}

/// Split one typed line into its recorder selector and verbatim argument.
///
/// The match is on the **first whitespace-separated token**, case-insensitively,
/// so `actrecord my path.json` is the same command as `ACTRECORD my path.json`.
fn split_control(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    let split = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (head, tail) = trimmed.split_at(split);
    let selector = RECORDER_CONTROL_NAMES
        .iter()
        .find(|name| name.eq_ignore_ascii_case(head))?;
    Some((selector, tail.trim_start()))
}

/// Whether typed text is one of the recorder control commands.
pub(crate) fn is_action_recorder_command(text: &str) -> bool {
    split_control(text).is_some()
}

fn path_like(argument: &str) -> bool {
    argument.contains('/')
        || argument.contains('\\')
        || argument.starts_with('.')
        || argument.ends_with(".json")
        || argument.ends_with(".JSON")
}

/// The display name for a typed path or name.
fn macro_name(argument: &str) -> String {
    if !path_like(argument) {
        return argument.to_string();
    }
    let base = argument
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(argument)
        .trim();
    let without_json = base
        .strip_suffix(".json")
        .or_else(|| base.strip_suffix(".JSON"))
        .unwrap_or(base);
    let without_actm = without_json
        .strip_suffix(".actm")
        .or_else(|| without_json.strip_suffix(".ACTM"))
        .unwrap_or(without_json);
    if without_actm.is_empty() {
        DEFAULT_MACRO_NAME.to_string()
    } else {
        without_actm.to_string()
    }
}

fn valid_destination(argument: &str) -> bool {
    let path = argument.trim();
    if path.is_empty() || path.contains('\0') {
        return false;
    }
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn write_reason(path: &str, error: &NamedFileError) -> String {
    if matches!(error, NamedFileError::Unavailable) {
        return ACTION_RECORDER_UNSUPPORTED.to_string();
    }
    format!("actstop.write_failed: {path}: {error}")
}

impl CadSession {
    /// Dispatch one typed command line through the ordinary funnel, capturing
    /// the successful steps of an active recording (ADR-0056).
    pub fn command(&self, text: &str) -> Result<(), String> {
        // Recorder controls are session commands and are never captured; the
        // wrapper is the one place a successful non-control line becomes a step.
        if let Some(outcome) = self.action_recorder_command(text) {
            return outcome;
        }
        let outcome = self.command_inner(text);
        if outcome.is_ok() {
            self.action_recorder_note_success(text);
        }
        outcome
    }

    /// Whether this carrier can persist macros (a real named-file bridge).
    pub fn action_recorder_supported(&self) -> bool {
        self.core.resources.supports_named_files()
    }

    /// The current recorder state for the Manage ▸ Action Recorder panel.
    ///
    /// A carrier without a file bridge refuses with
    /// [`ACTION_RECORDER_UNSUPPORTED`] instead of answering an empty list that
    /// would read as "this session has no macros".
    pub fn action_recorder(&self) -> Result<ActionRecorderSnapshot, String> {
        if !self.action_recorder_supported() {
            return Err(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        Ok(self.action_recorder.borrow().snapshot())
    }

    /// Dispatch one recorder control command, or `None` when the text is not one.
    pub(crate) fn action_recorder_command(&self, text: &str) -> Option<Result<(), String>> {
        let (selector, argument) = split_control(text)?;
        Some(match selector {
            ACTRECORD_SELECTOR => self.action_record_start(argument),
            ACTSTOP_SELECTOR | DASH_ACTSTOP_SELECTOR => self.action_record_stop(argument),
            ACTUSERINPUT_SELECTOR => self.action_record_input(argument),
            ACTUSERMESSAGE_SELECTOR | DASH_ACTUSERMESSAGE_SELECTOR => {
                self.action_record_message(argument)
            }
            ACTMANAGER_SELECTOR => self.action_manager(argument),
            _ => Ok(()),
        })
    }

    /// Append one successfully executed command to the live recording.
    pub(crate) fn action_recorder_note_success(&self, text: &str) {
        if is_action_recorder_command(text) {
            return;
        }
        let Some((command, argument)) = recorded_command(text) else {
            return;
        };
        let mut state = self.action_recorder.borrow_mut();
        let Some(recording) = state.recording.as_mut() else {
            return;
        };
        if recording.steps.len() >= ACTION_MACRO_MAX_STEPS {
            recording.truncated = true;
            return;
        }
        recording.steps.push(MacroStep::Command {
            command: command.to_string(),
            argument,
        });
    }

    /// Record a stable recorder refusal in the session error *and* the panel's
    /// own `lastError`, then return it.
    fn action_refuse(&self, reason: String) -> Result<(), String> {
        // A few call sites still hold the state borrow while they decide the
        // refusal; never panic on that (the session error is the authority).
        if let Ok(mut state) = self.action_recorder.try_borrow_mut() {
            state.last_error = Some(reason.clone());
        }
        self.refuse_argument(reason)
    }

    fn action_record_start(&self, argument: &str) -> Result<(), String> {
        if !self.action_recorder_supported() {
            return self.action_refuse(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        {
            let state = self.action_recorder.borrow();
            if state.recording.is_some() {
                return self.action_refuse(
                    "actrecord.already_recording: a recording is already active; stop it with \
                     ACTSTOP first"
                        .to_string(),
                );
            }
            if state.playback.is_some() {
                return self.action_refuse(
                    "actrecord.playback_active: finish or stop the playback before recording"
                        .to_string(),
                );
            }
        }
        let argument = argument.trim();
        if argument.contains('\0') {
            return self.action_refuse(
                "actrecord.name_invalid: the macro name or path contains a NUL byte".to_string(),
            );
        }
        let name = if argument.is_empty() {
            DEFAULT_MACRO_NAME.to_string()
        } else {
            macro_name(argument)
        };
        let path = (path_like(argument) && !argument.is_empty()).then(|| argument.to_string());
        self.action_recorder.borrow_mut().recording = Some(Recording {
            name: name.clone(),
            name_explicit: !argument.is_empty() && !path_like(argument),
            path: path.clone(),
            steps: Vec::new(),
            truncated: false,
        });
        self.action_recorder.borrow_mut().last_error = None;
        self.core.record_command(ACTRECORD_SELECTOR);
        *self.error.borrow_mut() = None;
        let destination = path
            .as_deref()
            .map(|path| format!("; save with ACTSTOP (pending path {path})"))
            .unwrap_or_else(|| "; save with ACTSTOP <path.json>".to_string());
        let _ = self
            .core
            .set_external_status(format!("ACTRECORD: recording {name:?}{destination}"));
        Ok(())
    }

    fn action_record_stop(&self, argument: &str) -> Result<(), String> {
        if !self.action_recorder_supported() {
            return self.action_refuse(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        let (mut name, name_explicit, pending_path, steps, truncated) = {
            let state = self.action_recorder.borrow();
            let Some(recording) = state.recording.as_ref() else {
                return self.action_refuse(
                    "actstop.not_recording: no action recording is active; start one with \
                     ACTRECORD"
                        .to_string(),
                );
            };
            (
                recording.name.clone(),
                recording.name_explicit,
                recording.path.clone(),
                recording.steps.clone(),
                recording.truncated,
            )
        };
        let argument = argument.trim();
        let path = if argument.is_empty() {
            pending_path
        } else {
            Some(argument.to_string())
        };
        if !name_explicit {
            if let Some(path) = path.as_deref() {
                name = macro_name(path);
            }
        }
        let Some(path) = path else {
            return self.action_refuse(
                "actstop.path: ACTSTOP needs a destination ending in `.json`; type `ACTSTOP \
                 <path.json>` (no file dialog exists in this build)"
                    .to_string(),
            );
        };
        if !valid_destination(&path) {
            return self.action_refuse(format!(
                "actstop.path: {path:?} has no `.json` destination; type `ACTSTOP <path.json>`"
            ));
        }
        let macro_file = ActionMacro::new(name.clone(), steps.clone());
        let encoded = match encode_action_macro(&macro_file) {
            Ok(encoded) => encoded,
            Err(error) => return self.action_refuse(error.reason),
        };
        if let Err(error) = self
            .core
            .resources
            .write_named_file(Path::new(&path), encoded.as_bytes())
        {
            return self.action_refuse(write_reason(&path, &error));
        }
        {
            let mut state = self.action_recorder.borrow_mut();
            state.upsert(SavedMacro {
                name: name.clone(),
                path: Some(path.clone()),
                steps,
            });
            state.last_macro = Some(name.clone());
            state.recording = None;
        }
        self.action_recorder.borrow_mut().last_error = None;
        self.core.record_command(ACTSTOP_SELECTOR);
        *self.error.borrow_mut() = None;
        let suffix = if truncated {
            "; the step cap was reached, later commands were not captured"
        } else {
            ""
        };
        let _ = self.core.set_external_status(format!(
            "ACTSTOP: wrote {} bytes to {path} ({} step(s){suffix})",
            encoded.len(),
            macro_file.steps.len()
        ));
        Ok(())
    }

    fn action_record_input(&self, argument: &str) -> Result<(), String> {
        if !self.action_recorder_supported() {
            return self.action_refuse(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        let message = argument.trim();
        let mut state = self.action_recorder.borrow_mut();
        let Some(recording) = state.recording.as_mut() else {
            return self.action_refuse(
                "actuserinput.not_recording: ACTUSERINPUT records a pause only while ACTRECORD \
                 is active"
                    .to_string(),
            );
        };
        if recording.steps.len() >= ACTION_MACRO_MAX_STEPS {
            recording.truncated = true;
            return self.action_refuse(format!(
                "actuserinput.too_many_steps: the macro already holds {} steps",
                recording.steps.len()
            ));
        }
        recording.steps.push(MacroStep::Input {
            message: (!message.is_empty()).then(|| message.to_string()),
        });
        drop(state);
        self.action_recorder.borrow_mut().last_error = None;
        self.core.record_command(ACTUSERINPUT_SELECTOR);
        *self.error.borrow_mut() = None;
        let _ = self
            .core
            .set_external_status("ACTUSERINPUT: recorded an input pause".to_string());
        Ok(())
    }

    fn action_record_message(&self, argument: &str) -> Result<(), String> {
        if !self.action_recorder_supported() {
            return self.action_refuse(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        let text = argument.trim();
        if text.is_empty() {
            return self.action_refuse(
                "actusermessage.message: ACTUSERMESSAGE needs a non-empty message".to_string(),
            );
        }
        let mut state = self.action_recorder.borrow_mut();
        let Some(recording) = state.recording.as_mut() else {
            return self.action_refuse(
                "actusermessage.not_recording: ACTUSERMESSAGE records a message only while \
                 ACTRECORD is active"
                    .to_string(),
            );
        };
        if recording.steps.len() >= ACTION_MACRO_MAX_STEPS {
            recording.truncated = true;
            return self.action_refuse(format!(
                "actusermessage.too_many_steps: the macro already holds {} steps",
                recording.steps.len()
            ));
        }
        recording.steps.push(MacroStep::Message {
            text: text.to_string(),
        });
        drop(state);
        self.action_recorder.borrow_mut().last_error = None;
        self.core.record_command(ACTUSERMESSAGE_SELECTOR);
        *self.error.borrow_mut() = None;
        let _ = self
            .core
            .set_external_status("ACTUSERMESSAGE: recorded a message step".to_string());
        Ok(())
    }

    fn action_manager(&self, argument: &str) -> Result<(), String> {
        if !self.action_recorder_supported() {
            return self.action_refuse(ACTION_RECORDER_UNSUPPORTED.to_string());
        }
        let argument = argument.trim();
        let (option, tail) = match argument.find(char::is_whitespace) {
            Some(index) => (&argument[..index], argument[index..].trim()),
            None => (argument, ""),
        };
        let option = option.to_ascii_uppercase();
        match option.as_str() {
            "" | "LIST" | "?" => self.action_manager_list(),
            "PLAY" => self.action_manager_play(tail),
            "CONTINUE" => self.action_manager_continue(),
            "DELETE" => self.action_manager_delete(tail),
            _ => self.action_refuse(format!(
                "actmanager.option: unknown ACTMANAGER option {option:?}; use LIST, PLAY <name>, \
                 DELETE <name> or CONTINUE"
            )),
        }
    }

    fn action_manager_list(&self) -> Result<(), String> {
        self.action_recorder.borrow_mut().last_error = None;
        let state = self.action_recorder.borrow();
        self.core.record_command(ACTMANAGER_SELECTOR);
        *self.error.borrow_mut() = None;
        let line = if state.macros.is_empty() {
            "ACTMANAGER: no action macros are visible in this session".to_string()
        } else {
            let mut line = format!("ACTMANAGER: {} macro(s):", state.macros.len());
            for saved in &state.macros {
                line.push_str(&format!(
                    " {} ({} step(s){})",
                    saved.name,
                    saved.steps.len(),
                    saved
                        .path
                        .as_deref()
                        .map(|path| format!(", {path}"))
                        .unwrap_or_default()
                ));
            }
            line
        };
        drop(state);
        let _ = self.core.set_external_status(line);
        Ok(())
    }

    fn action_manager_play(&self, tail: &str) -> Result<(), String> {
        let target = tail.trim();
        if target.is_empty() {
            return self.action_refuse(
                "actmanager.play: PLAY needs a macro name (or a `.json` path)".to_string(),
            );
        }
        {
            let state = self.action_recorder.borrow();
            if state.recording.is_some() {
                return self.action_refuse(
                    "actmanager.recording: stop the recording with ACTSTOP before playing a macro"
                        .to_string(),
                );
            }
            if state.playback.is_some() {
                return self.action_refuse(
                    "actmanager.playback_active: a playback is already paused; use ACTMANAGER \
                     CONTINUE"
                        .to_string(),
                );
            }
        }
        let state = self.action_recorder.borrow();
        let found = state
            .macros
            .iter()
            .find(|saved| saved.name == target || saved.path.as_deref() == Some(target))
            .cloned();
        drop(state);
        let saved = match found {
            Some(saved) => saved,
            None if path_like(target) => self.action_manager_load(target)?,
            None => {
                return self.action_refuse(format!(
                    "actmanager.unknown_macro: no session macro named {target:?}; run ACTMANAGER \
                     LIST"
                ))
            }
        };
        {
            let mut state = self.action_recorder.borrow_mut();
            state.playback = Some(Playback {
                name: saved.name.clone(),
                steps: saved.steps.clone(),
                next: 0,
            });
            state.last_error = None;
        }
        self.run_playback()
    }

    fn action_manager_load(&self, path: &str) -> Result<SavedMacro, String> {
        let bytes = self
            .core
            .resources
            .read_named_file(Path::new(path))
            .map_err(|error| match error {
                NamedFileError::Unavailable => ACTION_RECORDER_UNSUPPORTED.to_string(),
                other => format!("actmanager.load_failed: {path}: {other}"),
            })?;
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|error| format!("action_macro.invalid_json: {path} is not UTF-8: {error}"))?;
        let macro_file = decode_action_macro(&text).map_err(|error| error.reason)?;
        let saved = SavedMacro {
            name: macro_file.name.clone(),
            path: Some(path.to_string()),
            steps: macro_file.steps,
        };
        self.action_recorder.borrow_mut().upsert(saved.clone());
        Ok(saved)
    }

    fn action_manager_delete(&self, tail: &str) -> Result<(), String> {
        let target = tail.trim();
        if target.is_empty() {
            return self.action_refuse(
                "actmanager.delete: DELETE needs a macro name (or a `.json` path)".to_string(),
            );
        }
        let mut state = self.action_recorder.borrow_mut();
        let before = state.macros.len();
        state
            .macros
            .retain(|saved| saved.name != target && saved.path.as_deref() != Some(target));
        if state.macros.len() == before {
            return self.action_refuse(format!(
                "actmanager.unknown_macro: no session macro named {target:?}; nothing was deleted"
            ));
        }
        if state.last_macro.as_deref() == Some(target) {
            state.last_macro = None;
        }
        drop(state);
        self.action_recorder.borrow_mut().last_error = None;
        self.core.record_command(ACTMANAGER_SELECTOR);
        *self.error.borrow_mut() = None;
        let _ = self.core.set_external_status(format!(
            "ACTMANAGER: deleted the session entry {target:?}; the macro file on disk was not \
             touched"
        ));
        Ok(())
    }

    fn action_manager_continue(&self) -> Result<(), String> {
        if self.action_recorder.borrow().playback.is_none() {
            return self.action_refuse(
                "actmanager.no_playback: no playback is paused; start one with ACTMANAGER PLAY \
                 <name>"
                    .to_string(),
            );
        }
        self.action_recorder.borrow_mut().last_error = None;
        self.run_playback()
    }

    /// Run the playback queue until it completes or pauses on an input step.
    ///
    /// Each command is re-entered through [`CadSession::command`], so it keeps
    /// the ordinary authorization/transaction/undo path. The one-based step
    /// number is reported on every refusal; the playback then stops (already
    /// executed steps are not undone).
    fn run_playback(&self) -> Result<(), String> {
        loop {
            let (index, step) = {
                let state = self.action_recorder.borrow();
                let Some(playback) = state.playback.as_ref() else {
                    return Ok(());
                };
                if playback.next >= playback.steps.len() {
                    let name = playback.name.clone();
                    let total = playback.steps.len();
                    drop(state);
                    self.action_recorder.borrow_mut().playback = None;
                    self.action_recorder.borrow_mut().last_error = None;
                    self.core.record_command(ACTMANAGER_SELECTOR);
                    *self.error.borrow_mut() = None;
                    let _ = self.core.set_external_status(format!(
                        "ACTMANAGER: played {total} step(s) of {name:?}"
                    ));
                    return Ok(());
                }
                (playback.next, playback.steps[playback.next].clone())
            };
            match step {
                MacroStep::Input { message } => {
                    {
                        let mut state = self.action_recorder.borrow_mut();
                        if let Some(playback) = state.playback.as_mut() {
                            playback.next = index + 1;
                        }
                    }
                    self.action_recorder.borrow_mut().last_error = None;
                    self.core.record_command(ACTMANAGER_SELECTOR);
                    *self.error.borrow_mut() = None;
                    let prompt = message
                        .map(|message| format!(": {message}"))
                        .unwrap_or_default();
                    let _ = self.core.set_external_status(format!(
                        "ACTMANAGER: paused before step {}{prompt}; run ACTMANAGER CONTINUE when \
                         ready",
                        index + 1
                    ));
                    return Ok(());
                }
                MacroStep::Message { text } => {
                    {
                        let mut state = self.action_recorder.borrow_mut();
                        if let Some(playback) = state.playback.as_mut() {
                            playback.next = index + 1;
                        }
                    }
                    let _ = self
                        .core
                        .set_external_status(format!("ACTMANAGER message: {text}"));
                }
                MacroStep::Command { command, argument } => {
                    {
                        let mut state = self.action_recorder.borrow_mut();
                        if let Some(playback) = state.playback.as_mut() {
                            playback.next = index + 1;
                        }
                    }
                    let step = MacroStep::Command { command, argument };
                    let line = step.command_line().expect("a command step has a line");
                    if let Err(reason) = self.command(&line) {
                        self.action_recorder.borrow_mut().playback = None;
                        return Err(format!(
                            "actmanager.step_failed: step {} `{line}`: {reason}",
                            index + 1
                        ));
                    }
                }
            }
        }
    }
}

/// The recorded `(canonical selector, verbatim argument)` for one successful
/// typed line, or `None` when the text is not a static command.
///
/// A multi-word canonical selector (`CIRCLE 2P`, `AREA OBJECT`) is recorded as
/// its whole name with no argument; a typed-argument selector keeps its base
/// selector and the tail exactly as typed (so a path is never case-folded).
fn recorded_command(text: &str) -> Option<(&'static str, Option<String>)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(name) = super::commands::canonical_command_name(trimmed) {
        return Some((name, None));
    }
    let (selector, argument) = super::commands::typed_command_argument(trimmed)?;
    Some((selector, Some(argument.to_string())))
}
