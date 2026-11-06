//! The resident CPU base: one shared prefix plus the segments appended since it.
//!
//! A full build produces a prefix and no segments. An accepted incremental
//! append pushes one segment; the prefix `Arc` is never re-materialised, which is
//! what makes the append O(changed) instead of O(scene).
//!
//! Invariants:
//!
//! * `appended` segments never carry `images`/`masks`. The append gate
//!   (`super::appendable_delta`) refuses such deltas, because the engine's append
//!   path deliberately keeps the resident textures/masks (it uploads them only
//!   for a non-append base change). [`ResidentScene::images`] /
//!   [`ResidentScene::masks`] therefore always come from the prefix.
//! * a segment is only ever appended; a full build replaces the whole resident
//!   scene (`from_prefix`), so a stale segment can never survive a rebuild.
//! * identity is pointer identity of the prefix plus pointer identity of every
//!   segment ([`ResidentScene::same_as`]), never content equality.
use std::ops::Range;
use std::sync::Arc;

use cad_domain::TaskStamp;
use cad_scene::{ImageBatch, MaskBatch, RenderBatch, SceneDelta};

/// The committed base: one shared prefix plus the segments appended since it.
///
/// A full build produces a prefix and no segments. An accepted incremental
/// append pushes one segment; the prefix `Arc` is never re-materialised, which is
/// what makes the append O(changed) instead of O(scene).
#[derive(Clone)]
pub struct ResidentScene {
    prefix: Arc<SceneDelta>,
    appended: Vec<Arc<SceneDelta>>,
}

impl ResidentScene {
    /// A resident scene made of one prefix and no appended segments (a full
    /// build).
    pub fn from_prefix(prefix: Arc<SceneDelta>) -> Self {
        Self {
            prefix,
            appended: Vec::new(),
        }
    }

    /// Append one committed segment without touching the prefix.
    ///
    /// The segment must not grow the resident raster sets: the engine does not
    /// re-upload images/masks on an append, so an appended `ImageBatch`/
    /// `MaskBatch` would be a silent partial success.
    pub fn push_appended(&mut self, segment: Arc<SceneDelta>) {
        debug_assert!(
            segment.images.is_empty() && segment.masks.is_empty(),
            "an appended segment must not carry images/masks (the append path never \
             re-uploads them)"
        );
        self.appended.push(segment);
    }

    /// The shared prefix. The engine retains this exact `Arc` as its applied base
    /// identity, and the renderer reads `images`/`masks` from it.
    pub fn prefix(&self) -> &Arc<SceneDelta> {
        &self.prefix
    }

    /// The segments appended since the prefix, in append order.
    pub fn appended(&self) -> &[Arc<SceneDelta>] {
        &self.appended
    }

    /// Total number of resident base batches across the prefix and every segment.
    pub fn batch_len(&self) -> usize {
        self.prefix.added.len()
            + self
                .appended
                .iter()
                .map(|segment| segment.added.len())
                .sum::<usize>()
    }

    /// The resident batches as the segments that hold them, in resident order.
    ///
    /// The prefix is always yielded first (even when empty), so a full build is
    /// exactly one slice.
    pub fn segments(&self) -> impl Iterator<Item = &[RenderBatch]> {
        std::iter::once(self.prefix.added.as_slice())
            .chain(self.appended.iter().map(|segment| segment.added.as_slice()))
    }

    /// Every resident batch, in resident order, flattened across segments.
    pub fn batches(&self) -> impl Iterator<Item = &RenderBatch> {
        self.segments().flatten()
    }

    /// The sub-slices that cover `range`, in resident order (segment-aware
    /// slicing).
    ///
    /// A range inside one segment yields exactly one slice — which is every
    /// accepted append (`keep_batches..batch_len()` is exactly the newly pushed
    /// segment) and every full build. A range spanning segments yields one slice
    /// per segment intersection, so the caller can stage them in order.
    ///
    /// `range.end` must not exceed [`Self::batch_len`]; anything beyond the end
    /// is clipped (never fabricated).
    pub fn slices(&self, range: Range<usize>) -> impl Iterator<Item = &[RenderBatch]> {
        let mut cursor = 0usize;
        self.segments().filter_map(move |segment| {
            let segment_start = cursor;
            cursor += segment.len();
            let start = range.start.max(segment_start);
            let end = range.end.min(cursor);
            (start < end).then(|| &segment[start - segment_start..end - segment_start])
        })
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
            .map_or(&self.prefix.stamp, |segment| &segment.stamp)
    }

    /// Whether `other` is the **same** resident scene: the same prefix allocation
    /// and the same segment allocations in the same order.
    ///
    /// Identity is pointers, not contents: a re-published prefix with identical
    /// batches is a different base (its GPU-committed range is a different
    /// allocation), while an accepted append keeps the predecessor's prefix and
    /// adds one segment.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.prefix, &other.prefix)
            && self.appended.len() == other.appended.len()
            && self
                .appended
                .iter()
                .zip(&other.appended)
                .all(|(left, right)| Arc::ptr_eq(left, right))
    }

    /// Materialise the whole resident scene as one [`SceneDelta`].
    ///
    /// O(scene) by construction: for byte-identity checks against a full build
    /// (tests, exporters), never on the append path.
    pub fn to_delta(&self) -> SceneDelta {
        SceneDelta {
            stamp: self.stamp().clone(),
            added: self.batches().cloned().collect(),
            removed_chunks: self.prefix.removed_chunks.clone(),
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
            removed_chunks: Vec::new(),
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
    }

    #[test]
    fn slices_cover_the_range_across_segments() {
        let mut prefix = (*empty_prefix()).clone();
        prefix.added = batches(2);
        let mut resident = ResidentScene::from_prefix(Arc::new(prefix));
        resident.push_appended(Arc::new(segment(1, DocumentId(2))));
        resident.push_appended(Arc::new(segment(2, DocumentId(2))));
        assert_eq!(resident.batch_len(), 5);

        let lengths = |range: Range<usize>| -> Vec<usize> {
            resident
                .slices(range)
                .map(|slice| slice.len())
                .collect::<Vec<_>>()
        };
        assert_eq!(lengths(0..5), vec![2, 1, 2], "prefix, then each segment");
        assert_eq!(
            lengths(3..5),
            vec![2],
            "an append range is exactly one slice inside one segment"
        );
        assert_eq!(lengths(1..4), vec![1, 1, 1], "a spanning range is split");
        assert_eq!(lengths(2..2), Vec::<usize>::new());
        assert_eq!(
            lengths(4..99),
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
            removed_chunks: Vec::new(),
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

        let prefix = controller.ready.as_ref().unwrap().base.prefix().clone();
        let prefix_batches = prefix.added.len();
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
                Arc::ptr_eq(ready.base.prefix(), &prefix),
                "step {step}: the append must not re-wrap the prefix Arc"
            );
            appended += 1;
            assert_eq!(
                ready.base.appended().len(),
                appended,
                "step {step}: exactly one segment per accepted append"
            );
            let added = ready.base.batch_len() - expected_batches;
            assert!(added > 0, "step {step}: the inserted line draws");
            expected_batches = ready.base.batch_len();
        }

        assert_eq!(expected_batches, prefix_batches + appended);
        assert_eq!(prefix.added.len(), prefix_batches);
    }
}
