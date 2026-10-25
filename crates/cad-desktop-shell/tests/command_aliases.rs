//! The command-alias IPC: the chrome's suggestion table.
//!
//! Aliases are a display aid, so the answer has two jobs: carry the real
//! abbreviations the router normalizes, and never publish a target that is not
//! itself an admitted canonical name (a suggestion must never point at a command
//! the host would refuse).

use cad_desktop_shell::commands::{cad_command_aliases_impl, COMMAND_NAMES};
use cad_ui_core::session::{admissible_names, canonical_command_name, normalize_command};

#[test]
fn cad_command_aliases_is_registered() {
    assert!(
        COMMAND_NAMES.contains(&"cad_command_aliases"),
        "cad_command_aliases must be registered with the other shell commands"
    );
}

#[test]
fn cad_command_aliases_reports_the_autocad_abbreviations_and_canonical_targets() {
    let aliases = cad_command_aliases_impl();
    assert!(!aliases.is_empty(), "the alias table must not be empty");

    let by_alias = |alias: &str| {
        aliases
            .iter()
            .find(|entry| entry.alias == alias)
            .map(|entry| entry.command.as_str())
    };
    assert_eq!(by_alias("L"), Some("LINE"));
    assert_eq!(by_alias("CO"), Some("COPY"));
    assert_eq!(by_alias("EX"), Some("EXTEND"));
    // The word synonyms that predate the abbreviation block are published too:
    // the answer is the whole `COMMAND_ALIASES` table, not just the new rows.
    assert_eq!(by_alias("EXPORT"), Some("PLOT"));
    // `MA` is AutoCAD's MATCHPROP abbreviation; MATCHPROP is not implemented, so
    // the alias must stay unpublished rather than invent a command.
    assert_eq!(by_alias("MA"), None);

    let admitted = admissible_names();
    for entry in &aliases {
        assert!(
            admitted.contains(&entry.command.as_str()),
            "{} targets {} which the router does not admit",
            entry.alias,
            entry.command
        );
        assert_eq!(
            canonical_command_name(&entry.command),
            Some(entry.command.as_str()),
            "{} target {} is not a canonical name",
            entry.alias,
            entry.command
        );
        // The target carries a real `CommandId` (aliases never point at the
        // interaction-only selectors) and the alias resolves to that same id.
        let id = normalize_command(&entry.command);
        assert!(id.is_some(), "{} has no CommandId", entry.command);
        assert_eq!(normalize_command(&entry.alias), id, "{}", entry.alias);
    }
}

#[test]
fn cad_command_aliases_serializes_as_camel_case_alias_and_command() {
    let aliases = cad_command_aliases_impl();
    let json = serde_json::to_value(&aliases).expect("serializes");
    let array = json.as_array().expect("an array of aliases");
    let line = array
        .iter()
        .find(|entry| entry.get("alias").and_then(|value| value.as_str()) == Some("L"))
        .expect("L is published");
    assert_eq!(
        line.get("command").and_then(|value| value.as_str()),
        Some("LINE")
    );
    // Exactly the two documented keys, so the React side reads a frozen shape.
    let keys: std::collections::BTreeSet<&str> = line
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["alias", "command"].into_iter().collect());
}
