//! Diagnostics aggregation, capability rows and the finished report.

use cad_connector_core::ImportReport;
use cad_db::{BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_PSETS_PER_ELEMENT, BIM_MAX_STRING_BYTES};
use cad_domain::{CadError, CadResult, Completeness, EntityCapability, SupportStatus, UnitContext};

use super::{no_geometry_message, unknown_only_message, Mapper};
use crate::{
    BIM_BUDGET_EXCEEDED, BIM_CATEGORY_FROM_TAG, BIM_NO_CATEGORY, BIM_TAGS_STORED, BIM_UNVERIFIED,
    BLOCK_MISSING, CELL_MIRRORED_Z, DEPTH_LIMIT, ELEMENT_NOT_MODELLED, GEOMETRY_REJECTED,
    GROUP_FLATTENED, LAYER_UNKNOWN, LINETYPE_PATTERN_UNSCALED, LINETYPE_UNKNOWN, MODEL_SKIPPED,
    RASTER_NOT_LOADED, TAGS_NOT_STORED, TEXT_ALIGN_UNSUPPORTED, TEXT_PLANE_NOT_SUPPORTED,
    TEXT_WIDTH_FACTOR, UNKNOWN_ELEMENT,
};

impl Mapper<'_> {
    /// Emit every aggregate diagnostic, then publish the database + report.
    pub(crate) fn finish(
        mut self,
        identity: cad_domain::DocumentIdentity,
        parse_ms: f64,
    ) -> CadResult<(cad_db::DrawingDatabase, UnitContext, ImportReport)> {
        if self.inserted == 0 {
            let unsupported: usize = self.unknown_types.values().sum::<usize>()
                + self.unmodelled_types.values().sum::<usize>();
            if unsupported > 0 {
                let mut names: Vec<String> = self
                    .unknown_types
                    .iter()
                    .map(|(name, count)| format!("{name} x{count}"))
                    .chain(
                        self.unmodelled_types
                            .iter()
                            .map(|(name, count)| format!("{name} x{count}")),
                    )
                    .collect();
                names.sort();
                return Err(CadError::Unsupported(unknown_only_message(
                    &names,
                    unsupported,
                )));
            }
            return Err(CadError::Unsupported(no_geometry_message(&self)));
        }
        let capabilities = self.capabilities();
        let completeness = self.completeness();
        self.emit_aggregates();

        self.builder
            .set_hidden_entities(self.hidden.iter().copied())?;
        let database = self.builder.finish()?;
        let report = ImportReport {
            identity,
            capabilities,
            completeness,
            diagnostics: self.diagnostics,
            connector: None,
            parse_ms: Some(parse_ms),
            object_xdata: Vec::new(),
        };
        Ok((database, self.units, report))
    }

    fn emit_aggregates(&mut self) {
        for (type_name, count) in std::mem::take(&mut self.unknown_types) {
            self.note(
                UNKNOWN_ELEMENT,
                format!(
                    "{count} DGN element(s) of unmodelled type '{type_name}' were skipped; the \
                     reader kept their record but this build has no vector mapping for them"
                ),
                None,
            );
        }
        for (label, count) in std::mem::take(&mut self.unmodelled_types) {
            self.note(
                ELEMENT_NOT_MODELLED,
                format!("{count} {label} element(s) were skipped: no cad-db vector mapping"),
                None,
            );
        }
        if self.tags > 0 || self.bim_undecodable_tags > 0 {
            self.note(
                TAGS_NOT_STORED,
                format!(
                    "{} DGN tag/attribute value(s) were not stored: {} belong to an element with no \
                     database row (skipped type, unknown element or a flattened group owner) and {} \
                     have no decoded tag name (the reader's `tag_<n>` fallback for a missing tag-set \
                     definition) or a value with no BIM form (binary, list or non-finite real); no \
                     name or value was fabricated",
                    self.tags + self.bim_undecodable_tags,
                    self.tags,
                    self.bim_undecodable_tags
                ),
                None,
            );
        }
        if self.bim_stored_tags > 0 {
            self.note(
                BIM_TAGS_STORED,
                format!(
                    "{} DGN tag/Item Type value(s) were stored as {} BIM property set(s) on {} \
                     element(s); each set is named after its tag set (or '{}' when the file named \
                     none) and every value keeps its decoded type",
                    self.bim_stored_tags,
                    self.bim_psets,
                    self.bim_elements,
                    crate::bim::DGN_TAGS_SET
                ),
                None,
            );
        }
        if self.bim_categories > 0 {
            self.note(
                BIM_CATEGORY_FROM_TAG,
                format!(
                    "{} element(s) took bim.category from an explicit Category/IfcType/IFCEntity \
                     tag (name compared case-insensitively, value must be a single token); no level \
                     name, cell name or geometric heuristic was used",
                    self.bim_categories
                ),
                None,
            );
        }
        if self.bim_no_category > 0 {
            self.note(
                BIM_NO_CATEGORY,
                format!(
                    "{} element(s) with stored BIM properties carry no category: the file declared \
                     no Category/IfcType/IFCEntity token for them, and a level name or shared-cell \
                     name is never read as one",
                    self.bim_no_category
                ),
                None,
            );
        }
        if self.bim_budget_dropped > 0 {
            self.note(
                BIM_BUDGET_EXCEEDED,
                format!(
                    "{} BIM part(s) exceeded a cad-db budget and were dropped whole, not truncated: \
                     at most {BIM_MAX_PSETS_PER_ELEMENT} property sets per element, \
                     {BIM_MAX_PROPERTIES_PER_SET} properties per set and {BIM_MAX_STRING_BYTES} bytes \
                     per string",
                    self.bim_budget_dropped
                ),
                None,
            );
        }
        if self.bim_elements > 0 {
            self.note(
                BIM_UNVERIFIED,
                format!(
                    "the DGN tags of {} element(s) were interpreted as BIM property sets with \
                     evidence 'declared' (the file's author wrote the values), but the BIM \
                     interpretation itself is unverified: this adapter has no real V8i/CONNECT sample \
                     with Item Types behind it",
                    self.bim_elements
                ),
                None,
            );
        }
        if self.unknown_layers > 0 {
            self.note(
                LAYER_UNKNOWN,
                format!(
                    "{} entity layer reference(s) named no imported level; they were placed on \
                     layer '0' rather than dropped",
                    self.unknown_layers
                ),
                None,
            );
        }
        if !self.unknown_linetypes.is_empty() {
            let names: Vec<&str> = self.unknown_linetypes.iter().map(String::as_str).collect();
            self.note(
                LINETYPE_UNKNOWN,
                format!(
                    "{} entity linetype name(s) are absent from the DGN linetype table ({}); \
                     the affected lines were drawn continuous",
                    names.len(),
                    super::summarize(&names)
                ),
                None,
            );
        }
        if self.rasters > 0 {
            self.note(
                RASTER_NOT_LOADED,
                format!(
                    "{} raster reference(s) were imported as placement only; cadkit-dgn reads no \
                     raster pixels, no embedded image or texture is stored, and a reference with \
                     no pixel size became an opaque placeholder",
                    self.rasters
                ),
                None,
            );
        }
        if self.groups_flattened > 0 {
            self.note(
                GROUP_FLATTENED,
                format!(
                    "{} DGN group(s) (complex chain/shape, text node or cell) were flattened into \
                     their member entities; no group row is stored",
                    self.groups_flattened
                ),
                None,
            );
        }
        if self.text_align_losses > 0 {
            self.note(
                TEXT_ALIGN_UNSUPPORTED,
                format!(
                    "{} text run(s) use an alignment beyond the domain's left/center/right \
                     horizontal set; they were stored left-aligned",
                    self.text_align_losses
                ),
                None,
            );
        }
        if self.text_plane_losses > 0 {
            self.note(
                TEXT_PLANE_NOT_SUPPORTED,
                format!(
                    "{} text run(s) lie on a non-world-Z plane; the domain text stores no plane, \
                     so the in-plane rotation was kept but the normal was dropped",
                    self.text_plane_losses
                ),
                None,
            );
        }
        if self.mtext_flattened > 0 {
            self.note(
                crate::TEXT_FORMATTING,
                format!(
                    "{} multiline text run(s) were stored as plain text; DGN text-node markup \
                     and per-line formatting are not represented",
                    self.mtext_flattened
                ),
                None,
            );
        }
        if self.text_width_losses > 0 {
            self.note(
                TEXT_WIDTH_FACTOR,
                format!(
                    "{} text run(s) carry a non-positive or non-finite width factor; the neutral \
                     factor 1.0 was stored",
                    self.text_width_losses
                ),
                None,
            );
        }
        if self.unscaled_linetypes > 0 {
            self.note(
                LINETYPE_PATTERN_UNSCALED,
                format!(
                    "{} linetype pattern(s) are defined on paper but the file declares no \
                     master-unit size, so they could not be expressed in drawing units",
                    self.unscaled_linetypes
                ),
                None,
            );
        }
        if self.mirrored_cells > 0 {
            self.note(
                CELL_MIRRORED_Z,
                format!(
                    "{} shared cell instance(s) have a negative determinant (mirrored); the \
                     transform is stored as read",
                    self.mirrored_cells
                ),
                None,
            );
        }
        if self.missing_blocks > 0 {
            self.note(
                BLOCK_MISSING,
                format!(
                    "{} shared cell instance(s) named a block with no definition and were \
                     skipped",
                    self.missing_blocks
                ),
                None,
            );
        }
        if self.depth_limited > 0 {
            self.note(
                DEPTH_LIMIT,
                format!(
                    "{} block/group nesting depth loss(es) were reported; the import did not \
                     expand past {} level(s)",
                    self.depth_limited, self.max_depth
                ),
                None,
            );
        }
        if self.skipped_models > 0 {
            self.note(
                MODEL_SKIPPED,
                format!(
                    "{} non-primary DGN model(s) were skipped; only one model is imported",
                    self.skipped_models
                ),
                None,
            );
        }
        if self.rejected > 0 {
            self.note(
                GEOMETRY_REJECTED,
                format!(
                    "{} element(s) were rejected before insertion; each has its own diagnostic",
                    self.rejected
                ),
                None,
            );
        }
    }

    fn capabilities(&self) -> Vec<EntityCapability> {
        let mut rows = Vec::new();
        for (type_key, partial) in &self.kinds {
            let (read, render, pick, measure) = match *type_key {
                "DgnText" | "DgnInsert" => (
                    SupportStatus::Verified,
                    SupportStatus::Unverified,
                    SupportStatus::Unverified,
                    SupportStatus::Unsupported,
                ),
                // Raster placement was read, but no pixels/embedding are
                // stored: the semantic row is explicitly partial, never a
                // silent "supported image".
                "DgnRaster" => (
                    SupportStatus::Verified,
                    SupportStatus::Unsupported,
                    SupportStatus::Unsupported,
                    SupportStatus::Unsupported,
                ),
                _ => (
                    SupportStatus::Verified,
                    SupportStatus::Verified,
                    SupportStatus::Verified,
                    SupportStatus::Verified,
                ),
            };
            rows.push(EntityCapability {
                type_key: (*type_key).to_string(),
                read,
                semantic: if *partial || *type_key == "DgnRaster" {
                    SupportStatus::Partial
                } else {
                    SupportStatus::Verified
                },
                render,
                pick,
                measure,
            });
        }
        rows.sort_by(|left, right| left.type_key.cmp(&right.type_key));
        rows
    }

    fn completeness(&self) -> Completeness {
        let mut reasons = vec![format!(
            "DGN level display, freeze and lock flags are not decoded; layer rows carry name \
             and visibility only ({} level(s))",
            self.doc.layers.len()
        )];
        if self.tags > 0 || self.bim_undecodable_tags > 0 {
            reasons.push(format!(
                "{} DGN tag/attribute value(s) are not stored",
                self.tags + self.bim_undecodable_tags
            ));
        }
        let unsupported: usize = self.unknown_types.values().sum::<usize>()
            + self.unmodelled_types.values().sum::<usize>();
        if unsupported > 0 {
            reasons.push(format!(
                "{unsupported} element(s) of unsupported type were skipped"
            ));
        }
        if self.skipped_models > 0 {
            reasons.push(format!(
                "{} non-primary DGN model(s) were not imported",
                self.skipped_models
            ));
        }
        let mut other = Vec::new();
        if self.rasters > 0 {
            other.push(format!(
                "{} raster reference(s) store placement only, no pixels",
                self.rasters
            ));
        }
        if self.groups_flattened > 0 {
            other.push(format!("{} group(s) were flattened", self.groups_flattened));
        }
        if self.rejected > 0 {
            other.push(format!(
                "{} element(s) were rejected by geometry validation",
                self.rejected
            ));
        }
        if self.missing_blocks > 0 {
            other.push(format!(
                "{} shared cell instance(s) named a missing block",
                self.missing_blocks
            ));
        }
        if self.unknown_layers > 0 {
            other.push(format!(
                "{} unknown layer reference(s) fell back to layer 0",
                self.unknown_layers
            ));
        }
        if !self.unknown_linetypes.is_empty() {
            other.push(format!(
                "{} unknown linetype name(s)",
                self.unknown_linetypes.len()
            ));
        }
        if self.mirrored_cells > 0 {
            other.push(format!(
                "{} mirrored shared cell instance(s)",
                self.mirrored_cells
            ));
        }
        if self.depth_limited > 0 {
            other.push(format!(
                "{} block/group nesting depth loss(es)",
                self.depth_limited
            ));
        }
        if self.text_align_losses
            + self.text_plane_losses
            + self.text_width_losses
            + self.mtext_flattened
            > 0
        {
            other.push(format!(
                "{} text formatting/plane loss(es)",
                self.text_align_losses
                    + self.text_plane_losses
                    + self.text_width_losses
                    + self.mtext_flattened
            ));
        }
        if self.unscaled_linetypes > 0 {
            other.push(format!(
                "{} linetype pattern(s) could not be expressed in drawing units",
                self.unscaled_linetypes
            ));
        }
        if self.units_partial {
            other.push("the master unit is outside the domain's unit set".to_string());
        }
        if self.bim_elements > 0 {
            other.push(format!(
                "BIM semantics read from DGN tags are Declared but Unverified ({} element(s), {} \
                 property set(s)); no real V8i/CONNECT sample backs them",
                self.bim_elements, self.bim_psets
            ));
        }
        if self.bim_budget_dropped > 0 {
            other.push(format!(
                "{} BIM part(s) were dropped by a cad-db budget",
                self.bim_budget_dropped
            ));
        }
        if !other.is_empty() {
            reasons.push(other.join("; "));
        }
        Completeness::Partial(reasons)
    }
}
