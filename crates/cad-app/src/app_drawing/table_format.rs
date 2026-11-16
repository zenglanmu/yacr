//! The table commands' shared **labels**: how a report/diagnostic spells a
//! table style, a border set, a colour and a lineweight.
//!
//! Split out of [`super::table_ops`] for the code-size ceiling; the spelling is
//! what the user reads, so the labels live together and every command that
//! reports a table (the `TABLESTYLE` report, `TABLEDIT FORMAT`, `MATCHCELL`)
//! uses one set.

use cad_domain::{TableRowKind, TableStyleValues};

/// How the report and the write message summarise the three row value sets.
pub(super) fn row_sets_label(style: &TableStyleValues) -> String {
    let data = style.data_row();
    let mut parts = Vec::with_capacity(3);
    for kind in [
        TableRowKind::Data,
        TableRowKind::Title,
        TableRowKind::Header,
    ] {
        let row = style.row_style(kind);
        let linked = if kind != TableRowKind::Data && row == data {
            " = data row"
        } else {
            ""
        };
        parts.push(format!(
            "{} row: text height {}, text style `{}`, alignment {}, borders {}, text colour {}, \
             fill {}{linked}",
            kind.label(),
            row.text_height,
            row.text_style_name,
            row.alignment.label(),
            borders_label(&row.borders),
            color_label(row.text_color, "BYLAYER"),
            color_label(row.fill_color, "NONE")
        ));
    }
    parts.join("; ")
}

/// `top/right/bottom/left` edge labels, in one short form.
pub(super) fn borders_label(borders: &cad_domain::TableCellBorders) -> String {
    format!(
        "{}/{}/{}/{}",
        edge_label(borders.top.visible, borders.top.double_spacing),
        edge_label(borders.right.visible, borders.right.double_spacing),
        edge_label(borders.bottom.visible, borders.bottom.double_spacing),
        edge_label(borders.left.visible, borders.left.double_spacing)
    )
}

pub(super) fn suppression_label(suppressed: bool) -> &'static str {
    if suppressed {
        "suppressed"
    } else {
        "shown"
    }
}

/// The style fields this build does not model (named, never implied).
pub(super) const STYLE_GAPS: &str =
    "; a per-border line type is not modelled, a stored border lineweight is exported but drawn \
     one pixel wide, a per-row double-line spacing reaches the cells (and round-trips through their \
     GRIDFORMAT blocks) rather than the TABLESTYLE row record, and the style's annotative flag is \
     not stored";

/// The `tablestyle.report` gap sentence's first character is joined mid-line, so
/// the constant starts there rather than with a space of its own.
pub(super) fn edge_label(visible: bool, double_spacing: f64) -> String {
    if !visible {
        return "hidden".to_string();
    }
    if double_spacing > 0.0 {
        format!("double({double_spacing})")
    } else {
        "single".to_string()
    }
}

/// `BYLAYER`/`NONE` for an absent override, `#RRGGBB` for a stored colour.
pub(super) fn color_label(color: Option<[u8; 3]>, absent: &str) -> String {
    match color {
        None => absent.to_string(),
        Some([r, g, b]) => format!("#{r:02X}{g:02X}{b:02X}"),
    }
}

/// The four edges' colours, in top/right/bottom/left order.
pub(super) fn borders_color_label(borders: &cad_domain::TableCellBorders) -> String {
    format!(
        "{}/{}/{}/{}",
        color_label(borders.top.color, "BYLAYER"),
        color_label(borders.right.color, "BYLAYER"),
        color_label(borders.bottom.color, "BYLAYER"),
        color_label(borders.left.color, "BYLAYER")
    )
}

/// The four edges' lineweights, in top/right/bottom/left order.
pub(super) fn borders_lineweight_label(borders: &cad_domain::TableCellBorders) -> String {
    let label = |weight: Option<u16>| match weight {
        None => "BYLAYER".to_string(),
        Some(hundredths) => format!("{:.2}mm", f64::from(hundredths) / 100.0),
    };
    format!(
        "{}/{}/{}/{}",
        label(borders.top.lineweight_hundredths),
        label(borders.right.lineweight_hundredths),
        label(borders.bottom.lineweight_hundredths),
        label(borders.left.lineweight_hundredths)
    )
}
