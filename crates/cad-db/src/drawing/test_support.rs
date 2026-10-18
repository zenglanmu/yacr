//! Test-only inspection helpers; the module is compiled only under `cfg(test)`.

use cad_domain::*;

use super::DrawingDatabase;
use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::entity::DbEntity;

#[cfg(test)]
impl DrawingDatabase {
    /// Test-only hook: the revision tag of the currently cached bounds.
    pub(crate) fn cached_bounds_revision_for_test(&self) -> Option<Revision> {
        self.bounds_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision
    }

    /// Reference bounds computed with the original algorithm
    /// (`model_space` + `block_entities` + recursion).
    ///
    /// Contract tests compare this against the optimized [`Self::bounds`] to
    /// prove the cache and the translation memo are semantics-preserving.
    pub(crate) fn bounds_reference_for_test(&self) -> Option<(Point3, Point3)> {
        let mut acc = BoundsAccumulator::new();
        let mut stack: Vec<BlockId> = Vec::new();
        for entity in self.model_space() {
            self.accumulate_bounds_reference(
                &mut acc,
                entity,
                &Transform3::identity(),
                0,
                &mut stack,
            );
        }
        acc.finish()
    }

    fn accumulate_bounds_reference(
        &self,
        acc: &mut BoundsAccumulator,
        entity: &DbEntity,
        transform: &Transform3,
        depth: usize,
        stack: &mut Vec<BlockId>,
    ) {
        if let SemanticGeometry::Insert {
            block,
            transform: insert,
        } = &entity.geometry
        {
            if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
                return;
            }
            let composed = transform.matrix_mul(insert);
            stack.push(*block);
            for child in self.block_entities(*block) {
                self.accumulate_bounds_reference(acc, child, &composed, depth + 1, stack);
            }
            stack.pop();
        } else {
            acc.add_geometry_transformed(&entity.geometry, transform);
        }
    }
}
