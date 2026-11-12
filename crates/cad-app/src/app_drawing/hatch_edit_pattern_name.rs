//! The `HATCHEDIT PATTERN <name>` library lookup, split out of `hatch_edit.rs`
//! to keep that file inside the code-size gate.
//!
//! The built-in `.pat` library ([`PatternLibrary`]) is the single name authority:
//! its rows are pinned to `cad_domain::hatch::predefined_patterns` by the
//! anti-drift test in `hatch_pattern.rs`, and neither the `cad-ui-core` grammar
//! nor this command carries a second name list. The two refusals here are the
//! command-side (defence in depth) half of the pre-IPC ones:
//! `hatchedit.needs_pattern_name` and `hatchedit.unknown_pattern`.

use super::hatch_edit::invalid;
use super::hatch_pattern::{PatternDefinition, PatternLibrary};

/// Resolve a typed `PATTERN <name>` against the built-in `.pat` library.
///
/// An empty name is `hatchedit.needs_pattern_name`; a name the library does not
/// hold is `hatchedit.unknown_pattern` and lists every accepted name. Nothing is
/// approximated onto a neighbouring pattern and no default is substituted.
pub(super) fn resolve_builtin_pattern(
    name: &str,
) -> cad_domain::CadResult<&'static PatternDefinition> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(invalid(
            "needs_pattern_name",
            "HATCHEDIT PATTERN needs a built-in pattern name, for example ANSI31",
        ));
    }
    PatternLibrary::builtin().find(trimmed).ok_or_else(|| {
        let names = PatternLibrary::builtin()
            .definitions()
            .iter()
            .map(|definition| definition.name())
            .collect::<Vec<_>>()
            .join(", ");
        invalid(
            "unknown_pattern",
            format!("'{trimmed}' is not a built-in pattern (this build implements {names})"),
        )
    })
}
