//! CPU payload release and forced full-base rebuild entry points.
//!
//! Split out of `mod.rs` so both files stay inside the file-size gate. The
//! controller owns the resident/ready scene and their shared `BatchSet`s; the
//! engine drives the release through its committed-scene gate (ADR-0057), while
//! these helpers exist for controller-level contracts and future hosts.

use super::*;

impl CadSceneController {
    /// Force the next preparation to be a full base build.
    ///
    /// The caller re-prepares from the same live inputs after the engine has
    /// released the CPU payload. Clearing the resident base here is what makes
    /// the build a genuine `full_base`: no append/update/retire shortcut can
    /// reuse the GpuOnly scene, and the new base gets fresh `BatchSet`
    /// allocations (so it is a different resident scene and a new base
    /// revision).
    pub fn force_full_base_next_prepare(&mut self) {
        self.force_full_base = true;
    }

    /// Release the CPU geometry of both the resident base and the ready scene
    /// (they share their `BatchSet` allocations).
    ///
    /// Returns the approximate bytes released. Idempotent. The engine normally
    /// releases the committed scene through its runtime gate; this method exists
    /// for the controller-level contracts and future hosts.
    pub fn release_base_payload(&self) -> usize {
        let mut released = self.base.as_ref().map_or(0, ResidentScene::release_payload);
        if let Some(ready) = &self.ready {
            released = released.saturating_add(ready.base.release_payload());
        }
        released
    }

    /// The one full base build shared by the `base_changed` arm and the
    /// append-decline fallback. It also reports the per-entity ownership of
    /// `added`, which becomes the new index wholesale (a rebuild never patches
    /// the previous one).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn full_base(
        &self,
        doc: Option<&DrawingDatabase>,
        stamp: &TaskStamp,
        fonts: &Option<Arc<FontEngine>>,
        layers: &LayerOverrideSet,
        space: SpaceSelection,
    ) -> CadResult<(ResidentScene, EntityBatchIndex)> {
        match doc {
            Some(db) => {
                let (delta, ranges, templates) =
                    super::assembly::build_scene_with_space_and_images_and_vendor_indexed(
                        db,
                        stamp.clone(),
                        fonts.clone(),
                        self.images.clone(),
                        layers,
                        space,
                        self.vendor.as_deref(),
                        self.display,
                        super::block_instancing_enabled(),
                    )?;
                Ok((
                    ResidentScene::from_owned_with_templates(delta, Arc::new(templates)),
                    EntityBatchIndex::from_ranges(ranges),
                ))
            }
            None => Ok((
                ResidentScene::from_owned(SceneDelta {
                    stamp: stamp.clone(),
                    added: vec![],
                    images: vec![],
                    masks: vec![],
                }),
                EntityBatchIndex::default(),
            )),
        }
    }
}
