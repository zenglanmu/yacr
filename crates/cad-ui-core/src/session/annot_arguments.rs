//! Typed-argument forms of the 2026-11-07 annotation/group batch:
//! `GROUP`, `-GROUP`, `UNGROUP`, `GROUPEDIT`, `QLEADER`, `LEADER`, `TEXTFIT`
//! and `HATCHSETORIGIN`.
//!
//! Each is an **argument form of its base selector**, not a second command:
//! `-GROUP` normalises onto `GROUP` before this module sees it, and every other
//! selector keeps its one [`CommandId`]. The selection (where relevant) is still
//! the input; this module only parses the typed argument and dispatches the
//! matching [`CommandPayload::Annotation`].
//!
//! A bare selector keeps the name-only funnel. For `GROUP` that funnel is the
//! read-only list; for `UNGROUP`/`GROUPEDIT`/`TEXTFIT`/`HATCHSETORIGIN` the
//! application refuses it with `*.needs_*`, never a fabricated argument. An
//! argument outside a grammar is refused before IPC with
//! [`invalid_annot_argument_reason`] and changes nothing.
//!
//! Group names are one non-whitespace token in this build (a space-separated
//! name would make `GROUPEDIT <name> ADD` ambiguous). `LEADER`/`QLEADER` text is
//! the whole trimmed remainder and may contain spaces.

use cad_app::{
    AnnotationBatchPayload, CommandId, CommandPayload, GroupCommand, GroupEditCommand,
    HatchOriginCommand, LeaderCommand, TextFitCommand,
};

use super::diagnostics::cad_error_text;
use super::CadSession;

/// `GROUP CREATE <name>|LIST|DELETE <name>`: the session-only group store.
pub const GROUP_SELECTOR: &str = "GROUP";

/// `UNGROUP <name>`: drop one session group, keep every entity.
pub const UNGROUP_SELECTOR: &str = "UNGROUP";

/// `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
pub const GROUPEDIT_SELECTOR: &str = "GROUPEDIT";

/// `QLEADER <arrow x,y> <landing x,y> <text>` (maps to MULTILEADER).
pub const QLEADER_SELECTOR: &str = "QLEADER";

/// `LEADER <arrow x,y> <landing x,y> <text>` (legacy name, maps to MULTILEADER).
pub const LEADER_SELECTOR: &str = "LEADER";

/// `TEXTFIT <target width>`: stretch only the horizontal width factor under the
/// batch's estimated width.
pub const TEXTFIT_SELECTOR: &str = "TEXTFIT";

/// `HATCHSETORIGIN <x> <y>`: modelled so the missing-field refusal can be named.
pub const HATCHSETORIGIN_SELECTOR: &str = "HATCHSETORIGIN";

/// Parse `GROUP CREATE <name>|LIST|DELETE <name>`.
pub fn parse_group_argument(argument: &str) -> Option<GroupCommand> {
    let trimmed = argument.trim();
    let (word, rest) = split_word(trimmed);
    if word.eq_ignore_ascii_case("LIST") && rest.is_empty() {
        return Some(GroupCommand::List);
    }
    if word.eq_ignore_ascii_case("CREATE") {
        return group_name(rest).map(GroupCommand::Create);
    }
    if word.eq_ignore_ascii_case("DELETE") {
        return group_name(rest).map(GroupCommand::Delete);
    }
    None
}

/// Parse `UNGROUP <name>`.
pub fn parse_ungroup_argument(argument: &str) -> Option<String> {
    group_name(argument)
}

/// Parse `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
pub fn parse_groupedit_argument(argument: &str) -> Option<GroupEditCommand> {
    let trimmed = argument.trim();
    let (name, rest) = split_word(trimmed);
    if name.is_empty() || rest.is_empty() {
        return None;
    }
    let (verb, rest) = split_word(rest);
    if verb.eq_ignore_ascii_case("ADD") {
        return Some(GroupEditCommand::Add {
            name: name.to_string(),
            entities: parse_entity_ids(rest)?,
        });
    }
    if verb.eq_ignore_ascii_case("REMOVE") {
        return Some(GroupEditCommand::Remove {
            name: name.to_string(),
            entities: parse_entity_ids(rest)?,
        });
    }
    if verb.eq_ignore_ascii_case("RENAME") {
        let new_name = group_name(rest)?;
        return Some(GroupEditCommand::Rename {
            name: name.to_string(),
            new_name,
        });
    }
    None
}

/// Parse `QLEADER <arrow x,y> <landing x,y> <text>`.
pub fn parse_qleader_argument(argument: &str) -> Option<LeaderCommand> {
    parse_leader_argument(argument)
}

/// Parse `LEADER <arrow x,y> <landing x,y> <text>`.
pub fn parse_leader_argument(argument: &str) -> Option<LeaderCommand> {
    let trimmed = argument.trim();
    let (arrow, rest) = split_word(trimmed);
    let (landing, text) = split_word(rest);
    if arrow.is_empty() || landing.is_empty() || text.is_empty() {
        return None;
    }
    Some(LeaderCommand {
        arrow: parse_point(arrow)?,
        landing: parse_point(landing)?,
        text: text.to_string(),
    })
}

/// Parse `TEXTFIT <target width>`.
///
/// The value is one signed plain decimal; finiteness and positivity are the
/// application's named `textfit.bad_width` refusal (so a `0`/negative input
/// names the real rule instead of disappearing in the grammar).
pub fn parse_textfit_argument(argument: &str) -> Option<TextFitCommand> {
    Some(TextFitCommand {
        target_width: super::commands::parse_signed_decimal(argument.trim())?,
    })
}

/// Parse `HATCHSETORIGIN <x> <y>`.
pub fn parse_hatchsetorigin_argument(argument: &str) -> Option<HatchOriginCommand> {
    let mut parts = argument.split_whitespace();
    let x = parts.next()?;
    let y = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(HatchOriginCommand {
        x: super::commands::parse_signed_decimal(x)?,
        y: super::commands::parse_signed_decimal(y)?,
    })
}

/// The stable refusal for an argument outside a selector's grammar.
pub fn invalid_annot_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed form is the documented \
         {selector} grammar (see the command contract); nothing was changed",
        text.trim()
    )
}

/// Split the first whitespace-delimited word from the trimmed remainder.
fn split_word(text: &str) -> (&str, &str) {
    match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim_start()),
        None => (text, ""),
    }
}

/// One non-empty group name with no internal whitespace.
fn group_name(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.split_whitespace().count() != 1 {
        return None;
    }
    Some(text.to_string())
}

/// Empty list, or comma/whitespace separated `u128` entity ids.
fn parse_entity_ids(text: &str) -> Option<Vec<u128>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(Vec::new());
    }
    let mut ids = Vec::new();
    for token in text.split([',', ' ', '\t']).filter(|part| !part.is_empty()) {
        ids.push(token.parse::<u128>().ok()?);
    }
    (!ids.is_empty()).then_some(ids)
}

/// `x,y` with two signed plain decimals; `z` is the flat work plane's 0.
fn parse_point(token: &str) -> Option<cad_domain::Point3> {
    let (x, y) = token.split_once(',')?;
    if y.contains(',') {
        return None;
    }
    Some(cad_domain::Point3 {
        x: super::commands::parse_signed_decimal(x)?,
        y: super::commands::parse_signed_decimal(y)?,
        z: 0.0,
    })
}

impl CadSession {
    /// Dispatch one admitted annotation-batch argument line.
    pub(super) fn annot_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            GROUP_SELECTOR => (
                CommandId::Group,
                parse_group_argument(argument).map(AnnotationBatchPayload::Group),
            ),
            UNGROUP_SELECTOR => (
                CommandId::Ungroup,
                parse_ungroup_argument(argument).map(AnnotationBatchPayload::Ungroup),
            ),
            GROUPEDIT_SELECTOR => (
                CommandId::GroupEdit,
                parse_groupedit_argument(argument).map(AnnotationBatchPayload::GroupEdit),
            ),
            QLEADER_SELECTOR => (
                CommandId::QLeader,
                parse_qleader_argument(argument).map(AnnotationBatchPayload::Leader),
            ),
            LEADER_SELECTOR => (
                CommandId::Leader,
                parse_leader_argument(argument).map(AnnotationBatchPayload::Leader),
            ),
            TEXTFIT_SELECTOR => (
                CommandId::TextFit,
                parse_textfit_argument(argument).map(AnnotationBatchPayload::TextFit),
            ),
            _ => (
                CommandId::HatchSetOrigin,
                parse_hatchsetorigin_argument(argument).map(AnnotationBatchPayload::HatchSetOrigin),
            ),
        };
        let Some(payload) = payload else {
            return self.refuse_argument(invalid_annot_argument_reason(selector, text));
        };
        self.core.record_command(selector);
        match self
            .core
            .command(id, CommandPayload::Annotation(Box::new(payload)))
        {
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
