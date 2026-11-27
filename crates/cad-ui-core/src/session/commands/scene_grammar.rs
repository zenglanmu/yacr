//! The `SCENE <option>` typed grammar, split out of `arguments.rs` for the
//! code-size gate.
//!
//! `SCENE SAVE <name>`, `RESTORE <name>`, `DELETE <name>` and `LIST`; the keyword
//! is case-insensitive and a name is the trimmed remainder (it may contain
//! spaces). Nothing else is admitted: a bare selector is refused by name and an
//! out-of-grammar line is the invalid-argument refusal.

use cad_app::SceneCommand;

/// The SketchUp `SCENE` selector.
pub const SCENE_SELECTOR: &str = "SCENE";

/// Parse one `SCENE` option line.
pub fn parse_scene_argument(argument: &str) -> Option<SceneCommand> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    match keyword.to_ascii_uppercase().as_str() {
        "LIST" if rest.is_empty() => Some(SceneCommand::List),
        "SAVE" if !rest.is_empty() => Some(SceneCommand::Save(rest.to_string())),
        "RESTORE" if !rest.is_empty() => Some(SceneCommand::Restore(rest.to_string())),
        "DELETE" if !rest.is_empty() => Some(SceneCommand::Delete(rest.to_string())),
        _ => None,
    }
}

/// The stable rejection reason for a `SCENE` line outside the grammar.
pub fn invalid_scene_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SCENE needs SAVE <name>, RESTORE <name>, LIST or DELETE \
         <name>)",
        text.trim()
    )
}
