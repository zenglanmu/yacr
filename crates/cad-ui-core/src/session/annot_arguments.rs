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
//! the whole trimmed remainder and may contain spaces (or be empty: an empty
//! text is a content-less leader, and `CONTENT BLOCK` requires it).

use cad_app::{
    AnnotationBatchPayload, CenterArm, CenterEditPayload, CenterLineCommand, CenterMarkCommand,
    CommandId, CommandPayload, ExtendedCommandId, GroupCommand, GroupEditCommand,
    HatchOriginCommand, LeaderCommand, MLeaderContentSpec, TextFitCommand,
};

use super::diagnostics::cad_error_text;
use super::CadSession;

/// `CENTERMARK [ARM <distance>|<factor>x]`: the associative centre mark.
pub const CENTERMARK_SELECTOR: &str = "CENTERMARK";

/// `CENTERLINE [EXTENSION <distance>] [GAP <distance>]`: the midway centreline.
pub const CENTERLINE_SELECTOR: &str = "CENTERLINE";

/// `GROUP CREATE <name>|LIST|DELETE <name>`: the session-only group store.
pub const GROUP_SELECTOR: &str = "GROUP";

/// `UNGROUP <name>`: drop one session group, keep every entity.
pub const UNGROUP_SELECTOR: &str = "UNGROUP";

/// `GROUPEDIT <name> ADD|REMOVE|RENAME <arg>`.
pub const GROUPEDIT_SELECTOR: &str = "GROUPEDIT";

/// `QLEADER [CONTENT …] [ARROW ON|OFF] [MAXPOINTS <n>] <arrow x,y> <landing x,y> [text]`
/// (maps to MULTILEADER).
pub const QLEADER_SELECTOR: &str = "QLEADER";

/// `LEADER [CONTENT …] <arrow x,y> <landing x,y> [text]` (legacy name, maps to
/// MULTILEADER).
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
    let (name_token, rest) = split_name(trimmed);
    let name = group_name(name_token)?;
    if rest.is_empty() {
        return None;
    }
    let (verb, rest) = split_word(rest);
    if verb.eq_ignore_ascii_case("ADD") {
        return Some(GroupEditCommand::Add {
            name,
            entities: parse_entity_ids(rest)?,
        });
    }
    if verb.eq_ignore_ascii_case("REMOVE") {
        return Some(GroupEditCommand::Remove {
            name,
            entities: parse_entity_ids(rest)?,
        });
    }
    if verb.eq_ignore_ascii_case("RENAME") {
        let new_name = group_name(rest)?;
        return Some(GroupEditCommand::Rename { name, new_name });
    }
    None
}

/// Parse `QLEADER [CONTENT …] [ARROW ON|OFF] [MAXPOINTS <n>] <arrow x,y> <landing x,y> [text]`.
pub fn parse_qleader_argument(argument: &str) -> Option<LeaderCommand> {
    parse_leader_line(argument, true)
}

/// Parse `LEADER [CONTENT …] <arrow x,y> <landing x,y> [text]`.
pub fn parse_leader_argument(argument: &str) -> Option<LeaderCommand> {
    parse_leader_line(argument, false)
}

/// The shared legacy-leader grammar.
///
/// Both names take the annotation `CONTENT MTEXT|BLOCK <name>|TOLERANCE` prefix
/// exactly as `MLEADER` does, so a block or tolerance leader is reachable from
/// the legacy spellings too; the payload then rides the one `MLEADER` creation
/// path. `QLEADER` additionally takes `ARROW ON|OFF` (the created line's own
/// arrowhead override) and `MAXPOINTS <n>` (checked against the creation's two
/// vertices by the command layer). The text is the whole trimmed remainder and
/// may be empty: an empty text is a content-less leader (or, with `CONTENT
/// BLOCK`, the required absence of an annotation string).
///
/// Options are a prefix and each may appear once; a repeated option, an unknown
/// option word, a missing block name and an unparsable point are `None` (the
/// session turns that into the selector's stable `*.argument` refusal).
fn parse_leader_line(argument: &str, qleader: bool) -> Option<LeaderCommand> {
    let mut rest = argument.trim();
    let mut content: Option<MLeaderContentSpec> = None;
    let mut arrowhead: Option<bool> = None;
    let mut max_points: Option<u32> = None;
    loop {
        let (word, after_word) = split_word(rest);
        if word.eq_ignore_ascii_case("CONTENT") {
            if content.is_some() {
                return None;
            }
            let (value, after_value) = split_word(after_word);
            if value.eq_ignore_ascii_case("MTEXT") {
                content = Some(MLeaderContentSpec::MText);
                rest = after_value;
            } else if value.eq_ignore_ascii_case("TOLERANCE") {
                content = Some(MLeaderContentSpec::Tolerance);
                rest = after_value;
            } else if value.eq_ignore_ascii_case("BLOCK") {
                let (name, after_name) = split_word(after_value);
                if name.is_empty() {
                    return None;
                }
                content = Some(MLeaderContentSpec::Block(name.to_string()));
                rest = after_name;
            } else {
                return None;
            }
        } else if qleader && word.eq_ignore_ascii_case("ARROW") {
            if arrowhead.is_some() {
                return None;
            }
            let (value, after_value) = split_word(after_word);
            if value.eq_ignore_ascii_case("ON") {
                arrowhead = Some(true);
            } else if value.eq_ignore_ascii_case("OFF") {
                arrowhead = Some(false);
            } else {
                return None;
            }
            rest = after_value;
        } else if qleader && word.eq_ignore_ascii_case("MAXPOINTS") {
            if max_points.is_some() {
                return None;
            }
            let (value, after_value) = split_word(after_word);
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            max_points = Some(value.parse::<u32>().ok()?);
            rest = after_value;
        } else {
            // The first non-option token is the arrow X: options are a prefix.
            break;
        }
    }
    let (arrow, after_arrow) = split_word(rest);
    let (landing, text) = split_word(after_arrow);
    if arrow.is_empty() || landing.is_empty() {
        return None;
    }
    Some(LeaderCommand {
        arrow: parse_point(arrow)?,
        landing: parse_point(landing)?,
        text: text.to_string(),
        content,
        arrowhead,
        max_points,
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

/// One non-empty group name.
///
/// A name with spaces (legal in a source drawing and in `tables/groups.json`) is
/// addressed by double-quoting it; an unquoted name is exactly one token.
fn group_name(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('"') {
        let (name, tail) = rest.split_once('"')?;
        let name = name.trim();
        if name.is_empty() || !tail.trim().is_empty() {
            return None;
        }
        return Some(name.to_string());
    }
    if text.is_empty() || text.split_whitespace().count() != 1 {
        return None;
    }
    Some(text.to_string())
}

/// Split an optional double-quoted name from the rest of a `GROUPEDIT` line.
fn split_name(text: &str) -> (&str, &str) {
    if let Some(rest) = text.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let quoted_len = end + 2;
            return (&text[..quoted_len], text[quoted_len..].trim_start());
        }
    }
    split_word(text)
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

/// Parse `CENTERMARK [ARM] <distance>|<factor>x`.
///
/// The optional `ARM` keyword is accepted (`CENTERMARK ARM 5`); the value is a
/// positive finite decimal, optionally suffixed with `x` to mean a factor of the
/// source radius (`CENTERMARK 0.1x`). Anything else is `None`.
pub fn parse_centermark_argument(argument: &str) -> Option<CenterMarkCommand> {
    let mut tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() == 2 && tokens[0].eq_ignore_ascii_case("ARM") {
        tokens.remove(0);
    }
    if tokens.len() != 1 {
        return None;
    }
    let token = tokens[0];
    let (body, factor) = match token.strip_suffix(['x', 'X']) {
        Some(body) => (body, true),
        None => (token, false),
    };
    let value = super::commands::parse_signed_decimal(body)?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    Some(CenterMarkCommand {
        arm: Some(if factor {
            CenterArm::Factor(value)
        } else {
            CenterArm::Absolute(value)
        }),
    })
}

/// Parse `CENTERLINE [EXTENSION <distance>] [GAP <distance>]`.
///
/// The two keywords may appear in either order, at most once each, and each
/// takes a finite non-negative decimal. A bare `CENTERLINE` never reaches here
/// (it is the name-only default); an empty or unknown line is `None`.
pub fn parse_centerline_argument(argument: &str) -> Option<CenterLineCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() || !tokens.len().is_multiple_of(2) {
        return None;
    }
    let mut extension: Option<f64> = None;
    let mut gap: Option<f64> = None;
    for pair in tokens.as_chunks::<2>().0 {
        let [keyword, value] = *pair;
        let value = super::commands::parse_signed_decimal(value)?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        let slot = if keyword.eq_ignore_ascii_case("EXTENSION") {
            &mut extension
        } else if keyword.eq_ignore_ascii_case("GAP") {
            &mut gap
        } else {
            return None;
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    Some(CenterLineCommand {
        extension: extension.unwrap_or(0.0),
        gap: gap.unwrap_or(0.0),
    })
}

/// Stable refusal for a `CENTERMARK`/`CENTERLINE` argument outside the grammar.
pub fn invalid_center_argument_reason(selector: &str, text: &str) -> String {
    if selector == CENTERMARK_SELECTOR {
        format!(
            "unsupported CAD command: {} (CENTERMARK needs one positive arm length or a \
             `<factor>x` radius factor, e.g. `CENTERMARK 5` or `CENTERMARK 0.1x`)",
            text.trim()
        )
    } else {
        format!(
            "unsupported CAD command: {} (CENTERLINE accepts `EXTENSION <distance>` and \
             `GAP <distance>` in either order, e.g. `CENTERLINE EXTENSION 2 GAP 0.5`)",
            text.trim()
        )
    }
}

impl CadSession {
    /// Dispatch one admitted annotation-batch argument line.
    pub(super) fn annot_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        if selector == CENTERMARK_SELECTOR || selector == CENTERLINE_SELECTOR {
            let (id, payload) = if selector == CENTERMARK_SELECTOR {
                let Some(parsed) = parse_centermark_argument(argument) else {
                    return self.refuse_argument(invalid_center_argument_reason(selector, text));
                };
                (
                    CommandId::Extended(ExtendedCommandId::CenterMark),
                    CenterEditPayload::Mark(parsed),
                )
            } else {
                let Some(parsed) = parse_centerline_argument(argument) else {
                    return self.refuse_argument(invalid_center_argument_reason(selector, text));
                };
                (
                    CommandId::Extended(ExtendedCommandId::CenterLine),
                    CenterEditPayload::Line(parsed),
                )
            };
            return self.dispatch_typed_argument(selector, id, CommandPayload::Center(payload));
        }
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
