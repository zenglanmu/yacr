//! The typed argument grammars of the interactive modify commands:
//! `PEDIT`, `JOIN`, `OFFSET`, `MIRROR`, `CHAMFER`, `FILLET`.
//!
//! These are **argument forms of canonical selectors**, never new commands and
//! never a second IPC. The current selection (and, for the two-point captures,
//! the captured picks) stays the input; these parsers only decide what the typed
//! option says. Every refusal is stable and names the accepted grammar, so an
//! unknown word is never silently ignored or defaulted.

use cad_app::{
    ChamferMethod, JoinMode, JoinOptions, PeditEditCommand, PeditEditOption, SelectionProperty,
};

/// `JOIN [MULTIPLE] [CLOSE]`: the AutoCAD 2026 join sub-options.
pub const JOIN_SELECTOR: &str = "JOIN";

/// `OFFSET <distance|THROUGH|ERASE|LAYER …>`: the OFFSET option line.
pub const OFFSET_SELECTOR: &str = "OFFSET";

/// `MIRROR TEXT 0|1` / `MIRROR ERASE YES|NO`: the MIRROR option line.
pub const MIRROR_SELECTOR: &str = "MIRROR";

/// `CHAMFER` option line (`DISTANCE`, `ANGLE`, `TRIM`, `METHOD`, `POLYLINE`).
pub const CHAMFER_SELECTOR: &str = "CHAMFER";

/// `FILLET` option line (`RADIUS`, `TRIM`, `NOTRIM`, `POLYLINE`).
pub const FILLET_SELECTOR: &str = "FILLET";

/// One parsed `OFFSET` option line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffsetArgument {
    /// `OFFSET <distance>` (or the explicit `DISTANCE <d>`): a fixed distance.
    Distance(f64),
    /// `OFFSET T` / `OFFSET THROUGH`: the pick point fixes the distance.
    Through,
    /// `OFFSET E` / `OFFSET ERASE`: toggle the erase-source option.
    ToggleErase,
    /// `OFFSET L` / `OFFSET LAYER CURRENT|SOURCE`: store the target layer choice.
    Layer { current: bool },
    /// `OFFSET M` / `OFFSET MULTIPLE`: the multiple-offset loop request.
    Multiple,
    /// `OFFSET U` / `OFFSET UNDO`: undo the last document step.
    Undo,
}

/// One parsed `MIRROR` option line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorArgument {
    /// `MIRROR TEXT 0|1` (`MIRRTEXT`): readable (`0`) or mirrored glyphs (`1`).
    MirrorText(bool),
    /// `MIRROR ERASE YES|NO`: erase the source objects after mirroring.
    Erase(bool),
}

/// One parsed `CHAMFER` option line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChamferArgument {
    /// `CHAMFER D|DISTANCE <d1> [d2]`.
    Distances { first: f64, second: Option<f64> },
    /// `CHAMFER A|ANGLE <distance> <angle>`.
    Angle { distance: f64, degrees: f64 },
    /// `CHAMFER T|TRIM` (`true`) or `N|NOTRIM` (`false`).
    Trim(bool),
    /// `CHAMFER M|METHOD DISTANCE|ANGLE`.
    Method(ChamferMethod),
    /// `CHAMFER P|POLYLINE`: apply to the one selected polyline.
    Polyline,
}

/// One parsed `FILLET` option line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilletArgument {
    /// `FILLET R|RADIUS <r>`.
    Radius(f64),
    /// `FILLET T|TRIM` (`true`) or `N|NOTRIM` (`false`).
    Trim(bool),
    /// `FILLET P|POLYLINE`: apply to the one selected polyline.
    Polyline,
}

/// Parse the `JOIN` argument: `MULTIPLE` and/or `CLOSE` in any order.
///
/// A bare `JOIN` never reaches this parser (it keeps `CommandPayload::None`,
/// i.e. source-object mode with no close). Duplicated keywords are refused
/// rather than collapsed, so a typo cannot silently change one option.
pub fn parse_join_argument(argument: &str) -> Option<JoinOptions> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let mut options = JoinOptions::default();
    let mut seen_close = false;
    let mut seen_multiple = false;
    for token in tokens {
        match token.to_ascii_uppercase().as_str() {
            "CLOSE" | "C" if !seen_close => {
                seen_close = true;
                options.close = true;
            }
            "MULTIPLE" | "M" if !seen_multiple => {
                seen_multiple = true;
                options.mode = JoinMode::Multiple;
            }
            _ => return None,
        }
    }
    Some(options)
}

/// Parse the full `PEDIT` option line into a [`PeditEditCommand`].
///
/// Accepted: `CLOSE|C`, `OPEN|O`, `JOIN|J`, `WIDTH|W <w>`, `EDIT|E`, `FIT|F`,
/// `SPLINE|S [0-99]` (no precision = AutoCAD's 8), `DECURVE|D`,
/// `LTYPE [GEN|ON|OFF]`, `REVERSE|R`, `UNDO|U`.
///
/// The `PEDITACCEPT` conversion answer is a prefix: `CONVERT Y|N <option>` (and
/// the compact `Y <option>` / `N <option>`). A lone `PEDIT Y` / `PEDIT CONVERT Y`
/// has no option to run and is refused, because an answer alone edits nothing.
pub fn parse_pedit_edit_argument(argument: &str) -> Option<PeditEditCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let mut index = 0;
    let mut convert = false;
    match tokens[0].to_ascii_uppercase().as_str() {
        "CONVERT" => {
            let answer = parse_yes_no(tokens.get(1)?)?;
            convert = answer;
            index = 2;
        }
        "Y" | "YES" => {
            convert = true;
            index = 1;
        }
        "N" | "NO" => {
            convert = false;
            index = 1;
        }
        _ => {}
    }
    let rest = &tokens[index..];
    if rest.is_empty() {
        return None;
    }
    let option = parse_pedit_option(rest)?;
    Some(PeditEditCommand { option, convert })
}

fn parse_pedit_option(tokens: &[&str]) -> Option<PeditEditOption> {
    let keyword = tokens[0].to_ascii_uppercase();
    let arity = tokens.len() - 1;
    match keyword.as_str() {
        "CLOSE" | "C" if arity == 0 => Some(PeditEditOption::Close),
        "OPEN" | "O" if arity == 0 => Some(PeditEditOption::Open),
        "JOIN" | "J" if arity == 0 => Some(PeditEditOption::Join),
        "EDIT" | "E" if arity == 0 => Some(PeditEditOption::EditVertex),
        "FIT" | "F" if arity == 0 => Some(PeditEditOption::Fit),
        "DECURVE" | "D" if arity == 0 => Some(PeditEditOption::Decurve),
        "REVERSE" | "R" if arity == 0 => Some(PeditEditOption::Reverse),
        "UNDO" | "U" if arity == 0 => Some(PeditEditOption::Undo),
        "WIDTH" | "W" if arity == 1 => {
            let width = super::parse_signed_decimal(tokens[1])?;
            Some(PeditEditOption::Width(width))
        }
        "SPLINE" | "S" => match arity {
            0 => Some(PeditEditOption::Spline { precision: 8 }),
            1 => {
                let precision = tokens[1];
                if precision.is_empty() || !precision.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                let precision: u8 = precision.parse().ok()?;
                (precision <= 99).then_some(PeditEditOption::Spline { precision })
            }
            _ => None,
        },
        "LTYPE" | "L" => match arity {
            0 => Some(PeditEditOption::LtypeGen { enabled: true }),
            1 => match tokens[1].to_ascii_uppercase().as_str() {
                "GEN" | "ON" => Some(PeditEditOption::LtypeGen { enabled: true }),
                "OFF" => Some(PeditEditOption::LtypeGen { enabled: false }),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Parse the `OFFSET` argument.
///
/// A bare numeric argument is the AutoCAD distance prompt (`OFFSET 2.5`); the
/// explicit `DISTANCE <d>` spelling is admitted too. `THROUGH`, `ERASE`,
/// `LAYER CURRENT|SOURCE`, `MULTIPLE` and `UNDO` are the documented options.
pub fn parse_offset_argument(argument: &str) -> Option<OffsetArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.len() {
        1 => match tokens[0].to_ascii_uppercase().as_str() {
            "T" | "THROUGH" => Some(OffsetArgument::Through),
            "E" | "ERASE" => Some(OffsetArgument::ToggleErase),
            "M" | "MULTIPLE" => Some(OffsetArgument::Multiple),
            "U" | "UNDO" => Some(OffsetArgument::Undo),
            _ => super::parse_positive_decimal(tokens[0]).map(OffsetArgument::Distance),
        },
        2 => match tokens[0].to_ascii_uppercase().as_str() {
            "L" | "LAYER" => match tokens[1].to_ascii_uppercase().as_str() {
                "CURRENT" | "C" => Some(OffsetArgument::Layer { current: true }),
                "SOURCE" | "S" => Some(OffsetArgument::Layer { current: false }),
                _ => None,
            },
            "D" | "DISTANCE" => {
                super::parse_positive_decimal(tokens[1]).map(OffsetArgument::Distance)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Parse the `MIRROR` argument: `TEXT 0|1` or `ERASE YES|NO`.
pub fn parse_mirror_argument(argument: &str) -> Option<MirrorArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() != 2 {
        return None;
    }
    match tokens[0].to_ascii_uppercase().as_str() {
        // `MIRRTEXT` is the AutoCAD sysvar; `MIRRORTEXT` is the spelled-out word.
        "TEXT" | "MIRRTEXT" | "MIRRORTEXT" => match tokens[1] {
            "0" => Some(MirrorArgument::MirrorText(false)),
            "1" => Some(MirrorArgument::MirrorText(true)),
            _ => None,
        },
        "ERASE" => parse_yes_no(tokens[1]).map(MirrorArgument::Erase),
        _ => None,
    }
}

/// Parse the `CHAMFER` argument.
pub fn parse_chamfer_argument(argument: &str) -> Option<ChamferArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let keyword = tokens[0].to_ascii_uppercase();
    match (keyword.as_str(), tokens.len() - 1) {
        ("D" | "DISTANCE", 1) => {
            super::parse_signed_decimal(tokens[1]).map(|first| ChamferArgument::Distances {
                first,
                second: None,
            })
        }
        ("D" | "DISTANCE", 2) => Some(ChamferArgument::Distances {
            first: super::parse_signed_decimal(tokens[1])?,
            second: Some(super::parse_signed_decimal(tokens[2])?),
        }),
        ("A" | "ANGLE", 2) => Some(ChamferArgument::Angle {
            distance: super::parse_signed_decimal(tokens[1])?,
            degrees: super::parse_signed_decimal(tokens[2])?,
        }),
        ("T" | "TRIM", 0) => Some(ChamferArgument::Trim(true)),
        ("N" | "NOTRIM", 0) => Some(ChamferArgument::Trim(false)),
        ("M" | "METHOD", 1) => match tokens[1].to_ascii_uppercase().as_str() {
            "DISTANCE" | "D" => Some(ChamferArgument::Method(ChamferMethod::Distance)),
            "ANGLE" | "A" => Some(ChamferArgument::Method(ChamferMethod::Angle)),
            _ => None,
        },
        ("P" | "POLYLINE", 0) => Some(ChamferArgument::Polyline),
        _ => None,
    }
}

/// Parse the `FILLET` argument.
pub fn parse_fillet_argument(argument: &str) -> Option<FilletArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let keyword = tokens[0].to_ascii_uppercase();
    match (keyword.as_str(), tokens.len() - 1) {
        ("R" | "RADIUS", 1) => super::parse_signed_decimal(tokens[1]).map(FilletArgument::Radius),
        ("T" | "TRIM", 0) => Some(FilletArgument::Trim(true)),
        ("N" | "NOTRIM", 0) => Some(FilletArgument::Trim(false)),
        ("P" | "POLYLINE", 0) => Some(FilletArgument::Polyline),
        _ => None,
    }
}

/// `YES`/`Y`/`1` and `NO`/`N`/`0`, ASCII case-insensitive.
fn parse_yes_no(token: &str) -> Option<bool> {
    match token.to_ascii_uppercase().as_str() {
        "YES" | "Y" | "1" | "ON" => Some(true),
        "NO" | "N" | "0" | "OFF" => Some(false),
        _ => None,
    }
}

/// The stable refusal for a `JOIN` argument outside the grammar.
pub fn invalid_join_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (JOIN needs MULTIPLE and/or CLOSE; a bare JOIN keeps the \
         source-object, no-close form)",
        text.trim()
    )
}

/// The stable refusal for a `PEDIT` argument outside the full option grammar.
pub fn invalid_pedit_edit_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (PEDIT needs CLOSE, OPEN, JOIN, WIDTH <w>, EDIT, FIT, \
         SPLINE [0-99], DECURVE, LTYPE [GEN|ON|OFF], REVERSE, UNDO, optionally prefixed by \
         CONVERT Y|N; an answer alone has no option to run)",
        text.trim()
    )
}

/// The stable refusal for an `OFFSET` argument outside the grammar.
pub fn invalid_offset_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (OFFSET needs a positive distance, THROUGH, ERASE, \
         LAYER CURRENT|SOURCE, MULTIPLE or UNDO)",
        text.trim()
    )
}

/// The stable refusal for an `MIRROR` argument outside the grammar.
pub fn invalid_mirror_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (MIRROR needs TEXT 0|1 or ERASE YES|NO; a bare MIRROR keeps \
         readable text and no erase)",
        text.trim()
    )
}

/// The stable refusal for a `CHAMFER` argument outside the grammar.
pub fn invalid_chamfer_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (CHAMFER needs DISTANCE <d1> [d2], ANGLE <distance> \
         <angle>, TRIM, NOTRIM, METHOD DISTANCE|ANGLE or POLYLINE)",
        text.trim()
    )
}

/// The stable refusal for a `FILLET` argument outside the grammar.
pub fn invalid_fillet_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (FILLET needs RADIUS <r>, TRIM, NOTRIM or POLYLINE)",
        text.trim()
    )
}

/// Parse `<PROPERTY> <operator> <value> [APPEND|EXCLUDE] [SELECTION]`.
///
/// This is the structured `QSELECT` form. `None` keeps the legacy
/// `<PROPERTY> <value>` line working (it has no operator token, so it never
/// reaches this parser's success path).
pub fn parse_qselect_request(argument: &str) -> Option<cad_app::QSelectRequest> {
    let tokens = split_tokens(argument)?;
    if tokens.len() < 3 {
        return None;
    }
    let property = SelectionProperty::parse(&tokens[0])?;
    let operator = parse_selection_operator(&tokens[1])?;
    let value = tokens[2].clone();
    if value.is_empty() {
        return None;
    }
    let mut apply = cad_app::SelectionApply::IncludeNew;
    let mut scope = cad_app::SelectionScope::EntireDrawing;
    let mut seen_apply = false;
    let mut seen_scope = false;
    for token in &tokens[3..] {
        match token.to_ascii_uppercase().as_str() {
            "APPEND" if !seen_apply => {
                seen_apply = true;
                apply = cad_app::SelectionApply::Append;
            }
            "EXCLUDE" if !seen_apply => {
                seen_apply = true;
                apply = cad_app::SelectionApply::ExcludeNew;
            }
            "SELECTION" if !seen_scope => {
                seen_scope = true;
                scope = cad_app::SelectionScope::CurrentSelection;
            }
            _ => return None,
        }
    }
    Some(cad_app::QSelectRequest {
        object_type: None,
        predicate: cad_app::SelectionPredicate {
            property,
            operator,
            value,
        },
        scope,
        apply,
    })
}

/// Parse `<LOGIC> <term> [...]` or one `PROPERTY OP VALUE` term into the
/// structured `FILTER` request.
///
/// The grammar is functional prefix notation with parenthesised groups:
/// `AND`, `OR`, `XOR`, `NOT` each take their members from the rest of the group,
/// and a term is `PROPERTY OP VALUE`. The value may be double-quoted to contain
/// spaces. A term with a missing operator returns `None`, which keeps the
/// legacy `PROPERTY VALUE` line on its existing path.
pub fn parse_filter_request(argument: &str) -> Option<cad_app::FilterRequest> {
    let tokens = split_tokens(argument)?;
    if tokens.is_empty() {
        return None;
    }
    let mut index = 0;
    let expression = parse_filter_expression(&tokens, &mut index)?;
    if index != tokens.len() {
        return None;
    }
    Some(cad_app::FilterRequest {
        expression,
        scope: cad_app::SelectionScope::EntireDrawing,
        apply: cad_app::SelectionApply::Append,
    })
}

fn parse_filter_expression(
    tokens: &[String],
    index: &mut usize,
) -> Option<cad_app::FilterExpression> {
    let token = tokens.get(*index)?;
    if token == "(" {
        *index += 1;
        let expression = parse_filter_expression(tokens, index)?;
        if tokens.get(*index)? != ")" {
            return None;
        }
        *index += 1;
        return Some(expression);
    }
    let keyword = token.to_ascii_uppercase();
    let logic = match keyword.as_str() {
        "AND" => Some(cad_app::FilterLogic::And),
        "OR" => Some(cad_app::FilterLogic::Or),
        "XOR" => Some(cad_app::FilterLogic::Xor),
        "NOT" => Some(cad_app::FilterLogic::Not),
        _ => None,
    };
    if let Some(logic) = logic {
        *index += 1;
        let mut members = Vec::new();
        match logic {
            cad_app::FilterLogic::And | cad_app::FilterLogic::Or => {
                while let Some(next) = tokens.get(*index) {
                    if next == ")" {
                        break;
                    }
                    members.push(parse_filter_expression(tokens, index)?);
                }
                if members.is_empty() {
                    return None;
                }
            }
            cad_app::FilterLogic::Xor => {
                for _ in 0..2 {
                    members.push(parse_filter_expression(tokens, index)?);
                }
            }
            cad_app::FilterLogic::Not => {
                members.push(parse_filter_expression(tokens, index)?);
            }
        }
        return Some(cad_app::FilterExpression::Group { logic, members });
    }
    // A term: `PROPERTY OP VALUE`.
    if tokens.len() < *index + 3 {
        return None;
    }
    let property = SelectionProperty::parse(&tokens[*index])?;
    let operator = parse_selection_operator(&tokens[*index + 1])?;
    let value = tokens[*index + 2].clone();
    if value.is_empty() {
        return None;
    }
    *index += 3;
    Some(cad_app::FilterExpression::Predicate(
        cad_app::SelectionPredicate {
            property,
            operator,
            value,
        },
    ))
}

/// Whether the argument has a `PROPERTY <operator> …` shape.
///
/// Used only to tell a **malformed structured** `QSELECT` line (which must be
/// refused rather than silently downgraded) from the legacy
/// `PROPERTY VALUE` line that has no operator token.
pub fn looks_like_structured_query(argument: &str) -> bool {
    let Some(tokens) = split_tokens(argument) else {
        return false;
    };
    tokens.len() >= 3
        && SelectionProperty::parse(&tokens[0]).is_some()
        && parse_selection_operator(&tokens[1]).is_some()
}

/// Whether the argument starts with a FILTER logic keyword or a group.
pub fn looks_like_filter_expression(argument: &str) -> bool {
    let Some(tokens) = split_tokens(argument) else {
        return false;
    };
    matches!(
        tokens.first().map(|token| token.to_ascii_uppercase()),
        Some(keyword)
            if matches!(keyword.as_str(), "AND" | "OR" | "XOR" | "NOT")
    ) || tokens.first().is_some_and(|token| token == "(")
}

fn parse_selection_operator(token: &str) -> Option<cad_app::SelectionOperator> {
    match token {
        "=" | "==" => Some(cad_app::SelectionOperator::Equals),
        "!=" | "<>" => Some(cad_app::SelectionOperator::NotEqual),
        ">" => Some(cad_app::SelectionOperator::GreaterThan),
        "<" => Some(cad_app::SelectionOperator::LessThan),
        ">=" => Some(cad_app::SelectionOperator::GreaterThanOrEqual),
        "<=" => Some(cad_app::SelectionOperator::LessThanOrEqual),
        "~" => Some(cad_app::SelectionOperator::WildcardMatch),
        "*" => Some(cad_app::SelectionOperator::SelectAll),
        _ => None,
    }
}

/// Split on whitespace, keeping double-quoted runs (including spaces) in one
/// token and treating `(` / `)` as standalone tokens. An unbalanced quote is
/// refused.
fn split_tokens(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for ch in text.trim().chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '(' | ')' if !quoted => {
                if started || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() && !quoted => {
                if started || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started || !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

/// The stable refusal for a structured `QSELECT` argument outside the grammar.
pub fn invalid_qselect_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (QSELECT needs `<PROPERTY> <OP> <VALUE> [APPEND|EXCLUDE] \
         [SELECTION]`, OP one of = != > < >= <= ~, or the legacy `<PROPERTY> <VALUE>`)",
        text.trim()
    )
}

/// The stable refusal for a structured `FILTER` argument outside the grammar.
pub fn invalid_filter_logic_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (FILTER needs `AND|OR|XOR|NOT <term> [...]` with \
         parenthesised groups and `PROPERTY OP VALUE` terms, or the legacy `PROPERTY VALUE`)",
        text.trim()
    )
}
