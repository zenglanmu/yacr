//! `HATCHEDIT PATTERNFILE <path.pat> [<name>]`: a pattern from a user `.pat`
//! file, split out of `hatch_edit.rs` for the code-size gate.
//!
//! The file content arrives **in the payload**: the typed line is parsed in the
//! session, which cannot read a filesystem (it also runs in the browser), so the
//! host strategy (`cad_host::HostResources::read_named_file`) performed the read
//! and the command only parses it. The parser is the one
//! [`PatternLibrary`](super::hatch_pattern::PatternLibrary) the HATCH creation
//! path and `PATTERN <name>` already use, so a user file and the built-in table
//! cannot drift into two readers.
//!
//! Resolution rules (nothing is approximated):
//!
//! * no name → the file must hold exactly one pattern; two or more is
//!   `hatchedit.pattern_file_ambiguous` (naming them, and the disambiguating
//!   `PATTERNFILE <path> <name>` form);
//! * a name → the **file** is searched first (the user pointed at it, so a
//!   vendor file that redefines `ANSI31` wins), the built-in library second; in
//!   neither is `hatchedit.unknown_pattern`, naming both sources;
//! * an unparsable/empty file is `hatchedit.pattern_file_invalid` with the
//!   parser's own refusal (and its path) in the detail.
//!
//! The DXF classification follows the source of the rows: a file definition is a
//! custom pattern (`76 = 2`), a definition that fell back to the built-in table
//! is predefined (`76 = 1`).

use super::hatch_edit::invalid;
use super::hatch_pattern::{PatternDefinition, PatternLibrary};
use cad_domain::{CadResult, HatchPatternType};

/// Which library supplied the rows of the resolved pattern.
pub(super) enum PatternChoice {
    /// The built-in transcribed library (`PATTERN <name>`, or a `PATTERNFILE`
    /// name that only the built-in table holds).
    Builtin(&'static PatternDefinition),
    /// A definition parsed from the user's `.pat` file.
    File(PatternDefinition),
}

impl PatternChoice {
    pub(super) fn definition(&self) -> &PatternDefinition {
        match self {
            PatternChoice::Builtin(definition) => definition,
            PatternChoice::File(definition) => definition,
        }
    }

    /// DXF group 76 for the resolved pattern.
    pub(super) fn pattern_type(&self) -> HatchPatternType {
        match self {
            PatternChoice::Builtin(_) => HatchPatternType::Predefined,
            PatternChoice::File(_) => HatchPatternType::Custom,
        }
    }
}

/// Resolve `PATTERNFILE <path> [<name>]` from the text the host read.
pub(super) fn resolve_file_pattern(
    path: &str,
    name: Option<&str>,
    text: Option<&str>,
) -> CadResult<PatternChoice> {
    let path = path.trim();
    if path.is_empty() {
        return Err(invalid(
            "pattern_file_path",
            "HATCHEDIT PATTERNFILE needs a `.pat` file path",
        ));
    }
    let Some(text) = text else {
        // Defence in depth: only a payload built outside the session can get
        // here, and reading the file is the host's job, never this command's.
        return Err(cad_domain::CadError::Unsupported(format!(
            "hatchedit.pattern_file_not_read: the content of '{path}' was not delivered by the \
             host; this command does not read the filesystem"
        )));
    };
    let library = PatternLibrary::parse(text)
        .map_err(|error| invalid("pattern_file_invalid", format!("{path}: {error}")))?;
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => {
            if let Some(definition) = library.find(name) {
                return Ok(PatternChoice::File(definition.clone()));
            }
            // The name may be a predefined one the user expects this file to be
            // searched alongside; the built-in table is the second source.
            if let Some(definition) = PatternLibrary::builtin().find(name) {
                return Ok(PatternChoice::Builtin(definition));
            }
            Err(invalid(
                "unknown_pattern",
                format!(
                    "'{name}' is neither in {path} ({}) nor a built-in pattern ({})",
                    names(&library),
                    builtin_names()
                ),
            ))
        }
        None => match library.definitions() {
            [only] => Ok(PatternChoice::File(only.clone())),
            many => Err(invalid(
                "pattern_file_ambiguous",
                format!(
                    "{path} holds {} patterns ({}); name one: HATCHEDIT PATTERNFILE {path} <name>",
                    many.len(),
                    names(&library)
                ),
            )),
        },
    }
}

fn names(library: &PatternLibrary) -> String {
    library
        .definitions()
        .iter()
        .map(|definition| definition.name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn builtin_names() -> String {
    PatternLibrary::builtin()
        .definitions()
        .iter()
        .map(|definition| definition.name())
        .collect::<Vec<_>>()
        .join(", ")
}
