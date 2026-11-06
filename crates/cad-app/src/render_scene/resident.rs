//! The resident CPU base: one shared prefix plus the segments appended since it.
//!
//! A full build produces a prefix and no segments. An accepted incremental
//! append pushes one segment; the prefix is never re-materialised, which is
//! what makes the append O(changed) instead of O(scene).
//!
//! # Per-batch `Arc` payload
//!
//! The batch payload is stored as **one `Arc<RenderBatch>` per batch** inside
//! each segment, while the `Arc<SceneDelta>` is retained for the facts that are
//! still whole-segment (the stamp) and for the prefix's raster draw items. The
//! granularity is what makes a transform-only update possible: replacing one
//! entity's batch ([`ResidentScene::replace_batch`]) swaps a single `Arc` and
//! never copies the other batches of the prefix or the segment.
//!
//! # Invariants
//!
//! * `appended` segments never carry `images`/`masks`. The append gate
//!   (`super::appendable_delta`) refuses such deltas, because the engine's append
//!   path deliberately keeps the resident textures/masks (it uploads them only
//!   for a non-append base change). [`ResidentScene::images`] /
//!   [`ResidentScene::masks`] therefore always come from the prefix.
//! * a segment is only ever appended; a full build replaces the whole resident
//!   scene (`from_prefix`), so a stale segment can never survive a rebuild.
//! * identity is pointer identity of the prefix, of every segment's
//!   `Arc<SceneDelta>`, and of every per-batch `Arc`
//!   ([`ResidentScene::same_as`]), never content equality. A replaced batch is
//!   therefore a different resident scene, which is exactly what the engine's
//!   append admission must observe.
use std::ops::Range;
use std::sync::Arc;

use cad_domain::TaskStamp;
use cad_scene::{ImageBatch, MaskBatch, RenderBatch, SceneDelta};

/// One segment appended after the prefix: the committed delta it came from
/// (retained for its stamp) plus its batches, one `Arc` per batch.
#[derive(Clone)]
struct Segment {
    delta: Arc<SceneDelta>,
    batches: Vec<Arc<RenderBatch>>,
}

/// The committed base: one shared prefix plus the segments appended since it.
///
/// A full build produces a prefix and no segments. An accepted incremental
/// append pushes one segment; the prefix is never re-materialised, which is
/// what makes the append O(changed) instead of O(scene).
#[derive(Clone)]
pub struct ResidentScene {
    prefix: Arc<SceneDelta>,
    /// The prefix's batches, one `Arc` per batch, in resident order.
    prefix_batches: Vec<Arc<RenderBatch>>,
    appended: Vec<Segment>,
}

impl ResidentScene {
    /// A resident scene made of one prefix and no appended segments (a full
    /// build), **consuming** the delta.
    ///
    /// The batches move into their per-batch `Arc`s (no deep copy), and the
    /// retained prefix `SceneDelta` keeps the whole-segment facts (`stamp`,
    /// `images`, `masks`) with `added` emptied: the batch
    /// payload is the per-batch `Arc` list, so the scene never stores every batch
    /// twice. Read the batches through [`Self::batches`] /
    /// [`Self::materialized`], never through the prefix delta.
    pub fn from_owned(delta: SceneDelta) -> Self {
        let SceneDelta {
            stamp,
            added,
            images,
            masks,
        } = delta;
        let prefix_batches = added.into_iter().map(Arc::new).collect();
        Self {
            prefix: Arc::new(SceneDelta {
                stamp,
                added: Vec::new(),
                images,
                masks,
            }),
            prefix_batches,
            appended: Vec::new(),
        }
    }

    /// [`Self::from_owned`] for callers that hold the delta behind an `Arc`.
    ///
    /// The batches are cloned once (the `Arc` cannot be consumed), so the
    /// controller's full build uses [`Self::from_owned`] instead. The retained
    /// prefix delta still has `added` emptied — see [`Self::from_owned`].
    pub fn from_prefix(prefix: Arc<SceneDelta>) -> Self {
        Self::from_owned((*prefix).clone())
    }

    /// Append one committed segment without touching the prefix.
    ///
    /// The segment must not grow the resident raster sets: the engine does not
    /// re-upload images/masks on an append, so an appended `ImageBatch`/
    /// `MaskBatch` would be a silent partial success.
    ///
    /// The per-entity delta is converted to one `Arc` per batch. That copy is
    /// O(appended), i.e. O(one entity), never O(scene): the whole point of the
    /// segment payload.
    pub fn push_appended(&mut self, segment: Arc<SceneDelta>) {
        debug_assert!(
            segment.images.is_empty() && segment.masks.is_empty(),
            "an appended segment must not carry images/masks (the append path never \
             re-uploads them)"
        );
        let batches = segment.added.iter().cloned().map(Arc::new).collect();
        self.appended.push(Segment {
            delta: segment,
            batches,
        });
    }

    /// The shared prefix's whole-segment facts. The engine retains this exact
    /// `Arc` as its applied base identity, and the renderer reads `images`/
    /// `masks` from it.
    ///
    /// Its `added` list is **empty by construction**: the batch payload is the
    /// per-batch `Arc` list (see [`Self::from_owned`]). Use [`Self::batches`] or
    /// [`Self::materialized`] for the batches.
    pub fn prefix_delta(&self) -> &Arc<SceneDelta> {
        &self.prefix
    }

    /// Number of segments appended after the prefix.
    pub fn appended_len(&self) -> usize {
        self.appended.len()
    }

    /// Total number of resident base batches across the prefix and every segment.
    pub fn batch_len(&self) -> usize {
        self.prefix_batches.len()
            + self
                .appended
                .iter()
                .map(|segment| segment.batches.len())
                .sum::<usize>()
    }

    /// Every resident batch, in resident order, flattened across segments.
    pub fn batches(&self) -> impl Iterator<Item = &RenderBatch> {
        self.prefix_batches.iter().map(Arc::as_ref).chain(
            self.appended
                .iter()
                .flat_map(|segment| segment.batches.iter().map(Arc::as_ref)),
        )
    }

    /// Materialise a bounded batch range as owned [`RenderBatch`] values.
    ///
    /// The engine stages at most 256 batches / 262,144 vertices per piece, so
    /// cloning the contents of one bounded piece costs O(piece) and is accepted:
    /// an append piece is exactly one entity, and a full re-stage (device loss,
    /// a declined append, a refused overwrite) is already the slow path. Nothing
    /// here is on the per-frame path.
    ///
    /// `range.end` must not exceed [`Self::batch_len`]; anything beyond the end
    /// is clipped (never fabricated).
    pub fn materialized(&self, range: Range<usize>) -> Vec<RenderBatch> {
        let len = self.batch_len();
        let start = range.start.min(len);
        let end = range.end.min(len);
        if start >= end {
            return Vec::new();
        }
        (start..end)
            .filter_map(|index| self.batch_at(index))
            .cloned()
            .collect()
    }

    /// The resident batch at `index`, in resident order.
    fn batch_at(&self, index: usize) -> Option<&RenderBatch> {
        if let Some(batch) = self.prefix_batches.get(index) {
            return Some(batch);
        }
        let mut cursor = self.prefix_batches.len();
        for segment in &self.appended {
            if index < cursor + segment.batches.len() {
                return segment.batches.get(index - cursor).map(Arc::as_ref);
            }
            cursor += segment.batches.len();
        }
        None
    }

    /// Replace the resident batch at `index` with `batch`, in O(1) (plus the
    /// O(#segments) segment lookup): no other batch is cloned or moved.
    ///
    /// Returns `false` when `index` is out of range, in which case nothing is
    /// modified. The caller must have validated the whole replacement first: a
    /// batch swap is not reversible from here.
    ///
    /// A replacement changes the resident scene's identity ([`Self::same_as`])
    /// because the per-batch `Arc` differs. That is deliberate: the engine's
    /// exact-predecessor append admission must not treat a scene whose geometry
    /// was overwritten as the allocation it last committed.
    pub fn replace_batch(&mut self, index: usize, batch: Arc<RenderBatch>) -> bool {
        if let Some(slot) = self.prefix_batches.get_mut(index) {
            *slot = batch;
            return true;
        }
        let mut cursor = self.prefix_batches.len();
        for segment in &mut self.appended {
            if index < cursor + segment.batches.len() {
                segment.batches[index - cursor] = batch;
                return true;
            }
            cursor += segment.batches.len();
        }
        false
    }

    /// The resident image draw items. Always the prefix's: an appended segment
    /// never carries any.
    pub fn images(&self) -> &[ImageBatch] {
        &self.prefix.images
    }

    /// The resident wipeout masks. Always the prefix's: an appended segment never
    /// carries any.
    pub fn masks(&self) -> &[MaskBatch] {
        &self.prefix.masks
    }

    /// The stamp of the newest committed content: the last appended segment's
    /// stamp, or the prefix's when nothing was appended.
    pub fn stamp(&self) -> &TaskStamp {
        self.appended
            .last()
            .map_or(&self.prefix.stamp, |segment| &segment.delta.stamp)
    }

    /// Whether `other` is the **same** resident scene: the same prefix allocation,
    /// the same segment allocations in the same order, and the same per-batch
    /// `Arc` allocations.
    ///
    /// Identity is pointers, not contents: a re-published prefix with identical
    /// batches is a different base (its GPU-committed range is a different
    /// allocation), an accepted append keeps the predecessor's prefix and adds
    /// one segment, and an in-place batch overwrite replaces one per-batch `Arc`.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.prefix, &other.prefix)
            && self.prefix_batches.len() == other.prefix_batches.len()
            && self
                .prefix_batches
                .iter()
                .zip(&other.prefix_batches)
                .all(|(left, right)| Arc::ptr_eq(left, right))
            && self.appended.len() == other.appended.len()
            && self
                .appended
                .iter()
                .zip(&other.appended)
                .all(|(left, right)| {
                    Arc::ptr_eq(&left.delta, &right.delta)
                        && left.batches.len() == right.batches.len()
                        && left
                            .batches
                            .iter()
                            .zip(&right.batches)
                            .all(|(a, b)| Arc::ptr_eq(a, b))
                })
    }

    /// Materialise the whole resident scene as one [`SceneDelta`].
    ///
    /// O(scene) by construction: for byte-identity checks against a full build
    /// (tests, exporters), never on the append path.
    pub fn to_delta(&self) -> SceneDelta {
        SceneDelta {
            stamp: self.stamp().clone(),
            added: self.batches().cloned().collect(),
            images: self.prefix.images.clone(),
            masks: self.prefix.masks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostController;
    use crate::layers::LayerOverrideSet;
    use crate::render_scene::{CadSceneController, DrawingEditReceipt, OverlayInputs};
    use cad_domain::{DocumentId, Point3};
    use cad_representation::SpaceSelection;

    fn empty_prefix() -> Arc<SceneDelta> {
        Arc::new(SceneDelta {
            stamp: TaskStamp::new(DocumentId(1), 0),
            added: Vec::new(),
            images: Vec::new(),
            masks: Vec::new(),
        })
    }

    #[test]
    fn same_as_is_pointer_identity_not_content_identity() {
        let prefix = empty_prefix();
        let other_prefix = Arc::new((*prefix).clone());

        let resident = ResidentScene::from_prefix(prefix.clone());
        assert!(
            resident.same_as(&resident.clone()),
            "a clone shares the prefix and the segments"
        );
        assert!(
            !resident.same_as(&ResidentScene::from_prefix(other_prefix)),
            "a re-published prefix with identical content is a different base"
        );

        let mut appended = resident.clone();
        appended.push_appended(Arc::new((*prefix).clone()));
        assert!(
            !resident.same_as(&appended),
            "a pushed segment makes it a different resident scene"
        );
        assert!(
            !appended.same_as(&resident),
            "identity is not one-directional"
        );
        assert!(
            appended.same_as(&appended.clone()),
            "the same segment allocation is the same resident scene"
        );

        let mut other_segment = resident.clone();
        other_segment.push_appended(Arc::new((*prefix).clone()));
        assert!(
            !appended.same_as(&other_segment),
            "equal-length segment lists with distinct allocations are different"
        );

        // Replacing one batch swaps one `Arc`: every other batch is shared, but
        // the scene is a different one.
        let mut with_batches = ResidentScene::from_prefix(Arc::new(SceneDelta {
            added: batches(2),
            ..(*prefix).clone()
        }));
        let before = with_batches.clone();
        assert!(
            with_batches.replace_batch(1, Arc::new(batch(9))),
            "index 1 is in range"
        );
        assert!(
            !with_batches.same_as(&before),
            "a replaced per-batch Arc makes it a different resident scene"
        );
        assert!(
            Arc::ptr_eq(&with_batches.prefix_batches[0], &before.prefix_batches[0]),
            "replace_batch must not clone or replace the other batches"
        );
    }

    #[test]
    fn replace_batch_is_bounded_and_never_creates_a_gap() {
        let mut prefix = (*empty_prefix()).clone();
        prefix.added = batches(2);
        let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
        resident.push_appended(Arc::new(segment(1, DocumentId(2))));
        assert_eq!(resident.batch_len(), 3);

        assert!(
            !resident.replace_batch(3, Arc::new(batch(3))),
            "out of range is a refusal, never a silent append"
        );
        assert_eq!(resident.batch_len(), 3, "a refused replace changes nothing");
        assert!(resident.replace_batch(2, Arc::new(batch(7))));
        assert_eq!(
            resident.batch_len(),
            3,
            "a replace never changes the length"
        );
        assert_eq!(
            resident.batch_at(2).map(|batch| batch.draw_order),
            Some(7),
            "the replaced batch is the one at that resident index"
        );
    }

    #[test]
    fn materialized_covers_the_range_across_segments() {
        let mut prefix = (*empty_prefix()).clone();
        prefix.added = batches(2);
        let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
        resident.push_appended(Arc::new(segment(1, DocumentId(2))));
        resident.push_appended(Arc::new(segment(2, DocumentId(2))));
        assert_eq!(resident.batch_len(), 5);

        let draws = |range: Range<usize>| -> Vec<i64> {
            resident
                .materialized(range)
                .iter()
                .map(|batch| batch.draw_order)
                .collect()
        };
        assert_eq!(
            draws(0..5),
            vec![0, 1, 0, 0, 1],
            "prefix, then each segment"
        );
        assert_eq!(
            draws(3..5),
            vec![0, 1],
            "an append range is exactly the appended segment"
        );
        assert_eq!(
            draws(1..4),
            vec![1, 0, 0],
            "a spanning range is concatenated"
        );
        assert_eq!(draws(2..2), Vec::<i64>::new());
        assert_eq!(
            draws(4..99),
            vec![1],
            "a range past the end is clipped, never fabricated"
        );
        assert_eq!(
            resident.stamp().document,
            DocumentId(2),
            "the last segment's stamp is the newest committed stamp"
        );
    }

    fn batches(count: usize) -> Vec<RenderBatch> {
        (0..count).map(batch).collect()
    }

    fn batch(index: usize) -> RenderBatch {
        let x = index as f32 * 0.25;
        RenderBatch {
            conics: Vec::new(),
            local_origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            topology: cad_scene::RenderTopology::Lines,
            vertices: vec![
                [x, 0.0, 0.0],
                [x + 0.1, 0.1, 0.0],
                [x + 0.1, 0.1, 0.0],
                [x, 0.2, 0.0],
            ],
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            edges: Vec::new(),
            mirrored: false,
            alpha: 1.0,
            color: cad_scene::DEFAULT_BATCH_COLOR,
            color_unresolved: true,
            lineweight: 0.0,
            lineweight_unresolved: true,
            linetype: cad_scene::LinetypePattern::continuous(),
            linetype_unresolved: true,
            sources: Vec::new(),
            draw_order: index as i64,
        }
    }

    fn segment(count: usize, document: DocumentId) -> SceneDelta {
        SceneDelta {
            stamp: TaskStamp::new(document, 2),
            added: batches(count),
            images: Vec::new(),
            masks: Vec::new(),
        }
    }

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn prepare(
        controller: &mut CadSceneController,
        db: Arc<cad_db::DrawingDatabase>,
        edit: Option<&DrawingEditReceipt>,
    ) {
        controller
            .prepare_shared_with_overlays_and_images_and_edit(
                Some(db),
                DocumentId(1),
                None,
                &LayerOverrideSet::new(),
                SpaceSelection::Model,
                &OverlayInputs::default(),
                None,
                edit,
            )
            .unwrap();
    }

    #[test]
    fn append_pushes_a_segment_without_touching_the_prefix() {
        let mut host = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let mut drawing = host.drawing().unwrap();
        let mut controller = CadSceneController::default();
        prepare(&mut controller, drawing.clone(), None);

        let prefix = controller
            .ready
            .as_ref()
            .unwrap()
            .base
            .prefix_delta()
            .clone();
        let prefix_batches = controller.ready.as_ref().unwrap().base.batch_len();
        let mut expected_batches = prefix_batches;
        let mut appended = 0;

        for step in 0..3 {
            host.application
                .workspace
                .documents
                .get_mut(&DocumentId(1))
                .unwrap()
                .drawing = drawing;
            let (trace, receipt) = host
                .execute_traced_with_edit(crate::Command {
                    schema_version: 1,
                    id: crate::CommandId::CreateLine,
                    document: DocumentId(1),
                    viewport: cad_domain::ViewportId(1),
                    payload: crate::CommandPayload::Points(vec![
                        p(step as f64 * 10.0, 0.0),
                        p(step as f64 * 10.0 + 5.0, 5.0),
                    ]),
                })
                .unwrap();
            trace.result.unwrap();
            let receipt = receipt.expect("a single model insert is appendable");
            drawing = host.drawing().unwrap();
            prepare(&mut controller, drawing.clone(), Some(&receipt));

            let ready = controller.ready.as_ref().unwrap();
            assert!(
                ready.base_append.is_some(),
                "step {step}: the append must be accepted"
            );
            let append = ready.base_append.as_ref().unwrap();
            assert_eq!(
                append.keep_batches, expected_batches,
                "step {step}: the prefix is exactly the previous total"
            );
            assert!(
                Arc::ptr_eq(ready.base.prefix_delta(), &prefix),
                "step {step}: the append must not re-wrap the prefix Arc"
            );
            appended += 1;
            assert_eq!(
                ready.base.appended_len(),
                appended,
                "step {step}: exactly one segment per accepted append"
            );
            let added = ready.base.batch_len() - expected_batches;
            assert!(added > 0, "step {step}: the inserted line draws");
            expected_batches = ready.base.batch_len();
        }

        assert_eq!(expected_batches, prefix_batches + appended);
        assert!(
            prefix.added.is_empty(),
            "the prefix delta carries no duplicate batch payload"
        );
    }
}
