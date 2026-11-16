//! The TABLE half of the default provider.
//!
//! Split out of `default.rs` for the file-size ceiling (`check-file-size.py`).
//! Everything here is one table build: recursing over the table's derived
//! display, stamping each primitive's own appearance onto the fragments it
//! produced, and reporting the one thing the display cannot honour (a stored
//! per-cell border lineweight that this build draws one pixel wide).

use crate::*;

use super::TextScale;

impl DefaultRepresentationProvider {
    /// Build a table's fragments, stamping each derived primitive's own
    /// appearance onto the fragments it produced.
    ///
    /// The derived display is one geometry per drawn primitive **plus** a
    /// parallel appearance list of the same length (the domain builds both
    /// together), so the zip cannot drift. A primitive with
    /// [`TableDisplayAppearance::NONE`] keeps the entity's resolved colour and
    /// lineweight, i.e. its fragments are byte-identical to the pre-appearance
    /// build.
    ///
    /// The one thing the display cannot honour is a border **lineweight**: every
    /// line in this build is drawn one pixel wide, so a stored weight reaches the
    /// batch (where the renderer reports `render.lineweight_not_drawn`) but no
    /// wider stroke. That gap is reported here, once per build, and only when a
    /// weight is actually modelled.
    pub(super) fn build_table(
        &self,
        entity: &DbEntity,
        table: &cad_domain::TableGeometry,
        context: &RepresentationContext,
        annotative: Option<TextScale>,
    ) -> DisplayRepresentation {
        let mut representation = DisplayRepresentation {
            fragments: Vec::new(),
            completeness: Completeness::Complete,
            diagnostics: Vec::new(),
        };
        let appearances = table.display_appearances();
        let mut lineweight_modelled = false;
        for (index, child) in table.display().iter().enumerate() {
            let appearance = appearances
                .get(index)
                .copied()
                .unwrap_or(cad_domain::TableDisplayAppearance::NONE);
            let child_representation = match self.build_geometry(entity, child, context, annotative)
            {
                Ok(child) => child,
                Err(error) => {
                    // A child that cannot be built is reported, never silently
                    // dropped: the table's other cells still draw.
                    representation.completeness =
                        representation
                            .completeness
                            .combine(Completeness::Partial(vec![format!(
                                "a table display primitive could not be built: {error}"
                            )]));
                    representation.diagnostics.push(Diagnostic {
                        object: Some(ObjectId(entity.id.0)),
                        code: "representation.table_primitive_failed".into(),
                        message: format!("table display primitive {index} was not drawn: {error}"),
                    });
                    continue;
                }
            };
            if appearance.lineweight_hundredths.is_some() {
                lineweight_modelled = true;
            }
            for mut fragment in child_representation.fragments {
                if let Some(rgb) = appearance.color {
                    fragment.color = [
                        rgb[0] as f32 / 255.0,
                        rgb[1] as f32 / 255.0,
                        rgb[2] as f32 / 255.0,
                    ];
                    fragment.color_unresolved = false;
                }
                if let Some(hundredths) = appearance.lineweight_hundredths {
                    fragment.lineweight = hundredths as f32 / 100.0;
                    fragment.lineweight_unresolved = false;
                }
                representation.fragments.push(fragment);
            }
            representation
                .diagnostics
                .extend(child_representation.diagnostics);
            representation.completeness = representation
                .completeness
                .combine(child_representation.completeness);
        }
        if lineweight_modelled {
            representation.completeness = representation
                .completeness
                .combine(Completeness::Partial(vec![
                    "a table cell border lineweight is stored and exported but drawn one pixel                      wide: this build draws every line at one width"
                        .into(),
                ]));
            representation.diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "representation.table_lineweight_not_drawn".into(),
                message: "per-cell table border lineweights reach the batch but the renderer draws                           every line one pixel wide"
                    .into(),
            });
        }
        representation
    }
}
