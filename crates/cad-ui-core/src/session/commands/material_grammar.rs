//! The `MATERIAL` typed grammar.
//!
//! `MATERIAL <name> RENAME <new>`, `... COLOR <r> <g> <b>`, `...
//! TRANSPARENCY <0..=1>` and `... DELETE`. A name with spaces is double-quoted
//! (same rule as the group grammar); the command layer resolves and validates
//! the row itself.

use cad_app::MaterialCommand;

/// The `MATERIAL` selector.
pub const MATERIAL_SELECTOR: &str = "MATERIAL";

/// Parse one `MATERIAL` edit line.
pub fn parse_material_argument(argument: &str) -> Option<MaterialCommand> {
    let (name_token, rest) = split_name(argument.trim());
    let name = parse_name(name_token)?;
    let (verb, rest) = split_word(rest);
    if verb.eq_ignore_ascii_case("RENAME") {
        let new_name = parse_name(rest)?;
        return Some(MaterialCommand::Rename { name, new_name });
    }
    if verb.eq_ignore_ascii_case("COLOR") {
        let mut channels = rest.split_whitespace();
        let mut color = [0_u8; 3];
        for channel in &mut color {
            *channel = channels.next()?.parse::<u8>().ok()?;
        }
        if channels.next().is_some() {
            return None;
        }
        return Some(MaterialCommand::Color { name, color });
    }
    if verb.eq_ignore_ascii_case("TRANSPARENCY") {
        let value = rest.trim().parse::<f64>().ok()?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return None;
        }
        return Some(MaterialCommand::Transparency {
            name,
            transparency: value,
        });
    }
    if verb.eq_ignore_ascii_case("DELETE") && rest.trim().is_empty() {
        return Some(MaterialCommand::Delete { name });
    }
    None
}

/// The stable rejection reason for a `MATERIAL` line outside the grammar.
pub fn invalid_material_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (MATERIAL takes <name> RENAME <new>, COLOR <r> <g> <b>, \
         TRANSPARENCY <0..=1> or DELETE)",
        text.trim()
    )
}

fn split_word(text: &str) -> (&str, &str) {
    match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim_start()),
        None => (text, ""),
    }
}

fn split_name(text: &str) -> (&str, &str) {
    if let Some(rest) = text.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let quoted_len = end + 2;
            return (&text[..quoted_len], text[quoted_len..].trim_start());
        }
    }
    split_word(text)
}

fn parse_name(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('"') {
        let (name, tail) = rest.split_once('"')?;
        let name = name.trim();
        if name.is_empty() || !tail.trim().is_empty() {
            return None;
        }
        return Some(name.to_string());
    }
    (!text.is_empty() && text.split_whitespace().count() == 1).then(|| text.to_string())
}
