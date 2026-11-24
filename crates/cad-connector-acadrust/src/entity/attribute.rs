//! Block attribute conversion (`ATTDEF` definitions and `ATTRIB` values).
//!
//! ADR-0055 M2: an `ATTDEF` is a block **member entity** of class
//! [`SemanticGeometry::AttributeDefinition`]; the concrete values a reference
//! carries live on [`SemanticGeometry::Insert::attributes`] in the same
//! block-local frame. This module maps acadrust's typed entities onto that
//! model. It never flattens an `ATTDEF` into a `TEXT` (that was the pre-M2
//! behaviour) and never guesses an owner for a standalone `ATTRIB`.
//!
//! Submodule of [`crate::entity`].

use super::*;

use acadrust::entities::attribute_definition::{
    HorizontalAlignment as DxfHAlign, VerticalAlignment as DxfVAlign,
};

/// Why an attribute text placement could not be stored verbatim.
const ATTRIBUTE_HEIGHT_DEGENERATE: &str = "attribute.height_degenerate";
const ATTRIBUTE_TEXT_NONFINITE: &str = "attribute.text_nonfinite";
const ATTRIBUTE_WIDTH_FACTOR_DEGENERATE: &str = "attribute.width_factor_degenerate";

/// Map acadrust's `ATTDEF` flags onto the domain's named flags.
fn map_attribute_flags(flags: acadrust::entities::AttributeFlags) -> AttributeFlags {
    AttributeFlags {
        invisible: flags.invisible,
        constant: flags.constant,
        verify: flags.verify,
        preset: flags.preset,
        locked_position: flags.locked_position,
        annotative: flags.annotative,
    }
}

/// Map the `ATTDEF`/`ATTRIB` horizontal alignment onto the text vocabulary.
fn map_attribute_h_align(align: DxfHAlign) -> TextAlignH {
    match align {
        DxfHAlign::Center | DxfHAlign::Middle => TextAlignH::Center,
        DxfHAlign::Right => TextAlignH::Right,
        // Aligned/Fit still start at the left point; no stretch is invented.
        DxfHAlign::Left | DxfHAlign::Aligned | DxfHAlign::Fit => TextAlignH::Left,
    }
}

/// Map the `ATTDEF`/`ATTRIB` vertical alignment onto the text vocabulary.
fn map_attribute_v_align(align: DxfVAlign) -> TextAlignV {
    match align {
        DxfVAlign::Baseline => TextAlignV::Baseline,
        DxfVAlign::Bottom => TextAlignV::Bottom,
        DxfVAlign::Middle => TextAlignV::Middle,
        DxfVAlign::Top => TextAlignV::Top,
    }
}

impl<'a> ImporterBuilder<'a> {
    /// The text placement of one `ATTDEF`/`ATTRIB`, sanitised for storage.
    ///
    /// The rotation/oblique/normal are applied through the shared OCS rule the
    /// TEXT path uses (`text_ocs_placement`). A value the write validator would
    /// reject (non-finite position, non-positive height, a degenerate width
    /// factor) is replaced by the neutral value and named in `losses`; it is never
    /// stored as a value that cannot be drawn.
    #[allow(clippy::too_many_arguments)]
    fn attribute_text_fields(
        &mut self,
        style: &str,
        normal: acadrust::types::Vector3,
        insertion_point: acadrust::types::Vector3,
        alignment_point: acadrust::types::Vector3,
        height: f64,
        rotation: f64,
        width_factor: f64,
        oblique: f64,
        h_align: DxfHAlign,
        v_align: DxfVAlign,
    ) -> (AttributeText, Vec<String>) {
        let mut losses = Vec::new();
        let h = map_attribute_h_align(h_align);
        let v = map_attribute_v_align(v_align);
        // DXF places aligned text at group 11 when the alignment is not the
        // left/baseline default (exactly the TEXT rule).
        let raw_position = if (h, v) != (TextAlignH::Left, TextAlignV::Baseline) {
            alignment_point
        } else {
            insertion_point
        };
        let p = p3(raw_position);
        let (position, rotation) = text_ocs_placement(p3(normal), p, rotation);
        let position = if position.x.is_finite() && position.y.is_finite() && position.z.is_finite()
        {
            position
        } else {
            losses.push(ATTRIBUTE_TEXT_NONFINITE.to_string());
            Point3::default()
        };
        let height = if height.is_finite() && height > 0.0 {
            height
        } else {
            losses.push(ATTRIBUTE_HEIGHT_DEGENERATE.to_string());
            1.0
        };
        let width_factor = match text_width_factor_from_dxf(width_factor) {
            Some(factor) => factor,
            None => {
                losses.push(ATTRIBUTE_WIDTH_FACTOR_DEGENERATE.to_string());
                DEFAULT_TEXT_WIDTH_FACTOR
            }
        };
        let rotation = if rotation.is_finite() { rotation } else { 0.0 };
        let oblique = if oblique.is_finite() { oblique } else { 0.0 };
        (
            AttributeText {
                position,
                height,
                rotation,
                style: self.style_id(style),
                width_factor,
                h_align: h,
                v_align: v,
                wrap_width: None,
                line_spacing: TextLineSpacing::default(),
                font: self.style_font(style),
                oblique,
            },
            losses,
        )
    }

    /// Convert one `ATTDEF` into its block-member definition geometry.
    ///
    /// The ordering key is a monotonically increasing import ordinal: it keeps
    /// the source order within a block (the only order a reader can observe
    /// there) without pretending the source handle is an order.
    pub(crate) fn attribute_definition_semantics(
        &mut self,
        a: &acadrust::entities::AttributeDefinition,
    ) -> (SemanticGeometry, Completeness) {
        let (text, losses) = self.attribute_text_fields(
            &a.text_style,
            a.normal,
            a.insertion_point,
            a.alignment_point,
            a.height,
            a.rotation,
            a.width_factor,
            a.oblique_angle,
            a.horizontal_alignment,
            a.vertical_alignment,
        );
        let order = self.next_attribute_order;
        self.next_attribute_order += 1;
        let definition = AttributeDefinition {
            tag: a.tag.clone(),
            prompt: a.prompt.clone(),
            default_value: a.default_value.clone(),
            flags: map_attribute_flags(a.flags),
            order,
            text,
            source_handle: Some(format!("{:X}", a.common.handle.value())),
        };
        let geometry = SemanticGeometry::AttributeDefinition(Box::new(definition));
        let completeness = attribute_completeness(
            &mut self.reported_reasons,
            &mut self.diagnostics,
            losses,
            a.common.handle.value(),
        );
        (geometry, completeness)
    }

    /// Convert one `ATTRIB` carried by an `INSERT` into a block-local value.
    ///
    /// The acadrust reader already attached the value to its reference, so no
    /// owner is guessed. Duplicate tags stay allowed: the vector keeps every
    /// value in source order, exactly as the model permits.
    pub(crate) fn attribute_value(
        &mut self,
        a: &acadrust::entities::AttributeEntity,
    ) -> (AttributeValue, Completeness) {
        let (text, losses) = self.attribute_text_fields(
            &a.text_style,
            a.normal,
            a.insertion_point,
            a.alignment_point,
            a.height,
            a.rotation,
            a.width_factor,
            a.oblique_angle,
            a.horizontal_alignment,
            a.vertical_alignment,
        );
        let layer = self
            .layer_ids
            .get(&a.common.layer)
            .copied()
            .unwrap_or_else(|| self.layer_ids.get("0").copied().unwrap_or(LayerId(0)));
        let value = AttributeValue {
            tag: a.tag.clone(),
            value: a.value.clone(),
            flags: map_attribute_flags(a.flags),
            text,
            layer,
            appearance: self.attribute_appearance(&a.common),
            source_handle: Some(format!("{:X}", a.common.handle.value())),
        };
        let completeness = attribute_completeness(
            &mut self.reported_reasons,
            &mut self.diagnostics,
            losses,
            a.common.handle.value(),
        );
        (value, completeness)
    }

    /// The draw appearance of one attribute value, preserving symbolic provenance.
    ///
    /// The rules are the same ones an entity row uses: `ByLayer` stays symbolic
    /// (the representation resolves it against the value's own layer), `ByBlock`
    /// stays symbolic (the representation resolves it against the containing
    /// INSERT), and a concrete value is stored byte-identically.
    fn attribute_appearance(&self, common: &EntityCommon) -> AttributeAppearance {
        let handle_name = common
            .linetype_handle
            .filter(|h| !h.is_null())
            .and_then(|h| self.linetype_names_by_handle.get(&h.value()).cloned());
        let source_linetype: String = if is_bylayer_linetype(&common.linetype) {
            match handle_name {
                Some(name) => name,
                None => common.linetype.clone(),
            }
        } else {
            common.linetype.clone()
        };
        let named_key = source_linetype.trim().to_ascii_lowercase();
        let named_pattern =
            if is_bylayer_linetype(&source_linetype) || is_byblock_linetype(&source_linetype) {
                None
            } else {
                self.linetype_patterns.get(&named_key).cloned()
            };
        AttributeAppearance {
            transparency: resolve_entity_transparency(common.transparency),
            color: resolve_entity_color(common.color),
            lineweight: resolve_entity_lineweight(common.line_weight),
            linetype: resolve_entity_linetype(
                &source_linetype,
                common.linetype_scale,
                named_pattern,
            ),
            linetype_scale: sanitize_linetype_scale(common.linetype_scale),
        }
    }
}

/// Fold text-placement losses into a verdict and report each reason once.
fn attribute_completeness(
    reported: &mut BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
    losses: Vec<String>,
    handle: u64,
) -> Completeness {
    if losses.is_empty() {
        return Completeness::Complete;
    }
    for reason in &losses {
        if reported.insert(reason.clone()) {
            diagnostics.push(Diagnostic {
                object: None,
                code: "import.attribute_partial".into(),
                message: format!(
                    "attribute (handle {:X}) has a {reason} placement; the neutral value is used \
                     instead",
                    handle
                ),
            });
        }
    }
    Completeness::Partial(losses)
}
