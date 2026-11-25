//! POC block-instance templates for pure-translation INSERT placements.
//!
//! A placement is eligible only when every input that can change its geometry
//! or style is trivial: a direct `Insert`, no ATTRIB values, default
//! `EntityRenderAttributes` (no clip or explicit style), a pure translation,
//! an ordinary (non-dynamic) block without nested inserts, and no image/mask
//! fragment in the template. Everything else falls back to the expanded path
//! and is counted by reason; there is no silent template.
//!
//! The shared template is block-local. A placement is a metadata-only
//! `RenderBatch` at a resident index plus a `TemplateTable` entry; staging
//! composes the template with the placement's translated origin, draw order and
//! own sources (`ResidentScene::materialized`). This is gated by
//! `YACR_BLOCK_INSTANCING` and is not a product default.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::*;
use cad_domain::{
    BlockId, CadResult, Completeness, LayerId, SemanticGeometry, TaskStamp, Transform3,
};
use cad_representation::{ProviderRegistry, RepresentationContext};
use cad_scene::{RenderBatch, SceneBudget, SceneCache};

const TRANSLATION_EPSILON: f64 = 1e-12;

/// Shared block-local batch list for one (block, style) template.
type TemplateBatchList = Arc<Vec<Arc<RenderBatch>>>;
/// Block, effective layer and resolved render-attribute fingerprint.
type TemplateKey = (BlockId, LayerId, String);

/// One applied placement: metadata-only batches plus their shared templates.
pub(super) struct InstancedPlacement {
    pub batches: Vec<RenderBatch>,
    /// `(offset within `batches`, block-local template batch)`.
    pub templates: Vec<(usize, Arc<RenderBatch>)>,
}

/// Template cache + rejection counters for one scene build.
pub(super) struct BlockInstanceBuilder {
    cache: HashMap<TemplateKey, TemplateBatchList>,
    scene: SceneCache,
    rejected: BTreeMap<&'static str, usize>,
}

impl BlockInstanceBuilder {
    pub(super) fn new() -> Self {
        Self {
            cache: HashMap::new(),
            scene: SceneCache::new(SceneBudget::default()),
            rejected: BTreeMap::new(),
        }
    }

    /// Try to turn one model entity into a template placement.
    ///
    /// `Ok(None)` means "not eligible / fall back"; the reason is counted.
    pub(super) fn try_place(
        &mut self,
        database: &DrawingDatabase,
        entity: &cad_db::DbEntity,
        registry: &ProviderRegistry,
        context: &RepresentationContext,
        stamp: TaskStamp,
    ) -> CadResult<Option<InstancedPlacement>> {
        let SemanticGeometry::Insert {
            block,
            transform,
            attributes,
        } = &entity.geometry
        else {
            return Ok(None);
        };
        if !attributes.is_empty() {
            self.reject("attributes");
            return Ok(None);
        }
        let attributes = database.entity_render_attributes(entity.id);
        if attributes.clip.is_some() {
            self.reject("clip");
            return Ok(None);
        }
        if attributes.annotative.annotative || !attributes.annotative.overrides.is_empty() {
            self.reject("annotative");
            return Ok(None);
        }
        let Some(translation) = pure_translation(transform) else {
            self.reject("not_translation");
            return Ok(None);
        };
        let Some(definition) = database.block(*block) else {
            self.reject("missing_block");
            return Ok(None);
        };
        if definition.dynamic_visibility.is_some() {
            self.reject("dynamic_block");
            return Ok(None);
        }
        // The effective style is part of the shared template: two references
        // with different explicit colours/linetypes/lineweights/transparency
        // must not share one. Nested blocks are allowed; the block-local build
        // expands them once inside the template.
        let key = (*block, entity.layer, format!("{attributes:?}"));
        let template = if let Some(template) = self.cache.get(&key) {
            template.clone()
        } else {
            // Build the block at the identity transform, in its real INSERT's
            // style/layer context. The resulting fragments are block-local and
            // carry the member sources (with this INSERT in the instance path).
            let mut identity = entity.clone();
            identity.geometry = SemanticGeometry::Insert {
                block: *block,
                transform: Transform3::identity(),
                attributes: Vec::new(),
            };
            let representation = registry.build_expanded(database, &identity, context)?;
            if !representation.diagnostics.is_empty()
                || !matches!(&representation.completeness, Completeness::Complete)
            {
                self.reject("representation_diagnostics");
                return Ok(None);
            }
            let delta = self.scene.build_compact(&representation, stamp)?;
            if !delta.images.is_empty() || !delta.masks.is_empty() {
                self.reject("raster_or_mask");
                return Ok(None);
            }
            if delta.added.is_empty() {
                self.reject("empty_template");
                return Ok(None);
            }
            let template: Arc<Vec<Arc<RenderBatch>>> = Arc::new(
                delta
                    .added
                    .into_iter()
                    .map(Arc::new)
                    .collect::<Vec<Arc<RenderBatch>>>(),
            );
            self.cache.insert(key, template.clone());
            template
        };

        let mut batches = Vec::with_capacity(template.len());
        let mut templates = Vec::with_capacity(template.len());
        for (offset, template_batch) in template.iter().enumerate() {
            // The cached template may have been built from another INSERT
            // entity; only the first instance-path element (the reference) is
            // per placement. Member ids and sub-elements are shared.
            let mut sources = template_batch.sources.clone();
            for source in &mut sources {
                if let Some(first) = source.instance.0.first_mut() {
                    *first = entity.id;
                }
            }
            batches.push(template_batch.template_placeholder(
                translation,
                entity.draw_order,
                sources,
            ));
            templates.push((offset, template_batch.clone()));
        }
        Ok(Some(InstancedPlacement { batches, templates }))
    }

    pub(super) fn take_rejections(&self) -> BTreeMap<&'static str, usize> {
        self.rejected.clone()
    }

    fn reject(&mut self, reason: &'static str) {
        *self.rejected.entry(reason).or_default() += 1;
    }
}

/// A pure translation (identity 3x3 + zero projective row) as its offset.
pub(super) fn pure_translation(transform: &Transform3) -> Option<[f64; 3]> {
    let matrix = &transform.matrix;
    for (row, values) in matrix.iter().take(3).enumerate() {
        for (column, value) in values.iter().take(3).enumerate() {
            let expected = if row == column { 1.0 } else { 0.0 };
            if (value - expected).abs() > TRANSLATION_EPSILON {
                return None;
            }
        }
    }
    if matrix[3] != [0.0, 0.0, 0.0, 1.0] {
        return None;
    }
    let translation = [matrix[0][3], matrix[1][3], matrix[2][3]];
    translation
        .iter()
        .all(|value| value.is_finite())
        .then_some(translation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_translation_accepts_identity_and_translation_and_rejects_the_rest() {
        assert_eq!(
            pure_translation(&Transform3::identity()),
            Some([0.0, 0.0, 0.0])
        );
        let translated = Transform3::translation(Point3 {
            x: 1.5,
            y: -2.0,
            z: 3.0,
        });
        assert_eq!(pure_translation(&translated), Some([1.5, -2.0, 3.0]));

        let mut rotated = Transform3::identity();
        rotated.matrix[0][0] = 0.0;
        rotated.matrix[0][1] = -1.0;
        rotated.matrix[1][0] = 1.0;
        rotated.matrix[1][1] = 0.0;
        assert_eq!(pure_translation(&rotated), None);

        let mut scaled = Transform3::identity();
        scaled.matrix[0][0] = 2.0;
        assert_eq!(pure_translation(&scaled), None);

        let mut singular = Transform3::identity();
        singular.matrix[0][0] = 0.0;
        assert_eq!(pure_translation(&singular), None);
    }
}
