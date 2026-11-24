//! Block-attribute value drawing (ADR-0055 M2).
//!
//! One `ATTRIB` value of an `INSERT` is drawn through the **same** text path as
//! every other text run: [`text_fragment`] (shape → glyph fragments, `QTEXT`
//! handling, honest font diagnostics). There is no second text renderer here —
//! this module only projects one [`AttributeValue`] onto [`TextGeometry`].
//!
//! Submodule of [`crate::provider::default`].

use crate::*;

use super::text::{text_fragment, TextGeometry};

/// Build the text fragments for one attribute value, in **block-local**
/// coordinates.
///
/// The caller (instance expansion) applies the reference transform, the value's
/// stored appearance and the inherited clip/depth band, exactly like any other
/// block-member primitive. An empty value produces no fragments (an empty
/// attribute has nothing to draw); `QTEXT` still applies to a non-empty value
/// through the shared text path.
pub(crate) fn attribute_value_fragments(
    entity: &DbEntity,
    context: &RepresentationContext,
    value: &AttributeValue,
) -> DisplayRepresentation {
    let mut representation = DisplayRepresentation::empty(Completeness::Complete);
    if value.value.is_empty() {
        return representation;
    }
    if value.text.oblique.abs() > 1e-12 {
        representation.completeness =
            representation
                .completeness
                .clone()
                .combine(Completeness::Partial(vec![
                    "attribute oblique angle is stored but drawn upright".into(),
                ]));
        representation.diagnostics.push(Diagnostic {
            object: Some(ObjectId(entity.id.0)),
            code: "attribute.oblique_not_drawn".into(),
            message: format!(
                "attribute {:?} has an oblique angle {} rad; the shared text path draws it upright",
                value.tag, value.text.oblique
            ),
        });
    }
    text_fragment(
        entity,
        context,
        None,
        TextGeometry {
            text: &value.value,
            position: &value.text.position,
            style: &value.text.style,
            height: &value.text.height,
            rotation: &value.text.rotation,
            font: &value.text.font,
            h_align: &value.text.h_align,
            v_align: &value.text.v_align,
            wrap_width: &value.text.wrap_width,
            width_factor: &value.text.width_factor,
            line_spacing: &value.text.line_spacing,
            // A block attribute's stored text carries no MIRRTEXT bit; block
            // placement reflection is display-only and is not a per-value flag.
            mirrored: &false,
        },
        SelectionRef {
            document: context.document,
            entity: entity.id,
            instance: InstancePath::default(),
            sub_element: None,
        },
        GeometrySource::Analytic,
        &mut representation,
    );
    representation
}
