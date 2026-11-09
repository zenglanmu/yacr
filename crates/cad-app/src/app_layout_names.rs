//! Name resolution shared by the layout/viewport command handlers.
//!
//! Split out of `app_layout` to keep that module inside the 1000-line code gate.
//! The grammar mirrors `cad-db`'s `layout_edit` name validation exactly, so both
//! entry points accept and store the same spelling.

use super::*;
use cad_db::DrawingDatabase;

/// One paper layout row the commands resolve against.
pub(super) type LayoutSummary = (LayoutId, String, usize);

/// Validate a layout name and return the trimmed form.
pub(super) fn validate_layout_name(name: &str) -> CadResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must not be empty".into(),
        ));
    }
    if name.contains('\r') || name.contains('\n') {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must be a single line".into(),
        ));
    }
    if name.chars().count() > 255 {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name is limited to 255 characters".into(),
        ));
    }
    if name.chars().any(|ch| "<>/\\\":;?*|=`".contains(ch)) {
        return Err(CadError::InvalidInput(format!(
            "layout.invalid_name: layout name {:?} contains an AutoCAD-illegal character",
            name
        )));
    }
    Ok(name.to_string())
}

/// Find a layout by name (case-insensitive), excluding `exclude` when renaming.
pub(super) fn find_layout_named(
    drawing: &DrawingDatabase,
    name: &str,
    exclude: Option<LayoutId>,
) -> Option<LayoutId> {
    drawing
        .layouts()
        .filter(|layout| Some(layout.id) != exclude)
        .find(|layout| layout.name.eq_ignore_ascii_case(name))
        .map(|layout| layout.id)
}

/// Resolve a layout name: exact first, then a unique case-insensitive match.
pub(super) fn resolve_layout_name(
    drawing: &DrawingDatabase,
    name: &str,
) -> CadResult<LayoutSummary> {
    let rows: Vec<LayoutSummary> = drawing
        .layouts()
        .map(|layout| (layout.id, layout.name.clone(), layout.viewports.len()))
        .collect();
    let wanted = name.trim();
    if let Some(row) = rows
        .iter()
        .find(|(_, layout_name, _)| layout_name == wanted)
    {
        return Ok((*row).clone());
    }
    let matching: Vec<&LayoutSummary> = rows
        .iter()
        .filter(|(_, layout_name, _)| layout_name.eq_ignore_ascii_case(wanted))
        .collect();
    match matching.as_slice() {
        [row] => Ok((**row).clone()),
        [] => Err(CadError::InvalidInput(format!(
            "layout.unknown: no paper layout named {:?}",
            wanted
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "layout.ambiguous: more than one layout matches {:?} case-insensitively",
            wanted
        ))),
    }
}

/// Resolve a layer name exactly, then by a unique case-insensitive match.
pub(super) fn resolve_layer_name(drawing: &DrawingDatabase, name: &str) -> CadResult<LayerId> {
    if let Some(id) = drawing.layer_id_by_name(name) {
        return Ok(id);
    }
    let matches: Vec<_> = drawing
        .layers()
        .filter(|layer| layer.name.eq_ignore_ascii_case(name))
        .map(|layer| layer.id)
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(CadError::InvalidInput(format!(
            "layvpi.unknown_layer: no layer named {:?}",
            name
        ))),
        _ => Err(CadError::InvalidInput(format!(
            "layvpi.ambiguous_layer: more than one layer matches {:?} case-insensitively",
            name
        ))),
    }
}
