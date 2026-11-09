//! The typed-argument grammar of the 2026-11-07 selection/filter batch.
//!
//! These are **argument forms of canonical selectors**, not commands of their
//! own: `QSELECT TYPE LINE` is one `cad_command` line, and the session builds
//! the same [`cad_app::CommandPayload`] the IPC path would carry. Nothing here
//! matches entities; the application layer owns the matching semantics and
//! re-parses the property/value so a caller cannot bypass a refusal.
//!
//! * `QSELECT <PROPERTY> <VALUE>` / `FILTER <PROPERTY> <VALUE>` /
//!   `GETSEL <PROPERTY> <VALUE>`: the property is `TYPE`, `LAYER`, `COLOR` or
//!   `LINETYPE` (ASCII case-insensitive) and the value is the trimmed remainder
//!   (spaces allowed, must be non-empty). `GETSEL` accepts only `TYPE`/`LAYER`;
//!   the application names the refusal for the other two.
//! * `FIND <needle>` and `FIND <needle> => <replacement>`: the `=>` marker is the
//!   only split; the needle may not be empty. `-FIND <needle>` is the same
//!   report-only selector and refuses a `=>` replacement.
//!
//! A bare selector keeps the name-only funnel: the application refuses it with
//! the command's own needs-argument reason, never a fabricated default.

/// `QSELECT <PROPERTY> <VALUE>`: replace the selection with the filter match.
pub const QSELECT_SELECTOR: &str = "QSELECT";
/// `FILTER <PROPERTY> <VALUE>`: add the filter match to the selection.
pub const FILTER_SELECTOR: &str = "FILTER";
/// `GETSEL <LAYER|TYPE> <name>`: replace the selection (AutoCAD's form).
pub const GETSEL_SELECTOR: &str = "GETSEL";
/// `FIND <needle> [=> <replacement>]`: report, or replace in one transaction.
pub const FIND_SELECTOR: &str = "FIND";
/// `-FIND <needle>`: report-only text search.
pub const FIND_REPORT_SELECTOR: &str = "-FIND";
/// `SSX <criterion> ...`: the Express-Tool selection-filter builder.
pub const SSX_SELECTOR: &str = "SSX";
/// `AUDIT [YES|NO|LOG]`: the drawing audit and its `Fix any errors detected?`
/// answer (bare = the documented default **No**).
pub const AUDIT_SELECTOR: &str = "AUDIT";

/// Parse one typed `SSX` criterion line.
///
/// Grammar (keywords ASCII case-insensitive, one value per criterion):
///
/// | line | request |
/// | --- | --- |
/// | `BLOCK <name>` / `COLOR <n>` / `ENTITY <type>` / `LAYER <name>` / `LTYPE <name>` / `STYLE <name>` | overwrite that criterion |
/// | `FLAG` | the unmodelled entity-flags criterion (applying refuses by name) |
/// | `THICKNESS <v>` / `VECTOR <x> <y> <z>` | the unmodelled criteria (applying refuses by name) |
/// | `PICK` | seed every criterion from the current selection and apply |
/// | `CLEAR <criterion>` | remove that criterion and apply |
///
/// A name value is the trimmed remainder (spaces allowed). `None` is the local
/// refusal; the application owns the semantic refusals (empty filter,
/// unmodelled criterion).
pub fn parse_ssx_argument(argument: &str) -> Option<cad_app::SsxRequest> {
    use cad_app::{SsxCriterion, SsxCriterionKind, SsxRequest};

    let trimmed = argument.trim();
    let split = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let keyword = trimmed[..split].to_ascii_uppercase();
    let rest = trimmed[split..].trim();
    let name = |rest: &str| (!rest.is_empty()).then(|| rest.to_string());
    let criterion = match keyword.as_str() {
        "BLOCK" => SsxCriterion::Block(name(rest)?),
        "COLOR" => SsxCriterion::Color(name(rest)?),
        "ENTITY" => SsxCriterion::Entity(name(rest)?),
        "LAYER" => SsxCriterion::Layer(name(rest)?),
        "LTYPE" => SsxCriterion::LType(name(rest)?),
        "STYLE" => SsxCriterion::Style(name(rest)?),
        "FLAG" if rest.is_empty() => SsxCriterion::Flag,
        "THICKNESS" => {
            let value: f64 = rest.parse().ok()?;
            value
                .is_finite()
                .then_some(SsxCriterion::Thickness(value))?
        }
        "VECTOR" => {
            let mut parts = rest.split_whitespace();
            let x: f64 = parts.next()?.parse().ok()?;
            let y: f64 = parts.next()?.parse().ok()?;
            let z: f64 = parts.next()?.parse().ok()?;
            if parts.next().is_some() || ![x, y, z].iter().all(|value| value.is_finite()) {
                return None;
            }
            SsxCriterion::Vector(cad_domain::Point3 { x, y, z })
        }
        "PICK" if rest.is_empty() => return Some(SsxRequest::Pick),
        "CLEAR" => {
            let kind = match rest.to_ascii_uppercase().as_str() {
                "BLOCK" => SsxCriterionKind::Block,
                "COLOR" => SsxCriterionKind::Color,
                "ENTITY" => SsxCriterionKind::Entity,
                "FLAG" => SsxCriterionKind::Flag,
                "LAYER" => SsxCriterionKind::Layer,
                "LTYPE" => SsxCriterionKind::LType,
                "STYLE" => SsxCriterionKind::Style,
                "THICKNESS" => SsxCriterionKind::Thickness,
                "VECTOR" => SsxCriterionKind::Vector,
                _ => return None,
            };
            return Some(SsxRequest::Clear(kind));
        }
        _ => return None,
    };
    Some(SsxRequest::Set(criterion))
}

/// The stable refusal for an `SSX` argument outside the grammar.
pub fn invalid_ssx_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SSX needs \
         `<BLOCK|COLOR|ENTITY|FLAG|LAYER|LTYPE|STYLE|THICKNESS|VECTOR|PICK|CLEAR> ...`)",
        text.trim()
    )
}

/// Parse the typed `AUDIT` answer.
///
/// `YES`/`Y` = attempt the provable repairs, `NO`/`N` = detect only (the bare
/// selector's documented default), `LOG` = request the `AUDITCTL` `.adt` log
/// (refused by the application: host file I/O).
pub fn parse_audit_argument(argument: &str) -> Option<cad_app::AuditRequest> {
    match argument.trim().to_ascii_uppercase().as_str() {
        "YES" | "Y" => Some(cad_app::AuditRequest {
            fix: true,
            auditctl_log: false,
        }),
        "NO" | "N" => Some(cad_app::AuditRequest {
            fix: false,
            auditctl_log: false,
        }),
        "LOG" => Some(cad_app::AuditRequest {
            fix: false,
            auditctl_log: true,
        }),
        _ => None,
    }
}

/// The stable refusal for an `AUDIT` argument outside the grammar.
pub fn invalid_audit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (AUDIT needs `YES`, `NO` or `LOG`)",
        text.trim()
    )
}

/// Parse `<PROPERTY> <VALUE>` for the three filter selectors.
///
/// `None` when there is no property/value split, the property is not one of the
/// four words, or the value is empty. The property is returned canonically
/// upper-cased; the value is trimmed and keeps its internal spaces (`TYPE
/// AcDbLine` and `LAYER My Wall` both parse).
pub fn parse_property_filter(argument: &str) -> Option<(String, String)> {
    let trimmed = argument.trim();
    let split = trimmed.find([' ', '\t'])?;
    let (property, rest) = trimmed.split_at(split);
    let value = rest.trim();
    if property.is_empty() || value.is_empty() {
        return None;
    }
    let property = property.to_ascii_uppercase();
    matches!(property.as_str(), "TYPE" | "LAYER" | "COLOR" | "LINETYPE")
        .then(|| (property, value.to_string()))
}

/// Parse the `FIND` argument: `<needle>` or `<needle> => <replacement>`.
///
/// The needle is trimmed and must be non-empty. A `=>` marker (the first one)
/// splits the line; the replacement is the trimmed remainder and may be empty
/// (a plain delete), because the marker itself is explicit. There is no escape
/// syntax: a needle that contains `=>` cannot be expressed here.
pub fn parse_find_argument(argument: &str) -> Option<(String, Option<String>)> {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.split_once("=>") {
        Some((needle, replacement)) => {
            let needle = needle.trim();
            (!needle.is_empty()).then(|| (needle.to_string(), Some(replacement.trim().to_string())))
        }
        None => Some((trimmed.to_string(), None)),
    }
}

/// The stable refusal for a filter argument outside the grammar.
pub fn invalid_filter_argument_reason(text: &str, selector: &str) -> String {
    format!(
        "unsupported CAD command: {} ({selector} needs `<TYPE|LAYER|COLOR|LINETYPE> <value>`)",
        text.trim()
    )
}

/// The stable refusal for a `GETSEL` property outside its narrower grammar.
pub fn invalid_getsel_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (GETSEL needs `<LAYER|TYPE> <value>`)",
        text.trim()
    )
}

/// The stable refusal for a find argument outside the grammar.
///
/// `selector` is `FIND` or `-FIND`; the `-FIND` wording names the report-only
/// rule so a `=>` is refused by name instead of being silently ignored.
pub fn invalid_find_argument_reason(text: &str, selector: &str) -> String {
    if selector == FIND_REPORT_SELECTOR {
        format!(
            "unsupported CAD command: {} (-FIND needs one non-empty search string and never \
             replaces; use FIND <needle> => <text> to replace)",
            text.trim()
        )
    } else {
        format!(
            "unsupported CAD command: {} (FIND needs `<needle>` or `<needle> => <replacement>`)",
            text.trim()
        )
    }
}
