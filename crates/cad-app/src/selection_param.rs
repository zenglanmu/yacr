//! Parameter rows for the properties panel (ADR-0043 D9).
//!
//! A parameter is a document-level, named expression; an element binds some of
//! its type's closed slots to expressions. P0 does **not** add a parameters
//! panel: the palette only shows the parameters that the current selection's
//! bindings **directly reference**, one row each, so the panel can never become
//! a document table dump.
//!
//! The key is `param.<name>`. The name is the document's own spelling (source
//! data, unique inside one document), so it travels in
//! [`PropertyRow::label`] as well and is never translated. A row is writable
//! only when the selected ref addresses an element directly, exactly like the
//! fixed display rows and the dynamic BIM rows.
//!
//! Nothing here stores a value: the number is the result of
//! [`cad_db::DrawingDatabase::param_values`], evaluated once per call. A table
//! that cannot evaluate is a database invariant violation (the write path never
//! commits one), so the read side reports no parameter rows instead of
//! inventing values.

use cad_db::DrawingDatabase;
use cad_domain::param::ParamSeq;
use cad_domain::{EntityId, SelectionRef};
use std::collections::BTreeSet;

use crate::selection::{is_addressable, PropertyGroup, PropertyKind, PropertyRow};

/// The stable prefix of every parameter property key (`param.<name>`).
pub const KEY_PREFIX: &str = "param.";

/// Whether `key` is a parameter property key: the prefix plus a non-empty name.
///
/// This is the wire admission's question and the property command's dispatch
/// test. The suffix is only required to be non-empty; whether a parameter of
/// that name exists is the document's answer, not the key grammar's.
pub fn is_param_property_key(key: &str) -> bool {
    param_name(key).is_some()
}

/// The parameter name addressed by `key`, or `None` when the suffix is empty.
pub fn param_name(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX).filter(|name| !name.is_empty())
}

/// The parameter rows referenced by one element's bindings, in parameter order.
///
/// * An element without bindings emits **no** rows (never a placeholder).
/// * Only parameters named by a binding expression's [`Expr::references`] are
///   listed; the closure through other parameters is not expanded, because the
///   row is the parameter the element's own expression names.
/// * A reference whose sequence is absent from the table (or from the evaluated
///   values) is skipped rather than shown as a fabricated blank.
/// * Order is ascending parameter sequence, so the same document always yields
///   the same panel order.
pub(crate) fn param_property_rows(
    database: &DrawingDatabase,
    reference: &SelectionRef,
    id: EntityId,
) -> Vec<PropertyRow> {
    let Some(bindings) = database.param_bindings(id) else {
        return Vec::new();
    };
    if bindings.is_empty() {
        return Vec::new();
    }
    // The write path only ever commits an evaluable table; a failure here means
    // the document was corrupted outside that path, so there are no honest
    // numbers to show.
    let Ok(values) = database.param_values() else {
        return Vec::new();
    };
    let mut referenced: BTreeSet<ParamSeq> = BTreeSet::new();
    for (_, expr) in bindings.entries() {
        let mut refs = BTreeSet::new();
        expr.references(&mut refs);
        referenced.extend(refs);
    }
    let editable = is_addressable(reference);
    let mut rows = Vec::with_capacity(referenced.len());
    for seq in referenced {
        let Some(row) = database.parameters().get(seq) else {
            continue;
        };
        let Some(value) = values.get(seq) else {
            continue;
        };
        let mut property = PropertyRow::in_group(
            format!("{KEY_PREFIX}{}", row.name),
            format_f64(value.value),
            PropertyGroup::Param,
        );
        property.label = Some(row.name.clone());
        property.kind = PropertyKind::Number;
        property.editable = editable;
        rows.push(property);
    }
    rows
}

/// The shortest round-trip decimal form of a derived value.
///
/// This is display only: the write path parses the draft back into an
/// [`Expr::Num`], so the text a consumer reads must round-trip. Rust's `Display`
/// for `f64` already produces the shortest representation that parses back to
/// the same value; `-0.0` is normalised so a zero never renders with a sign.
fn format_f64(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::{is_param_property_key, param_name};

    #[test]
    fn the_key_prefix_needs_a_non_empty_name() {
        assert!(is_param_property_key("param.r"));
        assert!(is_param_property_key("param.板厚"));
        assert!(!is_param_property_key("param."));
        assert!(!is_param_property_key("parameter.r"));
        assert!(!is_param_property_key("bim.param"));
    }

    #[test]
    fn the_name_is_the_exact_suffix() {
        assert_eq!(param_name("param.r"), Some("r"));
        assert_eq!(param_name("param.a.b"), Some("a.b"));
        assert_eq!(param_name("param."), None);
    }
}
