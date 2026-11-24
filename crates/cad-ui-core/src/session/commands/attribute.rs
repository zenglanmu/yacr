//! Typed command lines for the block-attribute family (ADR-0055 M3).
//!
//! The parsers are pure syntax: they return the same [`cad_app`] payload the
//! command layer validates, or `None` outside the grammar. The application is
//! always the semantic authority (unknown block/tag, constants, locked
//! positions, the selection and every limit are re-checked there).
//!
//! Grammar summary:
//!
//! * `ATTDEF` / `-ATTDEF` / `ATTREDEF`
//!   <block> <tag> <default> [PROMPT <text>] [POSITION <x> <y> [<z>]]
//!   [HEIGHT <h>] [ROTATION <degrees>] [ORDER <n>] [WIDTH <factor>]
//!   [WRAP <width>] [OBLIQUE <degrees>] [CONSTANT] [INVISIBLE] [VERIFY]
//!   [PRESET] [LOCK] [ANNOTATIVE]`
//! * `ATTEDIT` / `-ATTEDIT` / `ATTIPEDIT` / `GATTE <block> <tag> <value>`
//! * `EATTEDIT <block> <tag>=<value> [<tag>=<value> ...]
//!   [POSITION <x> <y> [<z>]] [HEIGHT <h>] [ROTATION <degrees>]`
//! * `ATTSYNC <block>`
//! * `BATTORDER <block> <tag>,<tag>,...`
//! * `ATTDISP NORMAL|ON|OFF`
//!
//! Blocks, tags and values may be double-quoted when they contain whitespace;
//! an unbalanced quote is outside the grammar.

use cad_app::{
    AttributeAssignment, AttributeDefinitionCommand, AttributeDisplayMode, AttributeEditCommand,
    AttributeEditScope, AttributeOrderCommand, AttributeSyncCommand,
};
use cad_domain::Point3;

pub const ATTDEF_SELECTOR: &str = "ATTDEF";
pub const DASH_ATTDEF_SELECTOR: &str = "-ATTDEF";
pub const ATTEDIT_SELECTOR: &str = "ATTEDIT";
pub const DASH_ATTEDIT_SELECTOR: &str = "-ATTEDIT";
pub const EATTEDIT_SELECTOR: &str = "EATTEDIT";
pub const ATTIPEDIT_SELECTOR: &str = "ATTIPEDIT";
pub const GATTE_SELECTOR: &str = "GATTE";
pub const ATTSYNC_SELECTOR: &str = "ATTSYNC";
pub const ATTREDEF_SELECTOR: &str = "ATTREDEF";
pub const BATTORDER_SELECTOR: &str = "BATTORDER";
pub const ATTDISP_SELECTOR: &str = "ATTDISP";

/// Parse the definition grammar shared by `ATTDEF`, `-ATTDEF` and `ATTREDEF`.
pub fn parse_attribute_definition(argument: &str) -> Option<AttributeDefinitionCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() < 3 {
        return None;
    }
    let mut payload = AttributeDefinitionCommand {
        block: tokens[0].clone(),
        tag: tokens[1].clone(),
        default_value: tokens[2].clone(),
        ..AttributeDefinitionCommand::default()
    };
    if payload.block.trim().is_empty() || payload.tag.trim().is_empty() {
        return None;
    }
    let mut index = 3;
    while index < tokens.len() {
        let keyword = tokens[index].to_ascii_uppercase();
        index += 1;
        match keyword.as_str() {
            "PROMPT" => {
                payload.prompt = tokens.get(index)?.clone();
                index += 1;
            }
            "POSITION" => {
                let x = parse_number(tokens.get(index)?)?;
                let y = parse_number(tokens.get(index + 1)?)?;
                index += 2;
                let z = if index < tokens.len() && !is_option(&tokens[index]) {
                    let z = parse_number(&tokens[index])?;
                    index += 1;
                    z
                } else {
                    0.0
                };
                payload.position = Point3 { x, y, z };
            }
            "HEIGHT" => {
                payload.height = parse_number(tokens.get(index)?)?;
                index += 1;
            }
            "ROTATION" => {
                payload.rotation = parse_number(tokens.get(index)?)?.to_radians();
                index += 1;
            }
            "ORDER" => {
                payload.order = Some(tokens.get(index)?.parse::<u32>().ok()?);
                index += 1;
            }
            "WIDTH" => {
                payload.width_factor = parse_number(tokens.get(index)?)?;
                index += 1;
            }
            "WRAP" => {
                payload.wrap_width = Some(parse_number(tokens.get(index)?)?);
                index += 1;
            }
            "OBLIQUE" => {
                payload.oblique = parse_number(tokens.get(index)?)?.to_radians();
                index += 1;
            }
            "CONSTANT" => payload.flags.constant = true,
            "INVISIBLE" => payload.flags.invisible = true,
            "VERIFY" => payload.flags.verify = true,
            "PRESET" => payload.flags.preset = true,
            "LOCK" | "LOCKPOSITION" => payload.flags.locked_position = true,
            "ANNOTATIVE" => payload.flags.annotative = true,
            _ => return None,
        }
    }
    if !(payload.height.is_finite() && payload.height > 0.0) {
        return None;
    }
    if !(payload.width_factor.is_finite() && payload.width_factor > 0.0) {
        return None;
    }
    if let Some(width) = payload.wrap_width {
        if !(width.is_finite() && width > 0.0) {
            return None;
        }
    }
    if !(payload.position.x.is_finite()
        && payload.position.y.is_finite()
        && payload.position.z.is_finite()
        && payload.rotation.is_finite()
        && payload.oblique.is_finite())
    {
        return None;
    }
    Some(payload)
}

/// Parse one simple value edit (`<block> <tag> <value>`).
pub fn parse_attribute_edit(
    argument: &str,
    scope: AttributeEditScope,
) -> Option<AttributeEditCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() != 3 {
        return None;
    }
    if tokens[0].trim().is_empty() || tokens[1].trim().is_empty() {
        return None;
    }
    Some(AttributeEditCommand {
        block: tokens[0].clone(),
        assignments: vec![AttributeAssignment {
            tag: tokens[1].clone(),
            value: tokens[2].clone(),
        }],
        scope,
        position: None,
        height: None,
        rotation: None,
    })
}

/// Parse the enhanced (`EATTEDIT`) multi-assignment/placement grammar.
pub fn parse_eattedit(argument: &str) -> Option<AttributeEditCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() < 2 {
        return None;
    }
    let mut edit = AttributeEditCommand {
        block: tokens[0].clone(),
        assignments: Vec::new(),
        scope: AttributeEditScope::Selection,
        position: None,
        height: None,
        rotation: None,
    };
    if edit.block.trim().is_empty() {
        return None;
    }
    let mut index = 1;
    let mut saw_assignment = false;
    while index < tokens.len() {
        let token = &tokens[index];
        let upper = token.to_ascii_uppercase();
        match upper.as_str() {
            "POSITION" => {
                let x = parse_number(tokens.get(index + 1)?)?;
                let y = parse_number(tokens.get(index + 2)?)?;
                index += 3;
                let z = if index < tokens.len() && !is_option(&tokens[index]) {
                    let z = parse_number(&tokens[index])?;
                    index += 1;
                    z
                } else {
                    0.0
                };
                edit.position = Some(Point3 { x, y, z });
            }
            "HEIGHT" => {
                edit.height = Some(parse_number(tokens.get(index + 1)?)?);
                index += 2;
            }
            "ROTATION" => {
                edit.rotation = Some(parse_number(tokens.get(index + 1)?)?.to_radians());
                index += 2;
            }
            _ => {
                let (tag, value) = token.split_once('=')?;
                if tag.trim().is_empty() {
                    return None;
                }
                saw_assignment = true;
                edit.assignments.push(AttributeAssignment {
                    tag: tag.to_string(),
                    value: value.to_string(),
                });
                index += 1;
            }
        }
    }
    if !saw_assignment {
        return None;
    }
    if let Some(height) = edit.height {
        if !(height.is_finite() && height > 0.0) {
            return None;
        }
    }
    if edit.rotation.is_some_and(|rotation| !rotation.is_finite()) {
        return None;
    }
    Some(edit)
}

/// Parse `ATTSYNC <block>`.
pub fn parse_attribute_sync(argument: &str) -> Option<AttributeSyncCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() != 1 || tokens[0].trim().is_empty() {
        return None;
    }
    Some(AttributeSyncCommand {
        block: tokens[0].clone(),
    })
}

/// Parse `BATTORDER <block> <tag>,<tag>,...` (comma- and/or whitespace-separated).
pub fn parse_attribute_order(argument: &str) -> Option<AttributeOrderCommand> {
    let tokens = split_quoted_tokens(argument)?;
    if tokens.len() < 2 {
        return None;
    }
    let mut tags = Vec::new();
    for part in tokens[1..].join(" ").split(',') {
        let tag = part.trim();
        if tag.is_empty() {
            return None;
        }
        tags.push(tag.to_string());
    }
    if tags.is_empty() {
        return None;
    }
    Some(AttributeOrderCommand {
        block: tokens[0].clone(),
        tags,
    })
}

/// Parse `ATTDISP NORMAL|ON|OFF`.
pub fn parse_attribute_display(argument: &str) -> Option<AttributeDisplayMode> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "NORMAL" | "N" => Some(AttributeDisplayMode::Normal),
        "ON" => Some(AttributeDisplayMode::On),
        "OFF" => Some(AttributeDisplayMode::Off),
        _ => None,
    }
}

/// The stable rejection reason for an attribute command line outside its grammar.
pub fn invalid_attribute_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`; type the documented `{selector}` \
         form and nothing was changed",
        text.trim()
    )
}

fn parse_number(text: &str) -> Option<f64> {
    let value = text.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

/// Whether a token starts an option rather than an optional coordinate.
fn is_option(token: &str) -> bool {
    matches!(
        token.to_ascii_uppercase().as_str(),
        "PROMPT"
            | "POSITION"
            | "HEIGHT"
            | "ROTATION"
            | "ORDER"
            | "WIDTH"
            | "WRAP"
            | "OBLIQUE"
            | "CONSTANT"
            | "INVISIBLE"
            | "VERIFY"
            | "PRESET"
            | "LOCK"
            | "LOCKPOSITION"
            | "ANNOTATIVE"
    )
}

use super::arguments::split_quoted_tokens;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_definition_grammar_parses_options_and_quotes() {
        let payload = parse_attribute_definition(
            "TITLE ROOM \"A 101\" PROMPT \"Room?\" POSITION 1 2 3 HEIGHT 2.5 ROTATION 90 ORDER 4 CONSTANT LOCK",
        )
        .expect("grammar");
        assert_eq!(payload.block, "TITLE");
        assert_eq!(payload.tag, "ROOM");
        assert_eq!(payload.default_value, "A 101");
        assert_eq!(payload.prompt, "Room?");
        assert_eq!(
            payload.position,
            Point3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
        assert_eq!(payload.height, 2.5);
        assert!((payload.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert_eq!(payload.order, Some(4));
        assert!(payload.flags.constant);
        assert!(payload.flags.locked_position);
        assert!(!payload.flags.invisible);
    }

    #[test]
    fn attribute_definition_grammar_rejects_malformed_lines() {
        for text in [
            "",
            "TITLE",
            "TITLE ROOM",
            "TITLE ROOM A HEIGHT 0",
            "TITLE ROOM A HEIGHT -1",
            "TITLE ROOM A POSITION 1",
            "TITLE ROOM A UNKNOWN",
            "TITLE ROOM A ORDER x",
        ] {
            assert_eq!(parse_attribute_definition(text), None, "{text}");
        }
    }

    #[test]
    fn attribute_edit_grammars_bind_the_selector_scope() {
        let edit = parse_attribute_edit("TITLE ROOM \"B 2\"", AttributeEditScope::Selection)
            .expect("simple edit");
        assert_eq!(edit.scope, AttributeEditScope::Selection);
        assert_eq!(edit.assignments[0].value, "B 2");
        let global = parse_attribute_edit("TITLE ROOM B", AttributeEditScope::AllInstances)
            .expect("global edit");
        assert_eq!(global.scope, AttributeEditScope::AllInstances);
        assert!(parse_attribute_edit("TITLE ROOM", AttributeEditScope::Selection).is_none());
        let enhanced = parse_eattedit("TITLE ROOM=B FLOOR=2 POSITION 1 2 HEIGHT 3 ROTATION 45")
            .expect("enhanced");
        assert_eq!(enhanced.assignments.len(), 2);
        assert_eq!(
            enhanced.position,
            Some(Point3 {
                x: 1.0,
                y: 2.0,
                z: 0.0
            })
        );
        assert_eq!(enhanced.height, Some(3.0));
        assert!(enhanced.rotation.is_some());
        assert!(parse_eattedit("TITLE ROOM").is_none());
    }

    #[test]
    fn attribute_report_grammars_and_display_modes() {
        assert_eq!(parse_attribute_sync("TITLE").expect("sync").block, "TITLE");
        assert_eq!(
            parse_attribute_order("TITLE B,A").expect("order").tags,
            vec!["B".to_string(), "A".to_string()]
        );
        assert_eq!(
            parse_attribute_order("TITLE B, A").expect("order").tags,
            vec!["B".to_string(), "A".to_string()]
        );
        assert_eq!(parse_attribute_order("TITLE"), None);
        assert_eq!(parse_attribute_order("TITLE B,,A"), None);
        assert_eq!(
            parse_attribute_display("normal"),
            Some(AttributeDisplayMode::Normal)
        );
        assert_eq!(
            parse_attribute_display("ON"),
            Some(AttributeDisplayMode::On)
        );
        assert_eq!(
            parse_attribute_display("off"),
            Some(AttributeDisplayMode::Off)
        );
        assert_eq!(parse_attribute_display("maybe"), None);
    }

    #[test]
    fn attribute_selectors_are_admitted_argument_rows() {
        for selector in [
            ATTDEF_SELECTOR,
            DASH_ATTDEF_SELECTOR,
            ATTEDIT_SELECTOR,
            DASH_ATTEDIT_SELECTOR,
            EATTEDIT_SELECTOR,
            ATTIPEDIT_SELECTOR,
            GATTE_SELECTOR,
            ATTSYNC_SELECTOR,
            ATTREDEF_SELECTOR,
            BATTORDER_SELECTOR,
            ATTDISP_SELECTOR,
        ] {
            assert!(
                crate::session::commands::ARGUMENT_SELECTORS.contains(&selector),
                "{selector}"
            );
        }
    }
}
