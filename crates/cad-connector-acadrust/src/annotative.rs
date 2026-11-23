//! Annotation-scale import (spec §3.2 / capability matrix).
//!
//! Reads the drawing's `ACAD_SCALELIST` (`AcDbScale` objects) and the header
//! `CANNOSCALE`/`CANNOSCALEVALUE` into the database, marks annotative entities,
//! and attaches the per-scale placement overrides stored in an entity's
//! `AcDb*ObjectContextData` leaves.
//!
//! Exactness is explicit: TEXT/MTEXT placement and glyph scaling are exact;
//! every other annotative kind (dimension styles, multileader, hatch, blocks)
//! is imported as a flag plus an `annotative.unsupported_entity` `Partial`, and
//! an unknown active scale is reported rather than silently treated as 1:1.

use super::*;

use acadrust::objects::{ObjectContextData, ObjectContextKind, ObjectType, Scale as AcadScale};

/// Scale handle → name, for resolving an `ObjectContextData`'s scale reference.
pub(crate) type ScaleNames = HashMap<u64, String>;

/// Read every named annotation scale into the database.
///
/// Temporary scales (internal to acadrust) are skipped. The returned map is
/// keyed by the source handle value so a context leaf's scale reference can be
/// turned back into the name stored on the entity override.
pub(crate) fn read_scales(
    builder: &mut DrawingDatabaseBuilder,
    acad: &acadrust::CadDocument,
) -> CadResult<ScaleNames> {
    let mut names = HashMap::new();
    // Iterate in handle order so a hand-written/golden test sees stable ids.
    let mut scales: Vec<&AcadScale> = acad
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::Scale(scale) if !scale.is_temporary => Some(scale),
            _ => None,
        })
        .collect();
    scales.sort_by_key(|scale| scale.handle.value());
    for (index, scale) in scales.into_iter().enumerate() {
        let id = ScaleId(index as u128);
        names.insert(scale.handle.value(), scale.name.clone());
        builder.insert_scale(cad_db::Scale {
            id,
            name: scale.name.clone(),
            paper_units: scale.paper_units,
            drawing_units: scale.drawing_units,
        })?;
    }
    Ok(names)
}

/// Read the drawing's active annotation scale from the header.
///
/// A name that does not resolve in the Scale table is reported `Partial` with
/// the stable code `import.annotation_scale_unknown`; the header's raw value is
/// still stored so the fallback factor is the real one, never fabricated.
pub(crate) fn read_active_annotation_scale(
    builder: &mut DrawingDatabaseBuilder,
    acad: &acadrust::CadDocument,
    scales: &ScaleNames,
    diagnostics: &mut Vec<Diagnostic>,
) -> CadResult<()> {
    let header = &acad.header;
    let name = header.current_annotation_scale.trim();
    let value = header.annotation_scale_value;
    if name.is_empty() {
        return Ok(());
    }
    // A non-positive/non-finite header value cannot be a valid factor. Store a
    // safe 1.0 and report, rather than rejecting the whole drawing.
    let value = if value.is_finite() && value > 0.0 {
        value
    } else {
        diagnostics.push(Diagnostic {
            object: None,
            code: "import.annotation_scale_invalid".into(),
            message: format!(
                "CANNOSCALEVALUE {value} for scale '{name}' is not a positive number; using 1.0"
            ),
        });
        1.0
    };
    builder.set_active_annotation_scale(name, value)?;
    let known = scales.values().any(|n| n.eq_ignore_ascii_case(name));
    if !known {
        diagnostics.push(Diagnostic {
            object: None,
            code: "import.annotation_scale_unknown".into(),
            message: format!(
                "current annotation scale '{name}' is not in the drawing's Scale list; \
                 using the header factor {value}"
            ),
        });
    }
    Ok(())
}

/// Compute the annotative state for one entity.
///
/// Returns the attributes plus an optional stable `Partial` reason when an
/// annotative feature could not be represented exactly.
pub(crate) fn annotative_attributes(
    acad: &acadrust::CadDocument,
    entity: &EntityType,
    scales: &ScaleNames,
) -> (cad_db::AnnotativeAttributes, Option<String>) {
    let annotative = match entity {
        EntityType::MText(m) => m.is_annotative,
        EntityType::AttributeDefinition(a) => a.flags.annotative,
        EntityType::Text(t) => style_annotative(acad, &t.style),
        EntityType::AttributeEntity(a) => style_annotative(acad, &a.text_style),
        EntityType::Dimension(d) => dimstyle_annotative(acad, &d.base().style_name),
        EntityType::MultiLeader(m) => m.enable_annotation_scale,
        _ => false,
    };
    if !annotative {
        return (cad_db::AnnotativeAttributes::default(), None);
    }

    let (overrides, unsupported) = context_overrides(acad, entity, scales);
    let mut reason = None;
    if unsupported {
        reason = Some(format!(
            "annotative {} has context data this build cannot place exactly",
            entity_class_name(entity)
        ));
    }
    (
        cad_db::AnnotativeAttributes {
            annotative: true,
            overrides,
        },
        reason,
    )
}

/// Whether a text style is annotative.
fn style_annotative(acad: &acadrust::CadDocument, style: &str) -> bool {
    acad.text_styles
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(style.trim()))
        .map(|s| s.annotative)
        .unwrap_or(false)
}

/// Whether a dimension style is annotative.
fn dimstyle_annotative(acad: &acadrust::CadDocument, style: &str) -> bool {
    acad.dim_styles
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(style.trim()))
        .map(|s| s.annotative)
        .unwrap_or(false)
}

/// Walk `entity xdict -> "AcDbContextDataManager" -> "ACDB_ANNOTATIONSCALES"`
/// and collect the per-scale placement overrides.
///
/// Returns `(overrides, unsupported)`; `unsupported` is `true` when a leaf of a
/// kind this build cannot place was found, so the caller can report `Partial`.
fn context_overrides(
    acad: &acadrust::CadDocument,
    entity: &EntityType,
    scales: &ScaleNames,
) -> (Vec<cad_db::AnnotativeScaleOverride>, bool) {
    let common = entity.common();
    let Some(xdict) = common.xdictionary_handle.filter(|h| !h.is_null()) else {
        return (Vec::new(), false);
    };
    let Some(manager) = dictionary_entry(acad, xdict, "AcDbContextDataManager") else {
        return (Vec::new(), false);
    };
    let Some(annotation_scales) = dictionary_entry(acad, manager, "ACDB_ANNOTATIONSCALES") else {
        return (Vec::new(), false);
    };
    let Some(dict) = dictionary(acad, annotation_scales) else {
        return (Vec::new(), false);
    };

    let mut overrides = Vec::new();
    let mut unsupported = false;
    for (_key, leaf) in &dict.entries {
        let Some(ObjectType::ObjectContextData(context)) = acad.objects.get(leaf) else {
            continue;
        };
        let Some(name) = scales.get(&context.scale.value()) else {
            unsupported = true;
            continue;
        };
        match placement_from_context(context) {
            Some(placement) => overrides.push(cad_db::AnnotativeScaleOverride {
                scale: name.clone(),
                placement,
            }),
            None => unsupported = true,
        }
    }
    (overrides, unsupported)
}

/// Extract the placement this build understands, or `None` for an unsupported
/// context kind. `AnnotScale`/hatch/dim/mleader/leader/fcf leaves carry no
/// text placement, so they are explicitly unsupported rather than guessed.
fn placement_from_context(context: &ObjectContextData) -> Option<cad_db::AnnotativePlacement> {
    match &context.kind {
        ObjectContextKind::Text {
            horizontal_mode,
            rotation,
            insertion,
            alignment,
        } => {
            // Mirrors the base TEXT conversion: a non-left justification uses
            // the alignment point (DXF 11); otherwise the insertion point.
            let (x, y) = if *horizontal_mode != 0 {
                (alignment.x, alignment.y)
            } else {
                (insertion.x, insertion.y)
            };
            Some(cad_db::AnnotativePlacement {
                position: Point3 { x, y, z: 0.0 },
                rotation: *rotation,
                height: None,
            })
        }
        ObjectContextKind::MText(m) => Some(cad_db::AnnotativePlacement {
            position: p3(m.insertion),
            rotation: 0.0,
            height: None,
        }),
        _ => None,
    }
}

fn dictionary(
    acad: &acadrust::CadDocument,
    handle: acadrust::types::Handle,
) -> Option<&acadrust::objects::Dictionary> {
    match acad.objects.get(&handle) {
        Some(ObjectType::Dictionary(dict)) => Some(dict),
        _ => None,
    }
}

fn dictionary_entry(
    acad: &acadrust::CadDocument,
    dict: acadrust::types::Handle,
    key: &str,
) -> Option<acadrust::types::Handle> {
    dictionary(acad, dict)?
        .entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, h)| *h)
}
