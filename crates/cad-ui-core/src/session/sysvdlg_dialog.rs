//! The typed `SYSVDLG` line: read one variable, apply values, save/restore a
//! session-variable file.
//!
//! The bare `SYSVDLG` stays the application's read-only report. This module owns
//! the three forms the dark command line can express without a dialog:
//!
//! * `SYSVDLG <name>` — one modelled value, dispatched as
//!   [`cad_app::SysVarDialogCommand::Read`];
//! * `SYSVDLG <name> <value>` — one edit, dispatched as a one-entry
//!   [`cad_app::SysVarDialogCommand::Apply`]. The application re-validates and
//!   writes through the existing setter for the variable;
//! * `SYSVDLG SAVE <file.json>` / `SYSVDLG RESTORE <file.json>` — a versioned
//!   session-settings file. The host's named-file bridge owns the bytes
//!   (`cad_platform::write_named_file`, exactly like the action recorder); the
//!   values come from the **one** reader in `cad-app`, and a restore dispatches
//!   one validate-then-apply command so the header values land in one revision.
//!
//! # The file is not AutoCAD `.svf`
//!
//! The persisted format is a small tagged JSON document
//! (`{"format":"yacr.sysvar","version":1,"variables":{…}}`), because this build
//! has no `.svf` codec. Saving to a non-`.json` target is refused rather than
//! writing a file whose name suggests a format it cannot read. Assigned values
//! with no stored value at save time (for example `CVPORT` in model space) are
//! omitted; a restore leaves those values alone instead of inventing a clear.
//!
//! The interactive dialog itself is not implemented (a global UI decision); the
//! editable domain is the `cad-app` closed set and every name outside it is
//! refused by name, never skipped.

use std::collections::BTreeMap;
use std::path::Path;

use cad_app::{
    modelled_system_variable, AppMode, CommandId, ExtendedCommandId, ModelledValue,
    SysVarAssignment, SysVarDialogCommand, MODELLED_SYSTEM_VARIABLES,
};
use cad_platform::NamedFileError;
use serde::{Deserialize, Serialize};

use super::commands::split_quoted_tokens;
use super::CadSession;

/// `SYSVDLG <name>` / `<name> <value>` / `SAVE|RESTORE <file.json>`: the
/// command-line forms of the system-variable dialog selector.
pub const SYSVDLG_SELECTOR: &str = "SYSVDLG";

/// The tag carried by a session-variable settings file.
const SYSVAR_FILE_FORMAT: &str = "yacr.sysvar";
/// The only accepted settings-file version.
const SYSVAR_FILE_VERSION: u32 = 1;

/// One parsed `SYSVDLG` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SysVarDialogArgument {
    Read(String),
    Apply(Vec<SysVarAssignment>),
    Save(String),
    Restore(String),
}

impl CadSession {
    /// Route the typed `SYSVDLG <argument>` line.
    pub(super) fn sysvdlg_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let parsed = match parse_sysvdlg_argument(argument) {
            Some(parsed) => parsed,
            None => {
                return self.refuse_argument(format!(
                    "sysvdlg.invalid_argument: '{text}' needs `<name>`, `<name> <value>`, \
                     `SAVE <file.json>` or `RESTORE <file.json>`"
                ))
            }
        };
        match parsed {
            SysVarDialogArgument::Read(name) => self.dispatch_typed_argument(
                SYSVDLG_SELECTOR,
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                cad_app::CommandPayload::SysVarDialog(SysVarDialogCommand::Read(name)),
            ),
            SysVarDialogArgument::Apply(assignments) => self.dispatch_typed_argument(
                SYSVDLG_SELECTOR,
                CommandId::Extended(ExtendedCommandId::SysVarDialog),
                cad_app::CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(assignments)),
            ),
            SysVarDialogArgument::Save(path) => self.sysvdlg_save(&path, text),
            SysVarDialogArgument::Restore(path) => self.sysvdlg_restore(&path, text),
        }
    }

    /// `SYSVDLG SAVE <file.json>`: write every modelled value that has one.
    fn sysvdlg_save(&self, path: &str, text: &str) -> Result<(), String> {
        self.sysvdlg_write_allowed("save")?;
        let path = validated_json_path(path, text, "save")?;
        let controller = self.core.controller.borrow();
        let Some(drawing) = controller.drawing() else {
            return self.refuse_argument(
                "sysvdlg.no_document: SYSVDLG SAVE needs an open document".to_string(),
            );
        };
        let mut variables = BTreeMap::new();
        for name in MODELLED_SYSTEM_VARIABLES {
            match modelled_system_variable(drawing.as_ref(), &controller.session, name) {
                Some(ModelledValue::Value(value, _kind)) => {
                    variables.insert(name.to_string(), value);
                }
                // No stored value: omit it. A restore never invents a clear.
                Some(ModelledValue::Absent(_)) | None => {}
            }
        }
        drop(controller);
        let file = SysVarFile {
            format: SYSVAR_FILE_FORMAT.to_string(),
            version: SYSVAR_FILE_VERSION,
            variables,
        };
        let encoded = serde_json::to_string_pretty(&file)
            .map_err(|error| format!("sysvdlg.save_failed: {error}"))?;
        self.core
            .resources
            .write_named_file(Path::new(&path), encoded.as_bytes())
            .map_err(|error| save_error(&path, &error))?;
        self.core.record_command(SYSVDLG_SELECTOR);
        *self.error.borrow_mut() = None;
        Ok(())
    }

    /// `SYSVDLG RESTORE <file.json>`: validate the whole file, then apply it.
    fn sysvdlg_restore(&self, path: &str, text: &str) -> Result<(), String> {
        self.sysvdlg_write_allowed("restore")?;
        let path = validated_json_path(path, text, "restore")?;
        let bytes = self
            .core
            .resources
            .read_named_file(Path::new(&path))
            .map_err(|error| restore_error(&path, &error))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("sysvdlg.invalid_file: {path} is not UTF-8 JSON"))?;
        let file: SysVarFile = serde_json::from_str(text)
            .map_err(|error| format!("sysvdlg.invalid_file: {path}: {error}"))?;
        if file.format != SYSVAR_FILE_FORMAT {
            return self.refuse_argument(format!(
                "sysvdlg.invalid_file: {path} is not a yacr session-variable file (format {:?})",
                file.format
            ));
        }
        if file.version != SYSVAR_FILE_VERSION {
            return self.refuse_argument(format!(
                "sysvdlg.unsupported_version: {path} is version {}; this build reads version \
                 {SYSVAR_FILE_VERSION}",
                file.version
            ));
        }
        if file.variables.is_empty() {
            return self.refuse_argument(format!(
                "sysvdlg.empty_file: {path} stores no variable to restore"
            ));
        }
        let mut variables: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in file.variables {
            let name = name.trim().to_ascii_uppercase();
            if name.is_empty() {
                return self.refuse_argument(format!(
                    "sysvdlg.invalid_file: {path} stores an empty variable name"
                ));
            }
            if variables.insert(name.clone(), value).is_some() {
                return self.refuse_argument(format!(
                    "sysvdlg.duplicate_variable: {path} names '{name}' more than once; nothing was \
                     applied"
                ));
            }
        }
        let unknown: Vec<&str> = variables
            .keys()
            .map(String::as_str)
            .filter(|name| !MODELLED_SYSTEM_VARIABLES.contains(name))
            .collect();
        if !unknown.is_empty() {
            return self.refuse_argument(format!(
                "sysvdlg.unknown_variable: {path} names {} variable(s) this build does not model \
                 ({}); nothing was applied",
                unknown.len(),
                unknown.join(", ")
            ));
        }
        let assignments: Vec<SysVarAssignment> = variables
            .into_iter()
            .map(|(name, value)| SysVarAssignment { name, value })
            .collect();
        // One Apply command: `cad-app` validates every name/value before its
        // first write, and publishes all header values in one revision.
        self.dispatch_typed_argument(
            SYSVDLG_SELECTOR,
            CommandId::Extended(ExtendedCommandId::SysVarDialog),
            cad_app::CommandPayload::SysVarDialog(SysVarDialogCommand::Apply(assignments)),
        )
    }

    /// `SAVE`/`RESTORE` write the drawing or a file, so a Viewer session may not
    /// run them. The bare report and the read form stay Viewer-legal.
    fn sysvdlg_write_allowed(&self, action: &str) -> Result<(), String> {
        if self.core.controller.borrow().session.app_mode() == AppMode::Viewer {
            return self.refuse_argument(format!(
                "sysvdlg.viewer_write: SYSVDLG {action} is an authoring operation; Viewer mode \
                 may report and read values but not {action} them"
            ));
        }
        Ok(())
    }
}

/// Parse one typed `SYSVDLG` argument.
fn parse_sysvdlg_argument(argument: &str) -> Option<SysVarDialogArgument> {
    let tokens = split_quoted_tokens(argument.trim())?;
    let first = tokens.first()?;
    let upper = first.to_ascii_uppercase();
    if upper == "SAVE" || upper == "RESTORE" {
        if tokens.len() != 2 || tokens[1].trim().is_empty() {
            return None;
        }
        return Some(if upper == "SAVE" {
            SysVarDialogArgument::Save(tokens[1].clone())
        } else {
            SysVarDialogArgument::Restore(tokens[1].clone())
        });
    }
    if upper.is_empty() {
        return None;
    }
    if tokens.len() == 1 {
        return Some(SysVarDialogArgument::Read(first.to_ascii_uppercase()));
    }
    Some(SysVarDialogArgument::Apply(vec![SysVarAssignment {
        name: first.to_ascii_uppercase(),
        value: tokens[1..].join(" "),
    }]))
}

/// A non-empty `.json` destination; the format is not `.svf`.
fn validated_json_path(path: &str, text: &str, action: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err(format!(
            "sysvdlg.{action}_needs_path: '{text}' needs a `.json` destination; this build has \
             no file dialog and no `.svf` codec"
        ));
    }
    let is_json = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !is_json {
        return Err(format!(
            "sysvdlg.file_extension: {path} must end in .json; this build writes its own \
             session-variable file, not AutoCAD .svf"
        ));
    }
    Ok(path.to_string())
}

fn save_error(path: &str, error: &NamedFileError) -> String {
    match error {
        NamedFileError::Unavailable => format!(
            "sysvdlg.save_unavailable: unsupported-runtime: sysvar-file: this host cannot write \
             the user-named file '{path}' ({})",
            error.reason()
        ),
        NamedFileError::Missing(reason) => format!("sysvdlg.save_failed: {path}: {reason}"),
        NamedFileError::Unreadable(reason) => {
            format!("sysvdlg.save_failed: {path}: {reason}")
        }
    }
}

fn restore_error(path: &str, error: &NamedFileError) -> String {
    match error {
        NamedFileError::Unavailable => format!(
            "sysvdlg.restore_unavailable: unsupported-runtime: sysvar-file: this host cannot read \
             the user-named file '{path}' ({})",
            error.reason()
        ),
        NamedFileError::Missing(reason) => format!("sysvdlg.file_missing: {path}: {reason}"),
        NamedFileError::Unreadable(reason) => {
            format!("sysvdlg.file_unreadable: {path}: {reason}")
        }
    }
}

/// The versioned session-variable file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SysVarFile {
    format: String,
    version: u32,
    variables: BTreeMap<String, String>,
}
