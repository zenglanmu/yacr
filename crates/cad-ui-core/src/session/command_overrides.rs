//! Session-only command-vocabulary overrides: `UNDEFINE`, `REDEFINE` and the
//! real `ALIASEDIT` forms.
//!
//! # Why this lives in the session
//!
//! AutoCAD's `UNDEFINE`, `REDEFINE` and `ALIASEDIT` change how the command line
//! resolves **names**, not the drawing. They are therefore session state, kept
//! here next to the one resolution funnel ([`CadSession::command`]) rather than
//! in `cad-app` (which has no session vocabulary) or in the static tables (which
//! are the build's fixed vocabulary and must not be mutated per session).
//!
//! The rules implemented here are the resolved contract:
//!
//! * **`UNDEFINE <name>`** disables one built-in command for the rest of the
//!   session. A disabled name behaves exactly like an unknown command: dispatch
//!   refuses with the stable `command.undefined: <NAME>` reason and nothing is
//!   dispatched (no capture, no transaction, no revision, no undo entry). An
//!   alias (static or runtime) whose target is disabled is disabled too. The
//!   argument is resolved against the **static** vocabulary, so the tab-complete
//!   spelling (canonical name or shipped alias) works, but a runtime alias name
//!   is not a command and is refused.
//! * **`REDEFINE <name>`** removes one entry from that disabled set. A name that
//!   is not currently disabled is an explicit `redefine.not_undefined` refusal;
//!   a name outside the vocabulary is `redefine.unknown`.
//! * **`ALIASEDIT ADD <alias> <target>` / `REMOVE <alias>` / `LIST`** (bare
//!   `ALIASEDIT` is `LIST`) maintains a runtime alias table that is consulted
//!   **before** the static tables, so an added alias shadows a built-in selector
//!   of the same spelling and overrides a shipped `acad.pgp` abbreviation when it
//!   reuses one. `REMOVE` only ever removes runtime rows: a shipped alias cannot
//!   be deleted. Every row is session-only; no `acad.pgp` file is read or
//!   written, and `LIST` inspects the live session (the pre-existing `cad-app`
//!   `AliasEdit` report remains the build-table read for the payload path).
//!
//! # Essential commands
//!
//! [`ESSENTIAL_COMMANDS`] cannot be undefined, redefined or shadowed by a
//! runtime alias: they are the user's recovery/document path. The refusal code is
//! the stable `redefine.essential` on both directions (`UNDEFINE` and
//! `REDEFINE`), and `aliasedit.essential` when an alias would shadow one.
//!
//! # Residuals (declared, not faked)
//!
//! * `ALIASEDIT` has no interactive dialog: the typed `ADD`/`REMOVE`/`LIST`
//!   forms are the whole surface, exactly like the other dialog-free commands in
//!   this build.
//! * A runtime alias substitutes a command **name**; `LIST` returns the rows to a
//!   host/test through [`CadSession::runtime_command_aliases`], but this lane
//!   adds no command-window row rendering (the log is text-free). The observable
//!   effect is the resolution behaviour itself.
//! * Overrides are per session. Opening another drawing in the same session keeps
//!   them (they are vocabulary, not document state); a new session starts from
//!   the build's static tables.

use std::collections::{BTreeMap, BTreeSet};

use super::commands::{
    canonical_command_name, ALIASEDIT_SELECTOR, ARGUMENT_SELECTORS, COMMAND_ALIASES,
    ESSENTIAL_COMMANDS, REDEFINE_SELECTOR, UNDEFINE_SELECTOR,
};
use super::CadSession;

/// Longest accepted runtime alias name (AutoCAD's `acad.pgp` aliases are short
/// abbreviations; the bound keeps a typed line unambiguous).
pub const MAX_ALIAS_LEN: usize = 20;

/// What the session's runtime tables say about one typed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeResolution {
    /// A runtime alias maps the line to this canonical selector.
    Alias(&'static str),
    /// The line is disabled (directly, or through an alias whose target is).
    Undefined(String),
    /// No runtime override applies; the static tables answer.
    Static,
}

/// The session's mutable command-vocabulary state.
///
/// `disabled` holds **canonical** selectors (so `UNDEFINE L` disables `LINE` and
/// with it every alias onto `LINE`). `aliases` maps an uppercased runtime alias
/// to its canonical target; it is consulted before the static tables.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CommandOverrides {
    disabled: BTreeSet<&'static str>,
    aliases: BTreeMap<String, &'static str>,
}

impl CommandOverrides {
    /// Resolve one typed line against the runtime tables before the static ones.
    pub(crate) fn resolve(&self, text: &str) -> RuntimeResolution {
        let key = text.trim().to_ascii_uppercase();
        if let Some(target) = self.aliases.get(&key) {
            if self.disabled.contains(target) {
                // The alias is disabled together with its target; the refusal
                // names the spelling the user typed.
                return RuntimeResolution::Undefined(key);
            }
            return RuntimeResolution::Alias(target);
        }
        match canonical_command_name(text) {
            Some(canonical) if self.disabled.contains(&canonical) => {
                // Name the spelling the user typed: a static alias (`C` for a
                // disabled `CIRCLE`) is reported as `C`, exactly like a runtime
                // alias; a canonical spelling is reported as itself.
                RuntimeResolution::Undefined(key)
            }
            _ => RuntimeResolution::Static,
        }
    }

    /// The canonical target of a **name-only** runtime alias, if any.
    pub(crate) fn alias_target(&self, text: &str) -> Option<&'static str> {
        self.aliases.get(&text.trim().to_ascii_uppercase()).copied()
    }

    /// Whether one canonical selector is disabled.
    pub(crate) fn is_undefined(&self, canonical: &str) -> bool {
        self.disabled.contains(canonical)
    }

    /// `UNDEFINE <name>`: add one canonical selector to the disabled set.
    pub(crate) fn disable(&mut self, name: &str) -> Result<(), String> {
        let canonical = self.builtin_name(name)?;
        if ESSENTIAL_COMMANDS.contains(&canonical) {
            return Err(essential_reason(canonical));
        }
        if !self.disabled.insert(canonical) {
            return Err(format!("redefine.already_undefined: {canonical}"));
        }
        Ok(())
    }

    /// `REDEFINE <name>`: remove one canonical selector from the disabled set.
    pub(crate) fn enable(&mut self, name: &str) -> Result<(), String> {
        let canonical = self.builtin_name(name)?;
        if ESSENTIAL_COMMANDS.contains(&canonical) {
            return Err(essential_reason(canonical));
        }
        if !self.disabled.remove(canonical) {
            return Err(format!("redefine.not_undefined: {canonical}"));
        }
        Ok(())
    }

    /// `ALIASEDIT ADD <alias> <target>`: insert one runtime alias row.
    pub(crate) fn add_alias(&mut self, alias: &str, target: &str) -> Result<(), String> {
        let key = validate_alias(alias)?;
        let Some(canonical) = canonical_command_name(target) else {
            return Err(format!("aliasedit.unknown_target: {}", target.trim()));
        };
        if ESSENTIAL_COMMANDS.contains(&key.as_str()) {
            return Err(format!("aliasedit.essential: {key}"));
        }
        if COMMAND_ALIASES.iter().any(|(shipped, _)| *shipped == key)
            || self.aliases.contains_key(&key)
        {
            return Err(format!("aliasedit.duplicate: {key}"));
        }
        self.aliases.insert(key, canonical);
        Ok(())
    }

    /// `ALIASEDIT REMOVE <alias>`: drop one **runtime** alias row.
    pub(crate) fn remove_alias(&mut self, alias: &str) -> Result<(), String> {
        let key = validate_alias(alias)?;
        if COMMAND_ALIASES.iter().any(|(shipped, _)| *shipped == key) {
            return Err(format!("aliasedit.builtin: {key}"));
        }
        if self.aliases.remove(&key).is_none() {
            return Err(format!("aliasedit.unknown: {key}"));
        }
        Ok(())
    }

    /// Every runtime alias row, in stable (uppercased-alias) order.
    pub(crate) fn alias_rows(&self) -> Vec<(String, &'static str)> {
        self.aliases
            .iter()
            .map(|(alias, target)| (alias.clone(), *target))
            .collect()
    }

    /// Every disabled canonical selector, in stable order.
    pub(crate) fn undefined_rows(&self) -> Vec<&'static str> {
        self.disabled.iter().copied().collect()
    }

    /// Resolve a `UNDEFINE`/`REDEFINE` argument to a built-in canonical name.
    ///
    /// The static tables are the authority: a runtime alias is not a command and
    /// a name outside the vocabulary is refused by name.
    fn builtin_name(&self, name: &str) -> Result<&'static str, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("redefine.missing_name: <empty>".into());
        }
        canonical_command_name(trimmed).ok_or_else(|| format!("redefine.unknown: {trimmed}"))
    }
}

/// Validate and upper-case a runtime alias name.
fn validate_alias(alias: &str) -> Result<String, String> {
    let trimmed = alias.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= MAX_ALIAS_LEN
        && trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '?' | '*' | '+')
        });
    if !valid {
        return Err(format!("aliasedit.invalid_name: {trimmed}"));
    }
    Ok(trimmed.to_ascii_uppercase())
}

/// The one essential-command refusal used by both directions.
fn essential_reason(canonical: &str) -> String {
    format!(
        "redefine.essential: {canonical} is an essential command and cannot be undefined or \
         redefined"
    )
}

impl CadSession {
    /// Handle one session-only vocabulary-control line, if `text` is one.
    ///
    /// Runs before the ordinary command funnel so `UNDEFINE`/`REDEFINE`/
    /// `ALIASEDIT` never reach `normalize_command` (they carry no `CommandId`) and
    /// so the control lines themselves can never be disabled.
    pub(super) fn vocabulary_command(&self, text: &str) -> Option<Result<(), String>> {
        let trimmed = text.trim();
        let (head, rest) = split_head(trimmed);
        let selector = head.to_ascii_uppercase();
        match selector.as_str() {
            UNDEFINE_SELECTOR => Some(self.undefine_command(rest)),
            REDEFINE_SELECTOR => Some(self.redefine_command(rest)),
            ALIASEDIT_SELECTOR => Some(self.aliasedit_command(rest, trimmed)),
            _ => None,
        }
    }

    /// `UNDEFINE <name>`: disable one built-in command for this session.
    fn undefine_command(&self, argument: &str) -> Result<(), String> {
        if argument.trim().is_empty() {
            return self.refuse_argument(format!(
                "redefine.missing_name: {UNDEFINE_SELECTOR} needs one built-in command name"
            ));
        }
        let reason = {
            let mut overrides = self.overrides.borrow_mut();
            overrides.disable(argument).err()
        };
        match reason {
            Some(reason) => self.refuse_argument(reason),
            None => self.note_vocabulary_command(UNDEFINE_SELECTOR),
        }
    }

    /// `REDEFINE <name>`: re-enable one disabled built-in command.
    fn redefine_command(&self, argument: &str) -> Result<(), String> {
        if argument.trim().is_empty() {
            return self.refuse_argument(format!(
                "redefine.missing_name: {REDEFINE_SELECTOR} needs one built-in command name"
            ));
        }
        let reason = {
            let mut overrides = self.overrides.borrow_mut();
            overrides.enable(argument).err()
        };
        match reason {
            Some(reason) => self.refuse_argument(reason),
            None => self.note_vocabulary_command(REDEFINE_SELECTOR),
        }
    }

    /// The three typed `ALIASEDIT` forms (bare is `LIST`).
    fn aliasedit_command(&self, argument: &str, text: &str) -> Result<(), String> {
        let trimmed = argument.trim();
        if trimmed.is_empty() {
            return self.alias_list();
        }
        let (verb, rest) = split_head(trimmed);
        let verb = verb.to_ascii_uppercase();
        match verb.as_str() {
            "LIST" => {
                if rest.trim().is_empty() {
                    self.alias_list()
                } else {
                    self.refuse_argument(malformed_reason(text))
                }
            }
            "ADD" => {
                let (alias, target) = split_head(rest);
                let target = target.trim();
                if alias.is_empty() || target.is_empty() {
                    return self.refuse_argument(malformed_reason(text));
                }
                let reason = {
                    let mut overrides = self.overrides.borrow_mut();
                    overrides.add_alias(alias, target).err()
                };
                match reason {
                    Some(reason) => self.refuse_argument(reason),
                    None => self.note_vocabulary_command(ALIASEDIT_SELECTOR),
                }
            }
            "REMOVE" => {
                let alias = rest.trim();
                if alias.is_empty() || alias.split_whitespace().count() != 1 {
                    return self.refuse_argument(malformed_reason(text));
                }
                let reason = {
                    let mut overrides = self.overrides.borrow_mut();
                    overrides.remove_alias(alias).err()
                };
                match reason {
                    Some(reason) => self.refuse_argument(reason),
                    None => self.note_vocabulary_command(ALIASEDIT_SELECTOR),
                }
            }
            _ => self.refuse_argument(malformed_reason(text)),
        }
    }

    /// `ALIASEDIT LIST`: read the live session's alias table.
    ///
    /// The rows are exposed through [`Self::runtime_command_aliases`] (and the
    /// shipped table through [`COMMAND_ALIASES`]); this command records its
    /// selector echo and changes nothing.
    fn alias_list(&self) -> Result<(), String> {
        self.note_vocabulary_command(ALIASEDIT_SELECTOR)
    }

    /// Echo the admitted selector and clear the last error, like every other
    /// successful command funnel.
    fn note_vocabulary_command(&self, selector: &'static str) -> Result<(), String> {
        self.core.record_command(selector);
        *self.error.borrow_mut() = None;
        Ok(())
    }

    /// Record the stable `command.undefined: <NAME>` refusal and return it.
    pub(super) fn refuse_undefined(&self, name: &str) -> Result<(), String> {
        self.refuse_argument(format!("command.undefined: {name}"))
    }

    /// The runtime `ALIASEDIT ADD` rows, for hosts and contract tests.
    ///
    /// Session-only state: nothing here is persisted, and a new session starts
    /// from the build's static [`COMMAND_ALIASES`] table.
    pub fn runtime_command_aliases(&self) -> Vec<(String, String)> {
        self.overrides
            .borrow()
            .alias_rows()
            .into_iter()
            .map(|(alias, target)| (alias, target.to_string()))
            .collect()
    }

    /// The canonical selectors currently disabled by `UNDEFINE`, stable order.
    pub fn undefined_commands(&self) -> Vec<&'static str> {
        self.overrides.borrow().undefined_rows()
    }
}

/// `(head, rest)` of one typed line: the first whitespace-delimited token and
/// everything after it (trailing/leading whitespace trimmed).
fn split_head(text: &str) -> (&str, &str) {
    let trimmed = text.trim();
    match trimmed.find([' ', '\t']) {
        Some(index) => (&trimmed[..index], trimmed[index..].trim()),
        None => (trimmed, ""),
    }
}

/// The stable refusal for an `ALIASEDIT` line outside the three typed forms.
fn malformed_reason(text: &str) -> String {
    format!(
        "aliasedit.malformed: `{}` is not `ALIASEDIT`, `ALIASEDIT LIST`, `ALIASEDIT ADD <alias> \
         <target>` or `ALIASEDIT REMOVE <alias>`; nothing was changed",
        text.trim()
    )
}

/// Whether `typed_command_argument` can admit `<target> <argument>` for a runtime
/// alias (used by the shell gate, which normalizes before the queue).
///
/// Name-only aliases need no check: any canonical target is a real selector.
pub(crate) fn alias_accepts_argument(target: &str) -> bool {
    ARGUMENT_SELECTORS.contains(&target)
        || super::commands::SHELL_SETTING_SELECTORS.contains(&target)
}

/// Split a typed line into its first token and non-empty remainder.
///
/// The caller has already established that the whole line is not a static
/// selector; this only supplies the base name the runtime alias table may map.
pub(crate) fn runtime_alias_argument(text: &str) -> Option<(&str, &str)> {
    let (head, rest) = split_head(text);
    (!head.is_empty() && !rest.is_empty()).then_some((head, rest))
}
