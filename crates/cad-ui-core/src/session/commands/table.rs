//! Typed-argument grammars for the table commands (`TABLE`, `-TABLE`,
//! `TABLEDIT`, `MATCHCELL`, `TABLESTYLE`, `TABLEEXPORT`).
//!
//! Every parser is pure and strict: it admits only the documented tokens and
//! never clamps, defaults or repairs. The command layer re-validates the payload
//! (dimension caps, positive sizes, finite origin, 1-based cell range against
//! the selected table) and is the authority for the named refusals.
//!
//! Full contracts: `crate::app_drawing::table_edit` (creation and cell text) and
//! `crate::app_drawing::table_ops` (merge/format/style/match), re-exported from
//! `cad-app`; `crate::session::table_export` owns `TABLEEXPORT`.
//!
//! * `TABLE <rows> <cols> <col_width> <row_height> <x> <y>` — an empty grid on
//!   the world XY plane. `rows`/`cols` are positive integers (the command caps
//!   them), the two sizes are positive decimals and the origin is a pair of
//!   signed decimals.
//! * `-TABLE …` — the same grammar with the command-line selector spelling.
//! * `TABLEDIT <row> <col> [text...]` — a **1-based** cell address followed by
//!   the text remainder. The remainder is the rest of the line with spaces
//!   preserved (only leading whitespace is dropped) and may be empty, which
//!   clears the cell.
//! * `TABLEDIT MERGE <row> <col> <height> <width>` — merge the range anchored at
//!   the 1-based cell.
//! * `TABLEDIT UNMERGE <row> <col>` — split the range covering the cell.
//! * `TABLEDIT FORMAT <row> <col> [ALIGN <TL|TC|TR|ML|MC|MR|BL|BC|BR>]
//!   [ROTATION <degrees>] [COLOR <aci|BYLAYER>] [FILL <aci|NONE>]
//!   [LINEWEIGHT <mm|BYLAYER>] [BORDERCOLOR <aci|BYLAYER>]
//!   [BORDER [ALL] <ON|OFF|SINGLE|DOUBLE> [<spacing>]]
//!   [MARGIN <l> <t> <r> <b>|INHERIT]` — set the cell's own text alignment,
//!   rotation, text colour, background fill, the four border edges'
//!   colour/lineweight/visibility/type (a spacing above zero is the `Double`
//!   border type) and the cell's `GRIDFORMAT` margins. A **border line type**
//!   stays refused by name (`tabledit.format_unsupported`): the only DXF group is
//!   an `LTYPE` handle, this writer emits no per-cell border line type, and this
//!   build's fragment expansion has no per-primitive dash source, so a stored
//!   value could neither draw nor round-trip.
//! * `TABLEDIT STYLE <name>` — assign the table's style (the `TABLESTYLE <name>`
//!   write through the `TABLEDIT` selector).
//! * `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> [<dst_row> <dst_col> …]`
//!   — copy the source cell's whole format onto every destination cell.
//! * `TABLESTYLE` — the read-only report; `TABLESTYLE [<name>] [DESC <text>]
//!   [MARGIN <h> <v>] [ROW <DATA|TITLE|HEADER>] [HEIGHT <h>] [TEXTSTYLE <name>]
//!   [ALIGN <code>] [COLOR <aci|BYLAYER>] [FILL <aci|NONE>]
//!   [LINEWEIGHT <mm|BYLAYER>] [BORDERCOLOR <aci|BYLAYER>]
//!   [BORDER <TOP|RIGHT|BOTTOM|LEFT|ALL> <ON|OFF|SINGLE|DOUBLE> [<spacing>]]
//!   [TITLE <ON|OFF>] [HEADER <ON|OFF>]` — the style write. `ROW` scopes the
//!   **following** row-scoped fields (`HEIGHT`, `TEXTSTYLE`, `ALIGN`, `COLOR`,
//!   `FILL`, `LINEWEIGHT`, `BORDERCOLOR`, `BORDER`) to one row type; without it
//!   they reach all three value sets.
//! * `TABLEEXPORT <path.csv>` — one CSV file; the whole remainder is the path
//!   (see `super::table_export`).

use cad_app::{
    TableCellAddress, TableCellBorderCommand, TableColorValue, TableCreateCommand,
    TableEditCommand, TableLineweightValue, TableMarginValue, TablePayload, TableRowStyleCommand,
    TableStyleCommand, TableStyleEdge, TableStyleRowScope,
};
use cad_domain::{Point3, TableCellAlignment, TableCellMargins, TableRowKind};

use super::arguments::{parse_positive_decimal, parse_signed_decimal};

/// `TABLE <rows> <cols> <col_width> <row_height> <x> <y>`.
pub const TABLE_SELECTOR: &str = "TABLE";

/// `-TABLE <rows> <cols> <col_width> <row_height> <x> <y>`.
pub const DASH_TABLE_SELECTOR: &str = "-TABLE";

/// `TABLEDIT <row> <col> [text...]` / `TABLEDIT MERGE|UNMERGE …`.
pub const TABLEDIT_SELECTOR: &str = "TABLEDIT";

/// `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> …`.
pub const MATCHCELL_SELECTOR: &str = "MATCHCELL";

/// `TABLESTYLE [<name>] [<field> <value> …]`.
pub const TABLESTYLE_SELECTOR: &str = "TABLESTYLE";

/// `TABLEEXPORT <path.csv>`.
pub const TABLEEXPORT_SELECTOR: &str = "TABLEEXPORT";

/// Parse the `TABLE` / `-TABLE` argument: six whitespace-separated tokens.
///
/// The row/column counts are positive `u32` integers (zero, a sign, a fraction
/// and a value beyond `u32` are refused here); the column width and row height
/// are positive decimals; the origin `x`/`y` are signed finite decimals and `z`
/// is fixed at `0` (the world XY work plane). Nothing is clamped: the command
/// owns the `1000 × 1000` / `100 000`-cell caps.
pub fn parse_table_create_argument(argument: &str) -> Option<TableCreateCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() != 6 {
        return None;
    }
    Some(TableCreateCommand {
        rows: parse_positive_integer(tokens[0])?,
        cols: parse_positive_integer(tokens[1])?,
        col_width: parse_positive_decimal(tokens[2])?,
        row_height: parse_positive_decimal(tokens[3])?,
        origin: Point3 {
            x: parse_signed_decimal(tokens[4])?,
            y: parse_signed_decimal(tokens[5])?,
            z: 0.0,
        },
    })
}

/// Parse the `TABLEDIT` argument: a 1-based cell address plus the text
/// remainder.
///
/// The first two tokens are positive integers (1-based, so `0` is refused here
/// as out of the address space). The remainder is the rest of the line after
/// those two tokens: internal and trailing spaces are preserved and only the
/// leading whitespace between the column and the text is dropped. No remainder
/// is an explicit empty string (clear the cell), never a default.
pub fn parse_tabledit_argument(argument: &str) -> Option<TableEditCommand> {
    let text = argument.trim_start();
    let (row, rest) = split_first_token(text)?;
    let (col, rest) = split_first_token(rest)?;
    Some(TableEditCommand {
        row: parse_positive_integer(row)?,
        col: parse_positive_integer(col)?,
        text: rest.trim_start().to_string(),
    })
}

/// Split off the first whitespace-delimited token (the remainder is returned
/// untrimmed).
fn split_first_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let token = &text[..end];
    if token.is_empty() {
        return None;
    }
    Some((token, &text[end..]))
}

/// One positive `u32` written with ASCII digits only.
fn parse_positive_integer(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|value| *value > 0)
}

/// Parse the `TABLEDIT MERGE <row> <col> <height> <width>` /
/// `TABLEDIT UNMERGE <row> <col>` / `TABLEDIT FORMAT <row> <col> …` forms.
///
/// `None` when the first token is not one of those words (or `STYLE`), so the
/// caller can fall through to the cell-text grammar. A recognised word with a
/// malformed remainder is also `None` (the caller then refuses the whole line by
/// name): nothing is repaired or defaulted.
///
/// `TABLEDIT FORMAT` takes a 1-based cell address followed by at least one of
/// `ALIGN <code>` (one of the nine DXF cell-alignment codes) and
/// `ROTATION <degrees>`; the rotation is converted to the model's radians here.
/// A recognised but unrepresentable field word (`COLOR`, `FILL`, `LINEWEIGHT`) is
/// not handled here — [`tabledit_format_refusal`] names it instead, so it can
/// never be silently dropped.
pub fn parse_tabledit_option_argument(argument: &str) -> Option<TablePayload> {
    let text = argument.trim_start();
    let (word, rest) = split_first_token(text)?;
    if word.eq_ignore_ascii_case("MERGE") {
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() != 4 {
            return None;
        }
        return Some(TablePayload::Merge {
            anchor: TableCellAddress {
                row: parse_positive_integer(tokens[0])?,
                col: parse_positive_integer(tokens[1])?,
            },
            height: parse_positive_integer(tokens[2])?,
            width: parse_positive_integer(tokens[3])?,
        });
    }
    if word.eq_ignore_ascii_case("FORMAT") {
        return parse_tabledit_format(rest);
    }
    if word.eq_ignore_ascii_case("STYLE") {
        // `TABLEDIT STYLE <name>`: assign the table's style by name, the same
        // write `TABLESTYLE <name>` performs (one snapshot, one transaction).
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() != 1 {
            return None;
        }
        return Some(TablePayload::Style(Box::new(TableStyleCommand {
            name: Some(tokens[0].to_string()),
            ..TableStyleCommand::default()
        })));
    }
    if word.eq_ignore_ascii_case("UNMERGE") {
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() != 2 {
            return None;
        }
        return Some(TablePayload::Unmerge {
            cell: TableCellAddress {
                row: parse_positive_integer(tokens[0])?,
                col: parse_positive_integer(tokens[1])?,
            },
        });
    }
    None
}

/// Parse `FORMAT <row> <col> [ALIGN <code>] [ROTATION <degrees>]
/// [COLOR <aci|BYLAYER>] [FILL <aci|NONE>] [LINEWEIGHT <mm|BYLAYER>]
/// [BORDERCOLOR <aci|BYLAYER>] [BORDER <ON|OFF|SINGLE|DOUBLE> [<spacing>]]
/// [MARGIN <l> <t> <r> <b>|INHERIT]`.
///
/// Every field may appear once, in any order; a repeated field, an unknown word
/// or a malformed value is `None` (the caller refuses the whole line by name,
/// never a partially applied write). The rotation is converted to the model's
/// radians here; an ACI index and a canonical lineweight are validated by the
/// command, which owns the drawing-independent value sets.
fn parse_tabledit_format(rest: &str) -> Option<TablePayload> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 4 {
        return None;
    }
    let cell = TableCellAddress {
        row: parse_positive_integer(tokens[0])?,
        col: parse_positive_integer(tokens[1])?,
    };
    let mut alignment = None;
    let mut rotation = None;
    let mut color = None;
    let mut fill = None;
    let mut lineweight = None;
    let mut border_color = None;
    let mut border = None;
    let mut margin = None;
    let mut index = 2;
    while index < tokens.len() {
        let word = tokens[index];
        index += 1;
        if word.eq_ignore_ascii_case("ALIGN") {
            if alignment.is_some() {
                return None;
            }
            alignment = Some(TableCellAlignment::from_label(tokens.get(index)?)?);
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("ROTATION") {
            if rotation.is_some() {
                return None;
            }
            // Degrees on the wire (what a user types and what AutoCAD shows);
            // the model stores radians like every other rotation.
            rotation = Some(parse_signed_decimal(tokens.get(index)?)?.to_radians());
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("COLOR") || word.eq_ignore_ascii_case("COLOUR") {
            if color.is_some() {
                return None;
            }
            color = Some(parse_cell_color(tokens.get(index)?, "BYLAYER")?);
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("FILL") {
            if fill.is_some() {
                return None;
            }
            fill = Some(parse_cell_color(tokens.get(index)?, "NONE")?);
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("LINEWEIGHT") {
            if lineweight.is_some() {
                return None;
            }
            let token = tokens.get(index)?;
            lineweight = Some(if token.eq_ignore_ascii_case("BYLAYER") {
                TableLineweightValue::ByLayer
            } else {
                TableLineweightValue::Millimetres(parse_non_negative_decimal(token)? as f32)
            });
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("BORDERCOLOR") || word.eq_ignore_ascii_case("BORDERCOLOUR") {
            if border_color.is_some() {
                return None;
            }
            border_color = Some(parse_cell_color(tokens.get(index)?, "BYLAYER")?);
            index += 1;
            continue;
        }
        if word.eq_ignore_ascii_case("BORDER") {
            if border.is_some() {
                return None;
            }
            let (command, consumed) = parse_cell_border(&tokens[index..])?;
            border = Some(command);
            index += consumed;
            continue;
        }
        if word.eq_ignore_ascii_case("MARGIN") {
            if margin.is_some() {
                return None;
            }
            let (value, consumed) = parse_cell_margin(&tokens[index..])?;
            margin = Some(value);
            index += consumed;
            continue;
        }
        return None;
    }
    if alignment.is_none()
        && rotation.is_none()
        && color.is_none()
        && fill.is_none()
        && lineweight.is_none()
        && border_color.is_none()
        && border.is_none()
        && margin.is_none()
    {
        return None;
    }
    Some(TablePayload::Format {
        cell,
        alignment,
        rotation,
        color,
        fill,
        lineweight,
        border_color,
        border,
        margin,
    })
}

/// One `TABLEDIT FORMAT … BORDER [ALL] <ON|OFF|SINGLE|DOUBLE> [<spacing>]`
/// assignment and how many tokens it consumed.
///
/// The four edges are the cell's own resolved border set (the typed grammar has
/// no per-edge selector here — that lives on the row-level `TABLESTYLE BORDER
/// <edge> …` form). `ON`/`OFF` set only the visibility, `SINGLE` also clears the
/// double-line spacing, and `DOUBLE` sets a spacing when one follows (otherwise
/// the edge keeps the spacing it already had).
fn parse_cell_border(tokens: &[&str]) -> Option<(TableCellBorderCommand, usize)> {
    // `BORDER ALL <state>` is the explicit spelling of the only scope this form
    // has; a single-edge word is refused by name (see
    // [`tabledit_format_refusal`]) rather than silently reaching all four edges.
    let (tokens, consumed_all) = match tokens.first() {
        Some(token) if token.eq_ignore_ascii_case("ALL") => (&tokens[1..], 1),
        _ => (tokens, 0),
    };
    let state = *tokens.first()?;
    if state.eq_ignore_ascii_case("ON") {
        return Some((
            TableCellBorderCommand {
                visible: Some(true),
                spacing: None,
            },
            consumed_all + 1,
        ));
    }
    if state.eq_ignore_ascii_case("OFF") {
        return Some((
            TableCellBorderCommand {
                visible: Some(false),
                spacing: None,
            },
            consumed_all + 1,
        ));
    }
    if state.eq_ignore_ascii_case("SINGLE") {
        return Some((
            TableCellBorderCommand {
                visible: Some(true),
                spacing: Some(0.0),
            },
            consumed_all + 1,
        ));
    }
    if state.eq_ignore_ascii_case("DOUBLE") {
        let spacing = match tokens.get(1) {
            Some(token) if !is_format_field(token) => Some(parse_non_negative_decimal(token)?),
            _ => None,
        };
        let consumed = usize::from(spacing.is_some()) + 1;
        return Some((
            TableCellBorderCommand {
                visible: Some(true),
                spacing,
            },
            consumed_all + consumed,
        ));
    }
    None
}

/// One `TABLEDIT FORMAT … MARGIN <left> <top> <right> <bottom> | INHERIT`
/// assignment and how many tokens it consumed.
fn parse_cell_margin(tokens: &[&str]) -> Option<(TableMarginValue, usize)> {
    let first = *tokens.first()?;
    if first.eq_ignore_ascii_case("INHERIT") {
        return Some((TableMarginValue::Inherit, 1));
    }
    let left = parse_non_negative_decimal(first)?;
    let top = parse_non_negative_decimal(tokens.get(1)?)?;
    let right = parse_non_negative_decimal(tokens.get(2)?)?;
    let bottom = parse_non_negative_decimal(tokens.get(3)?)?;
    Some((
        TableMarginValue::Values(TableCellMargins::new(left, top, right, bottom)),
        4,
    ))
}

/// The field words a `TABLEDIT FORMAT` line can carry, so a `DOUBLE` without an
/// explicit spacing does not swallow the next field.
fn is_format_field(token: &str) -> bool {
    const FIELDS: [&str; 10] = [
        "ALIGN",
        "ROTATION",
        "COLOR",
        "COLOUR",
        "FILL",
        "LINEWEIGHT",
        "BORDERCOLOR",
        "BORDERCOLOUR",
        "BORDER",
        "MARGIN",
    ];
    FIELDS.iter().any(|field| token.eq_ignore_ascii_case(field))
}

/// One table colour token: an ACI index `1..=255`, or the field's own symbolic
/// "no colour" word (`BYLAYER` for a text colour, `NONE` for a fill).
///
/// `0` and `256` are deliberately **not** accepted: they are the symbolic
/// `ByBlock`/`ByLayer` DXF values, not palette entries, and a table cell stores
/// the resolved colour rather than a second symbolic layer.
fn parse_cell_color(token: &str, symbolic: &str) -> Option<TableColorValue> {
    if token.eq_ignore_ascii_case(symbolic) {
        return Some(TableColorValue::None);
    }
    let index = parse_positive_integer(token)?;
    // `u8::try_from` already bounds the index to `1..=255` (the parser rejected
    // `0`), which is exactly the palette range a cell record can carry.
    u8::try_from(index).ok().map(TableColorValue::Aci)
}

/// The named refusal for a `TABLEDIT FORMAT` field this build cannot store.
///
/// The one recognised word is the DXF per-cell **border line type** (the border
/// record's line-type handle, `GRIDFORMAT` group `340`). It stays refused because
/// none of the three links can carry it:
///
/// * the value is an `LTYPE` **handle**, and the domain has no drawing-level
///   linetype table, so a stored name could not be turned into a dash pattern;
/// * the DXF writer emits no per-cell border line type on an `ACAD_TABLE`
///   entity (only on a `TABLECONTENT` object, whose `GRIDFORMAT` block still
///   carries a handle, not a name); and
/// * this build's fragment expansion replaces a fragment's own linetype with the
///   entity-resolved one, so a per-border dash source could not reach the
///   renderer even if it were stored.
///
/// It is refused with a stable code and that reason, never parsed and dropped.
/// Every other `TABLEDIT FORMAT` field (`COLOR`, `FILL`, `LINEWEIGHT`,
/// `BORDERCOLOR`, `BORDER`, `MARGIN`) is supported and therefore not refused here.
/// `None` when the line carries no unsupported field.
pub fn tabledit_format_refusal(argument: &str) -> Option<String> {
    let text = argument.trim_start();
    let (word, rest) = split_first_token(text)?;
    if !word.eq_ignore_ascii_case("FORMAT") {
        return None;
    }
    // A single-edge word after `BORDER` is a recognised spelling this form does
    // not have: its border fields reach all four edges by design, so accepting
    // it would silently widen the write. It is refused by name, with the shapes
    // that do work.
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if !token.eq_ignore_ascii_case("BORDER") {
            continue;
        }
        let Some(edge) = tokens.get(index + 1) else {
            continue;
        };
        let single_edge = ["TOP", "RIGHT", "BOTTOM", "LEFT"]
            .iter()
            .any(|name| edge.eq_ignore_ascii_case(name));
        if single_edge {
            return Some(format!(
                "tabledit.format_edge_unsupported: TABLEDIT FORMAT cannot set one border edge: \
                 its border fields reach all four edges of the cell. Use BORDER ALL \
                 <ON|OFF|SINGLE|DOUBLE> [<spacing>] here, or TABLESTYLE ROW <DATA|TITLE|HEADER> \
                 BORDER <TOP|RIGHT|BOTTOM|LEFT|ALL> ... for a single edge of a row's value set; \
                 nothing was changed (edge `{}` was not applied)",
                edge.to_uppercase()
            ));
        }
    }
    for token in rest.split_whitespace() {
        let (field, reason) = if token.eq_ignore_ascii_case("LINETYPE")
            || token.eq_ignore_ascii_case("BORDERLINETYPE")
        {
            (
                "LINETYPE",
                "the DXF group is an LTYPE handle (this build has no drawing-level linetype table \
                 to resolve a name into a dash pattern), this writer emits no per-cell border line \
                 type on the entity record, and the fragment expansion has no per-primitive dash \
                 source, so a stored value could neither be drawn nor round-trip",
            )
        } else {
            continue;
        };
        return Some(format!(
            "tabledit.format_unsupported: TABLEDIT FORMAT cannot set {field}: {reason}; use ALIGN \
             <TL|TC|TR|ML|MC|MR|BL|BC|BR>, ROTATION <degrees>, COLOR <aci|BYLAYER>, FILL <aci|NONE>, \
             LINEWEIGHT <mm|BYLAYER>, BORDERCOLOR <aci|BYLAYER>, \
             BORDER <ON|OFF|SINGLE|DOUBLE> [<spacing>] or MARGIN <l> <t> <r> <b>|INHERIT instead; \
             nothing was changed"
        ));
    }
    None
}

/// Parse `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> …`.
///
/// At least one destination pair (four tokens) and an **even** token count: the
/// list is `source (row, col)` followed by one or more `(row, col)` pairs. Every
/// number is a positive integer; the command validates each address against the
/// selected table.
pub fn parse_matchcell_argument(argument: &str) -> Option<TablePayload> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    if tokens.len() < 4 || !tokens.len().is_multiple_of(2) {
        return None;
    }
    let numbers: Vec<u32> = tokens
        .iter()
        .map(|token| parse_positive_integer(token))
        .collect::<Option<_>>()?;
    let source = TableCellAddress {
        row: numbers[0],
        col: numbers[1],
    };
    let targets = numbers[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| TableCellAddress {
            row: pair[0],
            col: pair[1],
        })
        .collect();
    Some(TablePayload::MatchCell { source, targets })
}

/// Parse `TABLESTYLE [<name>] [<field> <value> …]`.
///
/// `None` is the bare selector (the read-only report is a separate path). A line
/// that starts with a known **field word** begins the field list immediately (so
/// `TABLESTYLE HEIGHT 3` keeps the table's current style name); any other first
/// token is the style name. Fields:
///
/// * `DESC <text>` — the description; it takes the rest of the line, so it must
///   be the last field;
/// * `MARGIN <h> <v>` — the horizontal/vertical cell margins;
/// * `ROW <DATA|TITLE|HEADER>` — scope the **following** row-scoped fields to one
///   row value set. Without it they reach all three (the pre-row-style
///   behaviour). A second `ROW` starts a new group, so the typed order is kept;
/// * `HEIGHT <h>` — the row's text height;
/// * `TEXTSTYLE <name>` — the row's text style name;
/// * `ALIGN <TL|TC|TR|ML|MC|MR|BL|BC|BR>` — the row's cell alignment;
/// * `COLOR <aci|BYLAYER>` — the row's cell text colour; `FILL <aci|NONE>` — the
///   row's background fill; `LINEWEIGHT <mm|BYLAYER>` — the four border edges'
///   lineweight; `BORDERCOLOR <aci|BYLAYER>` — the four border edges' colour;
/// * `BORDER <TOP|RIGHT|BOTTOM|LEFT|ALL> <ON|OFF|SINGLE|DOUBLE> [<spacing>]` —
///   one edge (or all), visible/off, explicitly single (clears a stored
///   double-line spacing) or doubled with an optional spacing;
/// * `TITLE <ON|OFF>`, `HEADER <ON|OFF>` — the row suppression flags.
///
/// A sign, a fraction where an integer belongs, an unknown field word, an
/// unknown alignment code or a missing value is `None`: the whole line is
/// refused, never partially applied.
pub fn parse_tablestyle_argument(argument: &str) -> Option<TableStyleCommand> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let mut command = TableStyleCommand::default();
    let mut index = 0;
    if let Some(first) = tokens.first() {
        if !is_tablestyle_field(first)
            && !first.starts_with(['-', '+', '.'])
            && !is_numberish(first)
        {
            command.name = Some((*first).to_string());
            index = 1;
        }
    }
    while index < tokens.len() {
        let word = tokens[index];
        index += 1;
        if word.eq_ignore_ascii_case("DESC") {
            let description = tokens[index..].join(" ");
            index = tokens.len();
            if description.is_empty() {
                return None;
            }
            command.description = Some(description);
            continue;
        }
        if word.eq_ignore_ascii_case("MARGIN") {
            let horizontal = parse_non_negative_decimal(tokens.get(index)?)?;
            let vertical = parse_non_negative_decimal(tokens.get(index + 1)?)?;
            index += 2;
            command.horizontal_margin = Some(horizontal);
            command.vertical_margin = Some(vertical);
            continue;
        }
        if word.eq_ignore_ascii_case("ROW") {
            let kind = TableRowKind::from_label(tokens.get(index)?)?;
            index += 1;
            command.rows.push(TableRowStyleCommand {
                scope: Some(TableStyleRowScope::One(kind)),
                ..TableRowStyleCommand::default()
            });
            continue;
        }
        if word.eq_ignore_ascii_case("TITLE") || word.eq_ignore_ascii_case("HEADER") {
            let suppressed = !parse_on_off(tokens.get(index)?)?;
            index += 1;
            if word.eq_ignore_ascii_case("TITLE") {
                command.title_suppressed = Some(suppressed);
            } else {
                command.header_suppressed = Some(suppressed);
            }
            continue;
        }
        // Everything below is row-scoped: it lands in the group the last `ROW`
        // opened, or in one implicit "all rows" group.
        if word.eq_ignore_ascii_case("HEIGHT") {
            let height = parse_positive_decimal(tokens.get(index)?)?;
            index += 1;
            current_row_group(&mut command).text_height = Some(height);
            continue;
        }
        if word.eq_ignore_ascii_case("TEXTSTYLE") {
            let name = *tokens.get(index)?;
            index += 1;
            current_row_group(&mut command).text_style_name = Some(name.to_string());
            continue;
        }
        if word.eq_ignore_ascii_case("ALIGN") {
            let alignment = TableCellAlignment::from_label(tokens.get(index)?)?;
            index += 1;
            current_row_group(&mut command).alignment = Some(alignment);
            continue;
        }
        if word.eq_ignore_ascii_case("COLOR") || word.eq_ignore_ascii_case("COLOUR") {
            let choice = parse_cell_color(tokens.get(index)?, "BYLAYER")?;
            index += 1;
            current_row_group(&mut command).text_color = Some(choice);
            continue;
        }
        if word.eq_ignore_ascii_case("BORDERCOLOR") || word.eq_ignore_ascii_case("BORDERCOLOUR") {
            let choice = parse_cell_color(tokens.get(index)?, "BYLAYER")?;
            index += 1;
            current_row_group(&mut command).border_color = Some(choice);
            continue;
        }
        if word.eq_ignore_ascii_case("FILL") {
            let choice = parse_cell_color(tokens.get(index)?, "NONE")?;
            index += 1;
            current_row_group(&mut command).fill_color = Some(choice);
            continue;
        }
        if word.eq_ignore_ascii_case("LINEWEIGHT") {
            let token = tokens.get(index)?;
            index += 1;
            let choice = if token.eq_ignore_ascii_case("BYLAYER") {
                TableLineweightValue::ByLayer
            } else {
                TableLineweightValue::Millimetres(parse_non_negative_decimal(token)? as f32)
            };
            current_row_group(&mut command).border_lineweight = Some(choice);
            continue;
        }
        if word.eq_ignore_ascii_case("BORDER") {
            let edge_word = *tokens.get(index)?;
            index += 1;
            let edge = parse_style_edge(edge_word)?;
            let state = *tokens.get(index)?;
            index += 1;
            // `SINGLE` is the explicit "one line, no double-line spacing" state:
            // it is the only way to clear a spacing a `DOUBLE` write stored, which
            // is why the border type needs its own word rather than `ON`.
            let (visible, mut spacing) = if state.eq_ignore_ascii_case("ON") {
                (true, None)
            } else if state.eq_ignore_ascii_case("OFF") {
                (false, None)
            } else if state.eq_ignore_ascii_case("SINGLE") {
                (true, Some(0.0))
            } else if state.eq_ignore_ascii_case("DOUBLE") {
                (true, None)
            } else {
                return None;
            };
            if state.eq_ignore_ascii_case("DOUBLE") {
                // An explicit spacing may follow; without one the domain keeps
                // the spacing it already had for that edge.
                if let Some(token) = tokens.get(index) {
                    if !is_tablestyle_field(token) {
                        spacing = Some(parse_non_negative_decimal(token)?);
                        index += 1;
                    }
                }
            }
            // A `SINGLE` write stores "single line" for that edge; a spacing on
            // the same line would contradict it.
            if state.eq_ignore_ascii_case("SINGLE") {
                if let Some(token) = tokens.get(index) {
                    if !is_tablestyle_field(token) {
                        return None;
                    }
                }
            }
            current_row_group(&mut command)
                .borders
                .push((edge, visible, spacing));
            continue;
        }
        return None;
    }
    Some(command)
}

/// The row group a row-scoped `TABLESTYLE` field belongs to.
///
/// Only a `ROW` token opens a new group; every row-scoped field then extends the
/// last one, so `ROW TITLE HEIGHT 6 ALIGN TC` is one group. A line with no `ROW`
/// token at all gets one implicit group with no scope, which the command applies
/// to all three value sets.
fn current_row_group(command: &mut TableStyleCommand) -> &mut TableRowStyleCommand {
    if command.rows.is_empty() {
        command.rows.push(TableRowStyleCommand::default());
    }
    command.rows.last_mut().expect("the group is present")
}

/// The `TABLESTYLE` field words (so a style name never swallows one).
fn is_tablestyle_field(token: &str) -> bool {
    const FIELDS: [&str; 15] = [
        "DESC",
        "MARGIN",
        "ROW",
        "HEIGHT",
        "TEXTSTYLE",
        "ALIGN",
        "COLOR",
        "COLOUR",
        "FILL",
        "LINEWEIGHT",
        "BORDERCOLOR",
        "BORDERCOLOUR",
        "BORDER",
        "TITLE",
        "HEADER",
    ];
    FIELDS.iter().any(|field| token.eq_ignore_ascii_case(field))
}

/// Whether a token looks like a number or a signed/point value (never a style
/// name), so `TABLESTYLE -foo` is refused as a field list rather than accepted as
/// a style name.
fn is_numberish(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
}

fn parse_on_off(token: &str) -> Option<bool> {
    if token.eq_ignore_ascii_case("ON") || token == "1" {
        Some(true)
    } else if token.eq_ignore_ascii_case("OFF") || token == "0" {
        Some(false)
    } else {
        None
    }
}

fn parse_style_edge(token: &str) -> Option<TableStyleEdge> {
    if token.eq_ignore_ascii_case("TOP") {
        Some(TableStyleEdge::Top)
    } else if token.eq_ignore_ascii_case("RIGHT") {
        Some(TableStyleEdge::Right)
    } else if token.eq_ignore_ascii_case("BOTTOM") {
        Some(TableStyleEdge::Bottom)
    } else if token.eq_ignore_ascii_case("LEFT") {
        Some(TableStyleEdge::Left)
    } else if token.eq_ignore_ascii_case("ALL") {
        Some(TableStyleEdge::All)
    } else {
        None
    }
}

/// One finite non-negative plain decimal (a margin or a border spacing).
fn parse_non_negative_decimal(text: &str) -> Option<f64> {
    parse_signed_decimal(text).filter(|value| *value >= 0.0)
}

/// The stable refusal for a `MATCHCELL` argument outside the grammar.
pub fn invalid_matchcell_argument_reason(text: &str) -> String {
    format!(
        "matchcell.argument: `{}` is not `MATCHCELL <src_row> <src_col> <dst_row> <dst_col> \
         [<dst_row> <dst_col> ...]` (1-based positive integers, at least one destination cell); \
         nothing was changed",
        text.trim()
    )
}

/// The stable refusal for a `TABLESTYLE` argument outside the grammar.
pub fn invalid_tablestyle_argument_reason(text: &str) -> String {
    format!(
        "tablestyle.argument: `{}` is not `TABLESTYLE [<name>] [DESC <text>] [MARGIN <h> <v>] \
         [ROW <DATA|TITLE|HEADER>] [HEIGHT <h>] [TEXTSTYLE <name>] \
         [ALIGN <TL|TC|TR|ML|MC|MR|BL|BC|BR>] [COLOR <aci|BYLAYER>] [FILL <aci|NONE>] \
         [LINEWEIGHT <mm|BYLAYER>] [BORDERCOLOR <aci|BYLAYER>] [BORDER \
         <TOP|RIGHT|BOTTOM|LEFT|ALL> <ON|OFF|DOUBLE> [<spacing>]] [TITLE <ON|OFF>] \
         [HEADER <ON|OFF>]`; nothing was changed",
        text.trim()
    )
}
