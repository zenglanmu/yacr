//! Action-recorder macros (ADR-0056): the versioned external file codec.
//!
//! A macro is **not** document content. It is a small, versioned JSON file the
//! session records and replays through the ordinary command funnel:
//!
//! ```jsonc
//! {
//!   "format": "yacr.action-macro",
//!   "version": 1,
//!   "name": "door-notes",
//!   "steps": [
//!     { "kind": "command", "command": "COLOR", "argument": "3" },
//!     { "kind": "input", "message": "pick the insertion point" },
//!     { "kind": "message", "text": "now place the block" }
//!   ]
//! }
//! ```
//!
//! The codec is pure and strict: it never clamps, drops or repairs a step. An
//! unknown version, format, step kind or field is a named refusal, and a macro
//! that tries to nest a recorder control command (which would recurse) is
//! refused before it can be written or played.
//!
//! The session-side state (recording buffer, saved session macros, playback) is
//! in [`crate::session::action_recorder`]; this module owns only the data model,
//! the stable refusals and the JSON round trip.

use serde::{Deserialize, Serialize};

/// The macro file format tag. A file without this exact tag is refused.
pub const ACTION_MACRO_FORMAT: &str = "yacr.action-macro";

/// The one format version this build writes and reads.
pub const ACTION_MACRO_VERSION: u32 = 1;

/// The stable marker for a runtime with no host file bridge (the browser).
pub const ACTION_RECORDER_UNSUPPORTED: &str = "unsupported-runtime: action-recorder";

/// The maximum number of steps one macro may hold.
pub const ACTION_MACRO_MAX_STEPS: usize = 10_000;

/// The maximum byte length of one command name, argument or message.
pub const ACTION_MACRO_MAX_TEXT: usize = 8_192;

/// One recorded step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MacroStep {
    /// One command: a canonical selector plus the verbatim typed argument, if
    /// the selector took one. The argument keeps its typed case (a path or a
    /// message must never be case-folded); no parser is re-run by the codec.
    Command {
        /// The canonical selector (`COLOR`, `TABLEEXPORT`, `CIRCLE 2P`).
        command: String,
        /// The typed argument exactly as the user wrote it, or `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        argument: Option<String>,
    },
    /// A pause: playback stops and the user continues with `ACTMANAGER CONTINUE`.
    Input {
        /// Optional prompt text recorded with `ACTUSERINPUT <message>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// A user-facing message step (`ACTUSERMESSAGE <text>`).
    Message {
        /// The message text, verbatim.
        text: String,
    },
}

impl MacroStep {
    /// The command-line text this step replays, or `None` for a pause/message.
    pub fn command_line(&self) -> Option<String> {
        match self {
            Self::Command { command, argument } => Some(match argument {
                Some(argument) if !argument.is_empty() => format!("{command} {argument}"),
                _ => command.clone(),
            }),
            Self::Input { .. } | Self::Message { .. } => None,
        }
    }
}

/// One complete action macro, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionMacro {
    /// Always [`ACTION_MACRO_FORMAT`] in a file this build accepts.
    pub format: String,
    /// Always [`ACTION_MACRO_VERSION`] in a file this build accepts.
    pub version: u32,
    /// The macro's display name (file stem by default).
    pub name: String,
    /// The ordered steps.
    pub steps: Vec<MacroStep>,
}

impl ActionMacro {
    /// Build a macro with the current format/version.
    pub fn new(name: impl Into<String>, steps: Vec<MacroStep>) -> Self {
        Self {
            format: ACTION_MACRO_FORMAT.to_string(),
            version: ACTION_MACRO_VERSION,
            name: name.into(),
            steps,
        }
    }
}

/// A strict codec refusal: the stable code plus a human-readable explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroCodecError {
    /// The stable reason string (prefix before the colon is the machine code).
    pub reason: String,
}

impl MacroCodecError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for MacroCodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for MacroCodecError {}

/// The recorder control names. A macro step may never name one: replaying it
/// would re-enter the recorder (ADR-0056 D3's no-recursion rule).
pub const RECORDER_CONTROL_NAMES: &[&str] = &[
    "ACTRECORD",
    "ACTSTOP",
    "-ACTSTOP",
    "ACTUSERINPUT",
    "ACTUSERMESSAGE",
    "-ACTUSERMESSAGE",
    "ACTMANAGER",
];

/// Whether `command` names a recorder control command (compared by its first
/// whitespace-separated token, case-insensitively).
pub fn is_recorder_control_name(command: &str) -> bool {
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    RECORDER_CONTROL_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(first))
}

fn check_text(field: &str, value: &str, allow_empty: bool) -> Result<(), MacroCodecError> {
    if !allow_empty && value.is_empty() {
        return Err(MacroCodecError::new(format!(
            "action_macro.empty_{field}: {field} must not be empty"
        )));
    }
    if value.len() > ACTION_MACRO_MAX_TEXT {
        return Err(MacroCodecError::new(format!(
            "action_macro.{field}_too_long: {field} is {} bytes; the cap is {}",
            value.len(),
            ACTION_MACRO_MAX_TEXT
        )));
    }
    if value.contains('\0') {
        return Err(MacroCodecError::new(format!(
            "action_macro.{field}_invalid: {field} contains a NUL byte"
        )));
    }
    Ok(())
}

/// Validate one decoded macro without writing anything.
pub fn validate_action_macro(macro_file: &ActionMacro) -> Result<(), MacroCodecError> {
    if macro_file.format != ACTION_MACRO_FORMAT {
        return Err(MacroCodecError::new(format!(
            "action_macro.format: expected {:?}, got {:?}",
            ACTION_MACRO_FORMAT, macro_file.format
        )));
    }
    if macro_file.version != ACTION_MACRO_VERSION {
        return Err(MacroCodecError::new(format!(
            "action_macro.unsupported_version: version {} is not supported; this build reads {}",
            macro_file.version, ACTION_MACRO_VERSION
        )));
    }
    check_text("name", &macro_file.name, false)?;
    if macro_file.steps.len() > ACTION_MACRO_MAX_STEPS {
        return Err(MacroCodecError::new(format!(
            "action_macro.too_many_steps: {} steps; the cap is {}",
            macro_file.steps.len(),
            ACTION_MACRO_MAX_STEPS
        )));
    }
    for (index, step) in macro_file.steps.iter().enumerate() {
        let number = index + 1;
        match step {
            MacroStep::Command { command, argument } => {
                check_text("command", command, false).map_err(|error| {
                    MacroCodecError::new(format!("step {number}: {}", error.reason))
                })?;
                if is_recorder_control_name(command) {
                    return Err(MacroCodecError::new(format!(
                        "action_macro.recursion: step {number} names the recorder control \
                         command {command:?}; recorder commands cannot be macro steps"
                    )));
                }
                if let Some(argument) = argument {
                    check_text("argument", argument, true).map_err(|error| {
                        MacroCodecError::new(format!("step {number}: {}", error.reason))
                    })?;
                }
            }
            MacroStep::Input { message } => {
                if let Some(message) = message {
                    check_text("message", message, true).map_err(|error| {
                        MacroCodecError::new(format!("step {number}: {}", error.reason))
                    })?;
                }
            }
            MacroStep::Message { text } => {
                check_text("message", text, false).map_err(|error| {
                    MacroCodecError::new(format!("step {number}: {}", error.reason))
                })?;
            }
        }
    }
    Ok(())
}

/// Encode one macro as pretty JSON after validating it.
pub fn encode_action_macro(macro_file: &ActionMacro) -> Result<String, MacroCodecError> {
    validate_action_macro(macro_file)?;
    serde_json::to_string_pretty(macro_file).map_err(|error| {
        MacroCodecError::new(format!(
            "action_macro.encode: the macro cannot be serialized: {error}"
        ))
    })
}

/// Decode and validate one macro file.
///
/// The decoder is the authority for the format/version tags and the step shape;
/// a syntactically valid JSON with an unknown field is refused rather than
/// silently narrowed, so a future writer cannot lose data through this build.
pub fn decode_action_macro(text: &str) -> Result<ActionMacro, MacroCodecError> {
    let macro_file: ActionMacro = serde_json::from_str(text).map_err(|error| {
        MacroCodecError::new(format!(
            "action_macro.invalid_json: the macro file is not a supported action macro: {error}"
        ))
    })?;
    validate_action_macro(&macro_file)?;
    Ok(macro_file)
}

/// What the recorder is doing right now (the host-read answer).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRecorderSnapshot {
    /// The live recording buffer, or `null` when not recording.
    pub recording: Option<ActionRecordingSnapshot>,
    /// Macros saved (or loaded) in this session, in save order.
    pub macros: Vec<ActionMacroSummary>,
    /// The name of the last macro this session saved, or `null`.
    pub last_macro: Option<String>,
    /// A playback paused on an input step, or `null` when none is running.
    pub playback: Option<ActionPlaybackSnapshot>,
    /// The last recorder refusal, or `null` after a successful recorder command.
    ///
    /// The desktop queues `cad_command`, so its immediate IPC answer only means
    /// "admitted"; this field is how the panel reads the real `act*` refusal.
    pub last_error: Option<String>,
}

/// The live recording buffer as the panel sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRecordingSnapshot {
    /// The pending macro name.
    pub name: String,
    /// The pending destination path, or `null` when the user has not typed one.
    pub path: Option<String>,
    /// How many steps have been captured so far.
    pub steps: usize,
}

/// A paused playback as the panel sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPlaybackSnapshot {
    /// The macro being played.
    pub name: String,
    /// The one-based number of the next step (the pause is *before* it).
    pub next_step: usize,
    /// The macro's total step count.
    pub total: usize,
}

/// One session-visible macro row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMacroSummary {
    /// The macro name.
    pub name: String,
    /// The path it was saved to (or loaded from), or `null`.
    pub path: Option<String>,
    /// The number of steps.
    pub steps: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ActionMacro {
        ActionMacro::new(
            "door-notes",
            vec![
                MacroStep::Command {
                    command: "COLOR".into(),
                    argument: Some("3".into()),
                },
                MacroStep::Input {
                    message: Some("pick the insertion point".into()),
                },
                MacroStep::Message {
                    text: "now place the block".into(),
                },
                MacroStep::Command {
                    command: "TABLEEXPORT".into(),
                    argument: Some("C:/Data/Mixed Case.csv".into()),
                },
            ],
        )
    }

    #[test]
    fn a_macro_round_trips_and_preserves_typed_case() {
        let encoded = encode_action_macro(&sample()).expect("encode");
        let decoded = decode_action_macro(&encoded).expect("decode");
        assert_eq!(decoded, sample());
        assert!(
            encoded.contains("C:/Data/Mixed Case.csv"),
            "the typed path must keep its case: {encoded}"
        );
    }

    #[test]
    fn the_wrong_format_or_version_is_refused() {
        let mut wrong_format = sample();
        wrong_format.format = "something-else".into();
        assert!(encode_action_macro(&wrong_format)
            .unwrap_err()
            .reason
            .starts_with("action_macro.format:"));
        let mut wrong_version = sample();
        wrong_version.version = 2;
        assert!(encode_action_macro(&wrong_version)
            .unwrap_err()
            .reason
            .starts_with("action_macro.unsupported_version:"));
    }

    #[test]
    fn an_unknown_field_or_step_kind_is_refused() {
        let badly_typed = r#"{
            "format": "yacr.action-macro",
            "version": 1,
            "name": "m",
            "steps": [{ "kind": "shell", "command": "rm -rf /" }]
        }"#;
        assert!(decode_action_macro(badly_typed)
            .unwrap_err()
            .reason
            .starts_with("action_macro.invalid_json:"));
        let extra_field = r#"{
            "format": "yacr.action-macro",
            "version": 1,
            "name": "m",
            "steps": [{ "kind": "command", "command": "COLOR", "script": "x" }]
        }"#;
        assert!(decode_action_macro(extra_field)
            .unwrap_err()
            .reason
            .starts_with("action_macro.invalid_json:"));
    }

    #[test]
    fn a_control_command_step_is_refused_as_recursion() {
        let mut macro_file = sample();
        macro_file.steps.push(MacroStep::Command {
            command: "ACTMANAGER".into(),
            argument: Some("PLAY m".into()),
        });
        let error = encode_action_macro(&macro_file).unwrap_err();
        assert!(
            error.reason.starts_with("action_macro.recursion:"),
            "{error}"
        );
        assert!(is_recorder_control_name("actrecord"));
        assert!(is_recorder_control_name("-ACTSTOP text"));
        assert!(!is_recorder_control_name("COLOR"));
    }

    #[test]
    fn empty_names_messages_and_oversized_buffers_are_refused() {
        let mut empty_name = sample();
        empty_name.name.clear();
        assert!(encode_action_macro(&empty_name)
            .unwrap_err()
            .reason
            .starts_with("action_macro.empty_name:"));

        let mut empty_message = sample();
        empty_message.steps.push(MacroStep::Message {
            text: String::new(),
        });
        assert!(encode_action_macro(&empty_message)
            .unwrap_err()
            .reason
            .contains("empty_message"));

        let mut too_long = sample();
        too_long.steps.push(MacroStep::Command {
            command: "COLOR".into(),
            argument: Some("x".repeat(ACTION_MACRO_MAX_TEXT + 1)),
        });
        assert!(encode_action_macro(&too_long)
            .unwrap_err()
            .reason
            .contains("argument_too_long"));
    }

    #[test]
    fn command_line_reconstruction_keeps_the_argument_verbatim() {
        let step = MacroStep::Command {
            command: "TABLEEXPORT".into(),
            argument: Some("a b/c.CSV".into()),
        };
        assert_eq!(
            step.command_line().as_deref(),
            Some("TABLEEXPORT a b/c.CSV")
        );
        let bare = MacroStep::Command {
            command: "STATUS".into(),
            argument: None,
        };
        assert_eq!(bare.command_line().as_deref(), Some("STATUS"));
        assert!(MacroStep::Input { message: None }.command_line().is_none());
    }
}
