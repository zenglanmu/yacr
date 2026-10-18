//! Image batch construction and eviction.

use super::*;

fn image_representation(
    entity: u128,
    resource: ResourceKey,
    transform: Transform3,
) -> DisplayRepresentation {
    let mut rep = line_representation(entity, Vec::new());
    rep.fragments[0].primitive = DisplayPrimitive::Image {
        resource,
        transform,
        clip: None,
    };
    rep
}

#[test]
fn image_fragment_becomes_an_image_batch_with_deferred_draw_order() {
    let mut cache = SceneCache::default();
    let resource = ResourceKey("img:logo".into());
    let transform = Transform3::identity();
    let rep = image_representation(7, resource.clone(), transform);
    let delta = cache.build(&rep, stamp()).unwrap();
    // The image is not smuggled into the line/mesh list as a fake batch.
    assert!(delta.added.is_empty());
    assert_eq!(delta.images.len(), 1);
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.image_count(), 1);
    let image = &cache.image_batches()[0];
    assert_eq!(image.resource, resource);
    assert_eq!(image.transform, transform);
    assert_eq!(image.alpha, 1.0);
    // Codifies the CURRENT deferred state: `draw_order` is NOT yet plumbed from
    // `DbEntity::draw_order` through `DisplayFragment` (deferred, see
    // `docs/handoff.md`), so the scene emits `0` and paint order falls back to
    // upload order. This is not a desired behaviour, only a snapshot of the
    // deferral; the assertion will change when threading lands.
    assert_eq!(image.draw_order, 0);
    assert_eq!(image.sources, vec![rep.fragments[0].source.clone()]);
    assert!(cache.image_used_bytes() > 0);
}

#[test]
fn over_budget_image_batches_are_evicted_and_counted() {
    // Learn the CPU size of one image draw item, then size a cache to hold
    // exactly one so a second publish must evict the oldest.
    let mut probe = SceneCache::default();
    let probe_delta = probe
        .build(
            &image_representation(1, ResourceKey("img:1".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    let image_bytes = probe_delta.images[0].approx_bytes();
    probe.publish(probe_delta, &stamp()).unwrap();
    assert_eq!(probe.image_evictions(), 0);

    let budget = SceneBudget {
        image_bytes,
        ..SceneBudget::default()
    };
    let mut cache = SceneCache::new(budget);

    let first = cache
        .build(
            &image_representation(1, ResourceKey("img:1".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    cache.publish(first, &stamp()).unwrap();
    assert_eq!(cache.image_count(), 1);
    assert_eq!(cache.image_evictions(), 0);

    let second = cache
        .build(
            &image_representation(2, ResourceKey("img:2".into()), Transform3::identity()),
            stamp(),
        )
        .unwrap();
    cache.publish(second, &stamp()).unwrap();

    // The oldest image was evicted to stay under the cap and the drop is
    // counted, not silently lost; the most recent image survives.
    assert_eq!(cache.image_count(), 1);
    assert_eq!(cache.image_evictions(), 1);
    assert!(cache.image_used_bytes() <= image_bytes);
    assert_eq!(
        cache.image_batches()[0].resource,
        ResourceKey("img:2".into())
    );
}

#[test]
fn change_set_removes_affected_image_batches() {
    let mut cache = SceneCache::default();
    let rep = image_representation(9, ResourceKey("img:9".into()), Transform3::identity());
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.image_count(), 1);
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(1),
        reason: "edit image".into(),
        changes: vec![ObjectChange::Update(
            ObjectId(9),
            cad_db::ChangeMask::GEOMETRY,
        )],
    };
    cache.apply_changes(&changes).unwrap();
    assert_eq!(cache.image_count(), 0);
    assert_eq!(cache.image_used_bytes(), 0);
}
