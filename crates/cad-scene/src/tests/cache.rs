//! Scene cache publish, eviction, and task queue limits.
//!
//! There is no change-set invalidation path here: a published delta only adds
//! content (see the crate docs). Content removal is the host's retire path plus
//! this cache's budget eviction, covered in `cad-app`'s `retire_tests.rs`.

use super::*;

#[test]
fn publish_rejects_stale_stamp() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, TaskStamp::new(DocumentId(1), 5)).unwrap();
    let current = TaskStamp::new(DocumentId(1), 6);
    assert_eq!(cache.publish(delta, &current), Err(CadError::StaleResult));
    assert_eq!(cache.chunk_count(), 0);
}

#[test]
fn eviction_respects_budget() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 40,
        ..Default::default()
    });
    for i in 0..4 {
        let rep = line_representation(
            i,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
    }
    assert!(cache.used_bytes() <= 40);
    assert!(cache.chunk_count() < 4);
}

#[test]
fn task_queue_rejects_over_capacity_without_dropping() {
    let mut queue = TaskQueue::new(2);
    queue.submit().unwrap();
    queue.submit().unwrap();
    let exceeded = queue.submit().unwrap_err();
    assert_eq!(exceeded.category, "queued_tasks");
    assert_eq!(exceeded.requested, 3);
    assert_eq!(exceeded.limit, 2);
    // The rejected submission did not consume a slot.
    assert_eq!(queue.in_flight(), 2);
    // Completing a task frees exactly one slot.
    queue.complete();
    queue.submit().unwrap();
    assert_eq!(queue.in_flight(), 2);
}

#[test]
fn scene_cache_accounts_cpu_bytes_and_evicts_over_budget() {
    // Each 3-point line batch is 36 vertex bytes. A 40-byte budget keeps one.
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 40,
        ..Default::default()
    });
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.total_cpu_bytes(), 36);
    assert_eq!(cache.chunk_count(), 1);
    // Publishing another 36-byte batch would exceed 40, so the oldest is evicted
    // first: the cache is never above budget, and bytes are accounted for real.
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert!(cache.total_cpu_bytes() <= 40);
    assert_eq!(cache.chunk_count(), 1);
}

#[test]
fn impossible_cache_reservation_preserves_live_chunks_and_accounting() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: 48,
        ..Default::default()
    });
    let delta = cache
        .build(&line_representation(1, unit_points()), stamp())
        .unwrap();
    cache.publish(delta, &stamp()).unwrap();
    let before: Vec<_> = cache.chunks().cloned().collect();
    for required_bytes in [49, usize::MAX] {
        assert!(matches!(
            cache.evict(required_bytes),
            Err(CadError::InvalidInput(_))
        ));
        assert_eq!(cache.chunks().cloned().collect::<Vec<_>>(), before);
        assert_eq!(cache.used_bytes(), 24);
    }
    assert_eq!(cache.evict(24), Ok(0));
    assert_eq!(cache.evict(48), Ok(1));
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn cache_reservation_near_usize_max_does_not_overflow() {
    let mut cache = SceneCache::new(SceneBudget {
        cpu_bytes: usize::MAX,
        ..Default::default()
    });
    let delta = cache
        .build(&line_representation(1, unit_points()), stamp())
        .unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.evict(usize::MAX - 24), Ok(0));
    assert_eq!(cache.used_bytes(), 24);
    assert_eq!(cache.evict(usize::MAX), Ok(1));
    assert_eq!(cache.chunk_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn scene_cpu_bytes_is_charged_on_publish_not_faked() {
    // Two small batches fit; the sum is exactly their counted bytes.
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.used_bytes(), 24);
    let delta = cache.build(&rep, stamp()).unwrap();
    cache.publish(delta, &stamp()).unwrap();
    assert_eq!(cache.used_bytes(), 48);
}
