//! The legacy-text parsing of the select/filter batch: the one-shot
//! `PROPERTY VALUE` line, the `SelectionMode` → apply-mode mapping, and the
//! named-filter (`.nfl`) refusal.
//!
//! Split out of `select_filter.rs` to keep that file inside the code-size limit.
//! Every item here is module-private; the parent module is its only caller.

use crate::command_payload::{
    SelectionApply, SelectionOperator, SelectionPredicate, SelectionProperty,
};
use crate::{CommandId, SelectionMode};
use cad_domain::{CadError, CadResult};

use super::{command_name, prefix};

/// Parse one legacy `PROPERTY VALUE` line into the shared predicate.
pub(super) fn legacy_predicate(
    id: CommandId,
    property: &str,
    value: &str,
) -> CadResult<SelectionPredicate> {
    let prefix = prefix(id);
    if value.trim().is_empty() {
        return Err(CadError::InvalidInput(format!(
            "{prefix}.empty_value: the filter value must not be empty"
        )));
    }
    let Some(property) = SelectionProperty::parse(property) else {
        let accepted = if id == CommandId::GetSelection {
            "LAYER or TYPE"
        } else {
            "TYPE, LAYER, COLOR, LINETYPE, LINEWEIGHT, LINETYPESCALE, TRANSPARENCY, SPACE, \
             TEXT or NAME"
        };
        return Err(CadError::InvalidInput(format!(
            "{prefix}.unknown_property: '{property}' is not a filter property ({accepted})"
        )));
    };
    if id == CommandId::GetSelection
        && !matches!(property, SelectionProperty::Layer | SelectionProperty::Type)
    {
        return Err(CadError::InvalidInput(
            "getsel.unknown_property: GETSEL accepts LAYER or TYPE only".into(),
        ));
    }
    Ok(SelectionPredicate {
        property,
        operator: SelectionOperator::Equals,
        value: value.trim().to_string(),
    })
}

/// Map the legacy `SelectionMode` onto the dialog apply modes.
pub(super) fn legacy_apply(id: CommandId, mode: SelectionMode) -> CadResult<SelectionApply> {
    let apply = match (id, mode) {
        (CommandId::QSelect, SelectionMode::Replace) => SelectionApply::IncludeNew,
        (CommandId::QSelect, SelectionMode::Add) => SelectionApply::Append,
        (CommandId::QSelect, SelectionMode::Remove) => SelectionApply::ExcludeNew,
        (CommandId::GetSelection, SelectionMode::Replace) => SelectionApply::IncludeNew,
        (CommandId::FilterSelect, SelectionMode::Add) => SelectionApply::Append,
        _ => {
            return Err(CadError::InvalidInput(format!(
                "{}.invalid_mode: {mode:?} is not an apply mode for {}",
                prefix(id),
                command_name(id)
            )))
        }
    };
    Ok(apply)
}

/// Whether a legacy property word asks for a **named filter** (`.nfl`).
///
/// Named filters are host file I/O (the AutoCAD `filter.nfl` store) and this
/// build has no host bridge for it, so it is a stable named refusal rather than
/// a silently empty selection.
pub(super) fn is_named_filter_query(property: &str) -> bool {
    property.trim().eq_ignore_ascii_case("NAMED")
        || property.trim().eq_ignore_ascii_case("NFILTER")
        || property.trim().eq_ignore_ascii_case("FILTER.NFL")
}

/// The named-filter refusal: `.nfl` loading/saving belongs to the host, not the
/// command layer.
pub(super) fn named_filter_refusal(name: &str) -> CadError {
    let name = name.trim();
    let detail = if name.is_empty() {
        "no filter name given".to_string()
    } else {
        format!("'{name}'")
    };
    CadError::Unsupported(format!(
        "filter.named_unsupported: named filter {detail} is host file I/O (filter.nfl); this \
         build has no host bridge for it"
    ))
}
