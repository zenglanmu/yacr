//! The paper-space **text scale** of `PSBSCALE`/`PSTSCALE`: which stored
//! lengths multiply when one annotation object is scaled to paper size.
//!
//! [`scaled_annotation_text`] is the single table of scalable text kinds, so the
//! two commands cannot disagree about what "text" means. Every other kind
//! returns `None`, and the owning command then reports its own `*.not_text` /
//! `*.not_block` refusal instead of silently ignoring the selection.
//!
//! Both compound objects are rebuilt through the domain's import constructor,
//! which re-derives the display from the definition — a stored display that
//! disagrees with the scaled text is not representable.
//!
//! Declared from `app_layout_paper` with an explicit `#[path]` so that module
//! stays inside the 1000-line code gate.

use super::*;

/// Scale the stored text of one paper-space annotation by `factor`.
///
/// `Ok(None)` means the kind carries no text this build can scale, so the caller
/// reports its own `*.not_text` / `*.not_block` refusal. Three kinds are
/// supported, and each keeps every non-text field bit-for-bit:
///
/// * `Text` (TEXT/MTEXT): `height` and the MTEXT `wrap_width` multiply; the
///   position, rotation, alignment, width factor and line spacing are unchanged.
/// * `MultiLeader`: the annotation text height and its width boundary multiply,
///   and the style snapshot's `text_height` multiplies with them so a masked and
///   an unmasked read cannot disagree after the scale. Roots, lines, arrowheads
///   and the plane frame are unchanged (this is not a transform of the object).
/// * `Table`: every cell's `text_height` and the style snapshot's `text_height`
///   multiply; column widths, row heights, merges, borders and text stay as they
///   were. Value-identity inheritance is preserved because both halves scale.
///
/// A degenerate stored length (non-finite or non-positive) and a result that is
/// not a usable dimension are refused **before** anything is written, with the
/// caller's `{prefix}.degenerate` / `{prefix}.invalid_result` namespace.
pub(super) fn scaled_annotation_text(
    geometry: &SemanticGeometry,
    factor: f64,
    prefix: &str,
) -> CadResult<Option<SemanticGeometry>> {
    Ok(match geometry {
        SemanticGeometry::Text { .. } => Some(scaled_text(geometry, factor, prefix)?),
        SemanticGeometry::MultiLeader(leader) => Some(SemanticGeometry::MultiLeader(Box::new(
            scaled_multileader_text(leader, factor, prefix)?,
        ))),
        SemanticGeometry::Table(table) => Some(SemanticGeometry::Table(Box::new(
            scaled_table_text(table, factor, prefix)?,
        ))),
        _ => None,
    })
}

/// Multiply one usable stored length by `factor`, refusing a degenerate input
/// and an unusable result under `prefix`.
fn scaled_length(value: f64, factor: f64, what: &str, prefix: &str) -> CadResult<f64> {
    if !value.is_finite() || value <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.degenerate: {what} is {value}, which is not a usable stored length"
        )));
    }
    let scaled = value * factor;
    if !scaled.is_finite() || scaled <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.invalid_result: {what} {value} x {factor} = {scaled} is not a usable \
             dimension"
        )));
    }
    Ok(scaled)
}

/// The `Text` arm of [`scaled_annotation_text`].
fn scaled_text(
    geometry: &SemanticGeometry,
    factor: f64,
    prefix: &str,
) -> CadResult<SemanticGeometry> {
    let SemanticGeometry::Text {
        text,
        position,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
        width_factor,
        line_spacing,
    } = geometry.clone()
    else {
        return Err(CadError::Invariant(
            "scaled_text: the caller passed a non-Text geometry".into(),
        ));
    };
    let new_height = scaled_length(height, factor, "the stored text height", prefix)?;
    let new_width = match wrap_width {
        Some(width) => Some(scaled_length(
            width,
            factor,
            "the stored wrap width",
            prefix,
        )?),
        None => None,
    };
    Ok(SemanticGeometry::Text {
        text,
        position,
        style,
        height: new_height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width: new_width,
        // A dimensionless stretch is unchanged by the height scale.
        width_factor,
        // Both halves of the line spacing are dimensionless ratios too.
        line_spacing,
    })
}

/// The `MultiLeader` arm of [`scaled_annotation_text`].
///
/// The geometry is rebuilt through the import constructor (which never rejects
/// and re-derives the display from the definition), so the stored display always
/// agrees with the scaled text. A zero/negative width boundary means "no width"
/// ([`cad_domain::wrap_width_from_dxf`]) and stays `0`.
fn scaled_multileader_text(
    leader: &cad_domain::MultiLeaderGeometry,
    factor: f64,
    prefix: &str,
) -> CadResult<cad_domain::MultiLeaderGeometry> {
    let text = leader.text();
    let new_height = scaled_length(text.height, factor, "the multileader text height", prefix)?;
    let new_width = if text.width.is_finite() && text.width > 0.0 {
        scaled_length(text.width, factor, "the multileader text width", prefix)?
    } else if text.width.is_finite() {
        // `<= 0` is the documented "no width boundary" sentinel.
        text.width
    } else {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.degenerate: the multileader text width is {}, which is not finite",
            text.width
        )));
    };
    let mut parts = cad_domain::MultiLeaderParts {
        style: leader.style().clone(),
        style_name: leader.style_name().to_string(),
        override_mask: leader.override_mask(),
        content: leader.content(),
        text: text.clone(),
        roots: leader.roots().to_vec(),
        lines: leader.lines().to_vec(),
        reserved: leader.reserved().clone(),
        plane: leader.plane(),
    };
    parts.text.height = new_height;
    parts.text.width = new_width;
    parts.style.text_height = scaled_length(
        parts.style.text_height,
        factor,
        "the multileader style text height",
        prefix,
    )?;
    let (scaled, _reasons) = cad_domain::MultiLeaderGeometry::from_import(parts);
    Ok(scaled)
}

/// The `Table` arm of [`scaled_annotation_text`].
///
/// Every cell height and the style snapshot's height scale together, so a cell
/// that still inherits its style (value-identity, see
/// [`cad_domain::TableStyleValues`]) keeps inheriting. The geometry is rebuilt
/// through the import constructor, which re-derives the display from the grid.
fn scaled_table_text(
    table: &cad_domain::TableGeometry,
    factor: f64,
    prefix: &str,
) -> CadResult<cad_domain::TableGeometry> {
    let mut rows = table.rows().to_vec();
    for row in &mut rows {
        for cell in &mut row.cells {
            cell.text_height =
                scaled_length(cell.text_height, factor, "a table cell text height", prefix)?;
        }
    }
    let mut style = table.style().clone();
    style.text_height = scaled_length(
        style.text_height,
        factor,
        "the table style text height",
        prefix,
    )?;
    let parts = cad_domain::TableParts {
        frame: table.frame(),
        columns: table.columns().to_vec(),
        rows,
        style_name: table.style_name().to_string(),
        name: table.name().to_string(),
        description: table.description().to_string(),
    };
    Ok(cad_domain::TableGeometry::from_import_with_style(
        parts, style,
    ))
}
