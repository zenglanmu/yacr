//! The typed `HATCH <option>` / `-HATCH <option>` line: the **pattern-name**
//! channel AutoCAD's `HATCH` prompts with.
//!
//! `HATCH` itself is a zero-point pre-selection tool: the current selection is
//! the boundary and the fill comes from the session tool-parameter channel
//! (`hatchPatternType`, `hatchPattern`, `hatchSpacing`, `hatchAngle`,
//! `hatchDouble`, `hatchStyle`). That channel's pattern selector is a **closed
//! 1-based index** over the three implemented `.pat` entries, so it cannot carry
//! a name. This module is that missing spelling — a typed form of the same
//! command, not a second command and not a second IPC.
//!
//! Grammar (case-insensitive, one line, no option stack):
//!
//! * `PATTERN <name>` — one built-in `.pat` library name (one token, no spaces;
//!   matched ASCII case-insensitively). A bare `PATTERN` is
//!   `hatch.needs_pattern_name`; a name the built-in table does not hold is
//!   `hatch.unknown_pattern`, answered with the names it does hold;
//! * `PATTERNFILE <path.pat> [<name>]` — the **same** resolution
//!   `HATCHEDIT PATTERNFILE` uses: the file the host read is searched first (a
//!   vendor file that redefines `ANSI31` wins), the built-in table second, and
//!   without a name the file must hold exactly one pattern. A second token that is
//!   itself a path is a spaced path the grammar cannot split from the name, so it
//!   is refused **by name** (`hatch.pattern_file_path`). The path travels to the
//!   host strategy ([`cad_host::HostResources::read_named_file`]) here — the
//!   command never touches the filesystem — so a missing/unreadable/non-UTF-8
//!   file is `hatch.pattern_file_missing` / `_unreadable` / `_encoding`, and a
//!   host without a filesystem is `hatch.pattern_file_unavailable`
//!   (`unsupported-runtime: pattern-file`);
//! * `SOLID` / `GRADIENT` — **named refusals** (`hatch.fill_not_pattern`): this
//!   typed form carries a pattern **name**; the fill kind is the session channel
//!   and a gradient has its own `GRADIENT` command. Refusing is better than
//!   ignoring the word or silently switching the fill.
//!
//! The selection is the boundary exactly as for the bare tool, so the command
//! keeps its one transaction / one undo step; the pattern resolves **before** the
//! document is read, and every refusal above writes nothing.

use cad_app::command_id::hatch::HatchPatternRequest;
use cad_app::{CommandId, CommandPayload, PatternFileText};
use cad_domain::hatch::predefined_patterns;
use cad_domain::SelectionRef;

use super::diagnostics::cad_error_text;
use super::hatchedit::parse_pattern_file_tokens;
use super::CadSession;

/// `HATCH PATTERN <name>` / `HATCH PATTERNFILE <path.pat> [<name>]`.
pub const HATCH_SELECTOR: &str = "HATCH";

/// The command-line spelling of the same typed forms (`-HATCH PATTERN …`).
pub const DASH_HATCH_SELECTOR: &str = "-HATCH";

/// The refusal-code prefix the `HATCH` pattern-file read uses (so the read
/// refusals answer with the command that owns the typed line).
pub(super) const HATCH_PATTERN_FILE_PREFIX: &str = "hatch";

/// Why a typed `HATCH` argument was refused **before** IPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HatchArgumentError {
    /// The line is outside the declared grammar.
    Malformed,
    /// `PATTERN` with no name: `hatch.needs_pattern_name`.
    MissingPatternName,
    /// `PATTERN <name>` where the built-in `.pat` library has no such name:
    /// `hatch.unknown_pattern`, answered with the accepted name list.
    UnknownPattern(String),
    /// `PATTERNFILE` whose second token is itself a path: a spaced path the
    /// grammar cannot tell from `<path> <name>` — `hatch.pattern_file_path`.
    PatternFilePath,
    /// `SOLID` / `GRADIENT`: the word names a fill kind this typed form does not
    /// carry — `hatch.fill_not_pattern`.
    FillNotPattern(&'static str),
    /// A `PATTERNFILE` request without the text the host read:
    /// `hatch.pattern_file_not_read` (only a payload built outside the session
    /// can reach this).
    FileNotRead,
}

/// One parsed `HATCH <option>` line, before any host read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HatchPatternArgument {
    /// `PATTERN <name>`: a built-in library name.
    Pattern(String),
    /// `PATTERNFILE <path> [<name>]`: a user file (read by the host in
    /// [`CadSession::hatch_argument`]).
    File { path: String, name: Option<String> },
}

/// Parse the argument of a `HATCH` / `-HATCH` selector.
///
/// Nothing is defaulted and no partial spelling is admitted: `PATTERN` needs
/// exactly one name token, `PATTERNFILE` one path and an optional name, and every
/// other line is [`HatchArgumentError::Malformed`] (the caller turns it into the
/// stable invalid-argument refusal). The two words that name a **different fill
/// kind** are parsed so the application can refuse them by name instead of
/// pretending the word was a typo.
pub fn parse_hatch_argument(argument: &str) -> Result<HatchPatternArgument, HatchArgumentError> {
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find([' ', '\t']) {
        Some(index) => (&trimmed[..index], trimmed[index..].trim()),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("PATTERN") {
        if rest.is_empty() {
            return Err(HatchArgumentError::MissingPatternName);
        }
        // A pattern name is one token: embedded whitespace is a malformed line,
        // not an unknown name (the library name set has no spaces).
        if rest.contains([' ', '\t']) {
            return Err(HatchArgumentError::Malformed);
        }
        // Resolved against the one built-in name table the `HATCHEDIT` grammar
        // also uses, so a typo is refused before IPC with the exact name set.
        if cad_domain::hatch::predefined_pattern(rest).is_none() {
            return Err(HatchArgumentError::UnknownPattern(rest.to_string()));
        }
        return Ok(HatchPatternArgument::Pattern(rest.to_string()));
    }
    if word.eq_ignore_ascii_case("PATTERNFILE") {
        return parse_pattern_file(rest);
    }
    if word.eq_ignore_ascii_case("SOLID") && rest.is_empty() {
        return Err(HatchArgumentError::FillNotPattern("SOLID"));
    }
    if word.eq_ignore_ascii_case("GRADIENT") && rest.is_empty() {
        return Err(HatchArgumentError::FillNotPattern("GRADIENT"));
    }
    Err(HatchArgumentError::Malformed)
}

/// Parse `PATTERNFILE <path.pat> [<name>]`: one or two whitespace-separated
/// tokens, never more. The content is **not** read here (the host does that), and
/// a second token carrying a path separator is a spaced path, refused by name.
fn parse_pattern_file(text: &str) -> Result<HatchPatternArgument, HatchArgumentError> {
    let (path, name) = parse_pattern_file_tokens(text).ok_or(HatchArgumentError::Malformed)?;
    if name
        .as_deref()
        .is_some_and(|name| name.contains(['/', '\\']))
    {
        return Err(HatchArgumentError::PatternFilePath);
    }
    Ok(HatchPatternArgument::File { path, name })
}

/// The stable refusal for a `HATCH` argument outside the grammar.
pub fn invalid_hatch_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (HATCH takes PATTERN <built-in name> or PATTERNFILE \
         <path.pat> [<name>]; SOLID/GRADIENT are named refusals — the fill kind is the session \
         channel and a gradient is the GRADIENT command; a bare HATCH starts the selection tool)",
        text.trim()
    )
}

/// The `hatch.needs_pattern_name` refusal: `PATTERN` with no name.
pub fn needs_hatch_pattern_name_reason() -> String {
    "hatch.needs_pattern_name: HATCH PATTERN needs a built-in pattern name, for example ANSI31"
        .to_string()
}

/// The `hatch.unknown_pattern` refusal: the accepted names come from the one
/// library table, never a second list.
pub fn hatch_unknown_pattern_reason(name: &str) -> String {
    let names = predefined_patterns()
        .iter()
        .map(|pattern| pattern.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "hatch.unknown_pattern: '{name}' is not a built-in pattern (this build implements \
         {names}); a user pattern needs HATCH PATTERNFILE <path.pat> [<name>]"
    )
}

/// The `hatch.pattern_file_path` refusal: a `PATTERNFILE` line whose second token
/// is a path, i.e. a spaced path the grammar cannot split from the name.
pub fn hatch_pattern_file_path_reason() -> String {
    "hatch.pattern_file_path: HATCH PATTERNFILE takes a whitespace-free `.pat` path and an \
     optional pattern name; a path containing spaces cannot be split from the name — give a path \
     without spaces"
        .to_string()
}

/// The `hatch.fill_not_pattern` refusal: the typed form carries a pattern name,
/// so the `SOLID`/`GRADIENT` words cannot be honoured here.
pub fn hatch_fill_not_pattern_reason(kind: &str) -> String {
    format!(
        "hatch.fill_not_pattern: HATCH {kind} is not the typed pattern form — HATCH PATTERN / \
         PATTERNFILE select a named `.pat` pattern, while the fill kind is the session \
         `hatchPatternType` channel; a gradient is created with the GRADIENT command. Nothing was \
         written"
    )
}

/// The stable refusal text for one typed-argument refusal.
pub fn hatch_argument_refusal(error: HatchArgumentError, text: &str) -> String {
    match error {
        HatchArgumentError::Malformed => invalid_hatch_argument_reason(text),
        HatchArgumentError::MissingPatternName => needs_hatch_pattern_name_reason(),
        HatchArgumentError::UnknownPattern(name) => hatch_unknown_pattern_reason(&name),
        HatchArgumentError::PatternFilePath => hatch_pattern_file_path_reason(),
        HatchArgumentError::FillNotPattern(kind) => hatch_fill_not_pattern_reason(kind),
        HatchArgumentError::FileNotRead => {
            "hatch.pattern_file_not_read: the `.pat` file content was not delivered by the host; \
             this build never reads the filesystem from the command"
                .to_string()
        }
    }
}

/// Build the exact `HATCH` payload for one parsed argument and boundary.
///
/// This is the **one** place a typed `HATCH` line becomes a
/// [`CommandPayload::HatchPattern`], so the dispatched payload is a pure function
/// of the parsed argument, the selection snapshot and (for a file) the text the
/// host read. `file_text` is ignored for the `PATTERN` form and required for
/// `PATTERNFILE` ([`HatchArgumentError::FileNotRead`]).
pub fn hatch_pattern_payload(
    argument: HatchPatternArgument,
    refs: Vec<SelectionRef>,
    file_text: Option<PatternFileText>,
) -> Result<CommandPayload, HatchArgumentError> {
    let request = match argument {
        HatchPatternArgument::Pattern(name) => HatchPatternRequest::Named(name),
        HatchPatternArgument::File { path, name } => {
            let Some(text) = file_text else {
                return Err(HatchArgumentError::FileNotRead);
            };
            HatchPatternRequest::File { path, name, text }
        }
    };
    Ok(CommandPayload::HatchPattern { refs, request })
}

impl CadSession {
    /// Apply one typed `HATCH <option>` line (also the `-HATCH` spelling).
    ///
    /// The current selection is the boundary, exactly like the bare tool. A
    /// `PATTERNFILE` path is read through the host strategy **after** the Viewer
    /// gate, so a command that cannot write never performs host I/O.
    pub(super) fn hatch_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let parsed = match parse_hatch_argument(argument) {
            Ok(parsed) => parsed,
            Err(error) => return self.refuse_argument(hatch_argument_refusal(error, text)),
        };
        // Viewer is refused **first** — before the host read and before any
        // selection or payload work — so a command that cannot write never
        // performs host I/O and the refusal text is the command layer's own.
        if let Err(error) = self
            .core
            .controller
            .borrow()
            .session
            .authorize(CommandId::Hatch)
        {
            return self.refuse_argument(cad_error_text(&error));
        }
        // The pattern file is the one form whose payload needs content: the host
        // reads it here (this session has no filesystem of its own and the command
        // must never read one). Every failure is a `hatch.pattern_file_*` refusal
        // recorded before IPC, and nothing is dispatched.
        let file_text = match &parsed {
            HatchPatternArgument::File { path, .. } => {
                match self.read_pattern_file_for(HATCH_PATTERN_FILE_PREFIX, CommandId::Hatch, path)
                {
                    Ok(file_text) => Some(file_text),
                    Err(reason) => return self.refuse_argument(reason),
                }
            }
            HatchPatternArgument::Pattern(_) => None,
        };
        let refs = {
            let controller = self.core.controller.borrow();
            controller.session.selection.refs().to_vec()
        };
        if refs.is_empty() {
            return self.refuse_argument(
                "hatch.selection: HATCH PATTERN/PATTERNFILE needs at least one selected closed \
                 boundary entity; nothing was written"
                    .to_string(),
            );
        }
        let payload = match hatch_pattern_payload(parsed, refs, file_text) {
            Ok(payload) => payload,
            Err(error) => return self.refuse_argument(hatch_argument_refusal(error, text)),
        };
        // Echo the admitted selector (never the typed argument). `-HATCH` shares
        // this one implementation, so it dispatches the same `CommandId::Hatch`.
        self.core.record_command(selector);
        match self.core.command(CommandId::Hatch, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
