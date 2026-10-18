//! Model-space bounds computation and its revision-tagged cache.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::DrawingDatabase;
use crate::bounds::{BoundsAccumulator, MAX_INSTANCE_DEPTH};
use crate::entity::DbEntity;

/// Cached model-space bounds, tagged with the [`Revision`] they describe.
///
/// `bounds()` is called on every transient-overlay rebuild, which happens on
/// every pan/zoom step; recomputing it from scratch is O(#entities) and was
/// ~99% of the per-step CPU cost on large drawings. Keying the cache on the
/// revision means every sanctioned write path (which raises the revision)
/// invalidates it. The one content mutation that preserves the revision,
/// [`DrawingDatabase::restore_entity_display_state`], clears it explicitly.
#[derive(Debug, Default)]
pub(crate) struct BoundsCache {
    /// Visible to the sibling `test_support` module, which reads the tag only.
    pub(super) revision: Option<Revision>,
    bounds: Option<(Point3, Point3)>,
}

/// Height and cycle summary of one block's INSERT subgraph.
///
/// Used to decide whether a block's contribution can be summarized as a single
/// local-space box: a cyclic block, or one whose nesting reaches
/// [`MAX_INSTANCE_DEPTH`], depends on where it is expanded from and is never
/// summarized.
#[derive(Debug, Clone, Copy, Default)]
struct BlockGraphMeta {
    /// Maximum INSERT nesting below the block (0 = no nested inserts), capped
    /// at [`MAX_INSTANCE_DEPTH`].
    height: usize,
    /// Whether the subgraph reachable from the block contains a cycle.
    cyclic: bool,
}

/// A block's bounds in its own local frame, valid only when the block is
/// acyclic and shallower than [`MAX_INSTANCE_DEPTH`].
#[derive(Debug, Clone, Copy)]
struct LocalExpansion {
    bounds: Option<(Point3, Point3)>,
    height: usize,
}

/// Per-`bounds()`-call block metadata and memoized local expansions.
#[derive(Default)]
struct BoundsComputation {
    meta: BTreeMap<BlockId, BlockGraphMeta>,
    memo: BTreeMap<BlockId, LocalExpansion>,
}

/// One block on [`DrawingDatabase::block_graph_meta`]'s explicit DFS stack.
struct BlockFrame {
    block: BlockId,
    dependencies: Vec<BlockId>,
    index: usize,
    height: usize,
    child_cyclic: bool,
}

impl BlockFrame {
    fn new(block: BlockId, dependencies: Vec<BlockId>) -> Self {
        BlockFrame {
            block,
            dependencies,
            index: 0,
            height: 0,
            child_cyclic: false,
        }
    }
}

/// Whether `transform` maps every axis-aligned box exactly to an axis-aligned
/// box, i.e. has an identity linear part (pure translation).
///
/// Only under such a transform may a memoized block box be translated directly;
/// any rotation/scale would make a transformed box conservative rather than
/// exact, so those instances always re-expand the block.
fn is_pure_translation(transform: &Transform3) -> bool {
    let matrix = &transform.matrix;
    for (row, values) in matrix.iter().take(3).enumerate() {
        for (column, value) in values.iter().take(3).enumerate() {
            let expected = if row == column { 1.0 } else { 0.0 };
            if *value != expected {
                return false;
            }
        }
    }
    true
}

impl DrawingDatabase {
    /// World bounds of model-space geometry, expanding INSERT instances.
    ///
    /// The result is memoized per database and tagged with the current
    /// [`Revision`], so a camera move (which changes no content) costs O(1)
    /// after the first call instead of re-scanning every entity. Every mutation
    /// either raises the revision or clears the cache explicitly, so a stale
    /// value can never be served.
    pub fn bounds(&self) -> Option<(Point3, Point3)> {
        {
            let cache = self
                .bounds_cache
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.revision == Some(self.revision) {
                return cache.bounds;
            }
        }
        let computed = self.compute_bounds();
        {
            let mut cache = self
                .bounds_cache
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.revision = Some(self.revision);
            cache.bounds = computed;
        }
        computed
    }

    /// Clear the cached bounds.
    ///
    /// Only needed for content mutations that keep the revision (currently
    /// [`Self::restore_entity_display_state`]); a revision bump invalidates the
    /// revision-keyed cache on its own.
    ///
    /// `pub(super)`: called from the sibling `access`/`edit` impl blocks, which
    /// are separate files of this same module.
    pub(super) fn invalidate_bounds_cache(&self) {
        let mut cache = self
            .bounds_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.revision = None;
        cache.bounds = None;
    }

    /// Recompute bounds from scratch; the cacheable work behind [`Self::bounds`].
    ///
    /// Model-space entities are read directly rather than through
    /// [`Self::model_space`]: bounds accumulation is order-independent, so that
    /// accessor's allocate-and-sort would be wasted work on a hot path.
    fn compute_bounds(&self) -> Option<(Point3, Point3)> {
        let mut acc = BoundsAccumulator::new();
        let mut stack: Vec<BlockId> = Vec::new();
        let mut context = BoundsComputation {
            meta: self.block_graph_meta(),
            memo: BTreeMap::new(),
        };
        for entity in self.entities.values() {
            if matches!(entity.space, SpaceId::Model) {
                self.accumulate_bounds(
                    &mut acc,
                    entity,
                    &Transform3::identity(),
                    0,
                    &mut stack,
                    &mut context,
                );
            }
        }
        acc.finish()
    }

    /// Fold one entity (and any INSERTs it carries) into `acc`.
    ///
    /// Nesting is bounded and cycles are cut so a malformed block graph cannot
    /// loop forever; a truncated/cyclic branch simply contributes no bounds.
    fn accumulate_bounds(
        &self,
        acc: &mut BoundsAccumulator,
        entity: &DbEntity,
        transform: &Transform3,
        depth: usize,
        stack: &mut Vec<BlockId>,
        context: &mut BoundsComputation,
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
            self.accumulate_block_bounds(acc, *block, &composed, depth, stack, context);
        } else {
            acc.add_geometry_transformed(&entity.geometry, transform);
        }
    }

    /// Fold one block instance into `acc`.
    ///
    /// When the instance transform is a pure translation and the block's whole
    /// expansion fits the depth budget, the block's memoized local bounds are
    /// translated as a box — exact for translations — instead of re-scanning the
    /// block's members for each INSERT instance.
    ///
    /// Under nested translated blocks the memo composes the offsets as
    /// `(local + inner) + outer` where the recursive path composes
    /// `local + (inner + outer)`; the two agree except for floating-point
    /// rounding (sub-ULP), which cannot change the containment, only the last
    /// bit of a coordinate.
    fn accumulate_block_bounds(
        &self,
        acc: &mut BoundsAccumulator,
        block: BlockId,
        transform: &Transform3,
        depth: usize,
        stack: &mut Vec<BlockId>,
        context: &mut BoundsComputation,
    ) {
        if depth >= MAX_INSTANCE_DEPTH || stack.contains(&block) {
            return;
        }
        if is_pure_translation(transform) {
            if let Some(expansion) = self.block_local_bounds(block, context) {
                if depth + expansion.height < MAX_INSTANCE_DEPTH {
                    if let Some((min, max)) = expansion.bounds {
                        acc.add_point(transform.apply_point(min));
                        acc.add_point(transform.apply_point(max));
                    }
                    return;
                }
            }
        }
        stack.push(block);
        self.expand_block_members(acc, block, transform, depth + 1, stack, context);
        stack.pop();
    }

    /// Fold the visible members of `block` into `acc` with `transform`.
    ///
    /// Members are read from the block's own list rather than
    /// [`Self::block_entities`]: bounds are order-independent, so the sort and
    /// allocation that accessor performs are unnecessary here.
    fn expand_block_members(
        &self,
        acc: &mut BoundsAccumulator,
        block: BlockId,
        transform: &Transform3,
        member_depth: usize,
        stack: &mut Vec<BlockId>,
        context: &mut BoundsComputation,
    ) {
        let Some(definition) = self.blocks.get(&block) else {
            return;
        };
        for member in &definition.entities {
            if !self.block_member_visible(block, *member) {
                continue;
            }
            if let Some(entity) = self.entities.get(member) {
                self.accumulate_bounds(acc, entity, transform, member_depth, stack, context);
            }
        }
    }

    /// Whether a block member is visible under the block's active visibility
    /// state (an unresolved/absent state leaves every member visible).
    fn block_member_visible(&self, block: BlockId, member: EntityId) -> bool {
        self.blocks
            .get(&block)
            .and_then(|definition| definition.dynamic_visibility.as_ref())
            .map(|visibility| visibility.is_visible(member))
            .unwrap_or(true)
    }

    /// Bounds of `block` in its own local frame, memoized for this computation.
    ///
    /// `None` when the block's expansion is cyclic or reaches
    /// [`MAX_INSTANCE_DEPTH`] from the origin: such a block's contribution
    /// depends on where it is expanded from, so it is never summarized.
    fn block_local_bounds(
        &self,
        block: BlockId,
        context: &mut BoundsComputation,
    ) -> Option<LocalExpansion> {
        if let Some(expansion) = context.memo.get(&block) {
            return Some(*expansion);
        }
        let meta = context.meta.get(&block).copied().unwrap_or_default();
        if meta.cyclic || meta.height >= MAX_INSTANCE_DEPTH {
            return None;
        }
        let mut acc = BoundsAccumulator::new();
        let mut stack: Vec<BlockId> = vec![block];
        self.expand_block_members(
            &mut acc,
            block,
            &Transform3::identity(),
            1,
            &mut stack,
            context,
        );
        stack.pop();
        let expansion = LocalExpansion {
            bounds: acc.finish(),
            height: meta.height,
        };
        context.memo.insert(block, expansion);
        Some(expansion)
    }

    /// Height and cycle summary of every block's INSERT subgraph.
    ///
    /// Traversed iteratively so a pathologically deep block chain cannot
    /// overflow the stack. A back edge marks every block on the current path as
    /// cyclic; a finished child's summary propagates its height and cycle flag.
    fn block_graph_meta(&self) -> BTreeMap<BlockId, BlockGraphMeta> {
        let mut meta: BTreeMap<BlockId, BlockGraphMeta> = BTreeMap::new();
        // 0 = unvisited, 1 = on the current DFS path, 2 = finished.
        let mut color: BTreeMap<BlockId, u8> = BTreeMap::new();
        let mut cyclic: BTreeSet<BlockId> = BTreeSet::new();

        for start in self.blocks.keys().copied() {
            if color.get(&start).copied().unwrap_or(0) == 2 {
                continue;
            }
            let mut frames: Vec<BlockFrame> =
                vec![BlockFrame::new(start, self.block_dependencies(start))];
            color.insert(start, 1);
            while let Some(top) = frames.len().checked_sub(1) {
                let next = {
                    let frame = &mut frames[top];
                    if frame.index < frame.dependencies.len() {
                        let child = frame.dependencies[frame.index];
                        frame.index += 1;
                        Some(child)
                    } else {
                        None
                    }
                };
                match next {
                    Some(child) => match color.get(&child).copied().unwrap_or(0) {
                        0 => {
                            color.insert(child, 1);
                            frames.push(BlockFrame::new(child, self.block_dependencies(child)));
                        }
                        1 => {
                            // Back edge: every block from `child` down the current
                            // path is part of a cycle.
                            if let Some(position) = frames.iter().position(|f| f.block == child) {
                                for frame in &frames[position..] {
                                    cyclic.insert(frame.block);
                                }
                            }
                            frames[top].child_cyclic = true;
                        }
                        _ => {
                            let child_meta = meta.get(&child).copied().unwrap_or_default();
                            let frame = &mut frames[top];
                            frame.child_cyclic |= child_meta.cyclic;
                            let height =
                                child_meta.height.saturating_add(1).min(MAX_INSTANCE_DEPTH);
                            frame.height = frame.height.max(height);
                        }
                    },
                    None => {
                        let frame = frames.pop().expect("frame is present");
                        color.insert(frame.block, 2);
                        let is_cyclic = frame.child_cyclic || cyclic.contains(&frame.block);
                        if is_cyclic {
                            cyclic.insert(frame.block);
                        }
                        let node_meta = BlockGraphMeta {
                            cyclic: is_cyclic,
                            height: if is_cyclic { 0 } else { frame.height },
                        };
                        meta.insert(frame.block, node_meta);
                        // Credit the finished child to its parent: the parent's
                        // `height` is only known once the child is popped.
                        if let Some(parent) = frames.last_mut() {
                            parent.child_cyclic |= node_meta.cyclic;
                            let height = node_meta.height.saturating_add(1).min(MAX_INSTANCE_DEPTH);
                            parent.height = parent.height.max(height);
                        }
                    }
                }
            }
        }
        meta
    }

    /// The blocks directly INSERTed by the members of `block`.
    fn block_dependencies(&self, block: BlockId) -> Vec<BlockId> {
        let Some(definition) = self.blocks.get(&block) else {
            return Vec::new();
        };
        let mut dependencies = Vec::new();
        for member in &definition.entities {
            if let Some(entity) = self.entities.get(member) {
                if let SemanticGeometry::Insert { block: child, .. } = &entity.geometry {
                    dependencies.push(*child);
                }
            }
        }
        dependencies
    }
}
