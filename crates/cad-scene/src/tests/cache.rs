//! Scene cache publish, change-set invalidation, eviction, and task queue limits.

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
fn change_set_removes_affected_chunks() {
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
    assert_eq!(cache.chunk_count(), 1);
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(1),
        reason: "edit".into(),
        changes: vec![ObjectChange::Update(
            ObjectId(1),
            cad_db::ChangeMask::GEOMETRY,
        )],
    };
    cache.apply_changes(&changes).unwrap();
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

#[test]
fn visibility_switch_invalidates_only_the_delta_members() {
    // Three member chunks are on the CPU cache: 50, 51 and 52. A visibility
    // switch A -> B removes member 50's geometry and adds member 52's; 51 is
    // common to both states and must stay cached.
    let mut cache = SceneCache::default();
    for entity in [50u128, 51, 52] {
        let rep = line_representation(
            entity,
            vec![
                Point3 {
                    x: entity as f64,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: entity as f64,
                    y: 1.0,
                    z: 0.0,
                },
            ],
        );
        let delta = cache.build(&rep, stamp()).unwrap();
        cache.publish(delta, &stamp()).unwrap();
    }
    assert_eq!(cache.chunk_count(), 3);

    // The exact ChangeSet the database publishes for switching A -> B:
    // member 50 leaves, member 52 enters, member 51 is untouched.
    let changes = ChangeSet {
        database: DatabaseId(1),
        before: Revision(0),
        after: Revision(1),
        transaction: TransactionId(9),
        reason: "dynamic visibility A -> B".into(),
        changes: vec![
            ObjectChange::Update(ObjectId(50), cad_db::ChangeMask::GEOMETRY),
            ObjectChange::Update(ObjectId(52), cad_db::ChangeMask::GEOMETRY),
        ],
    };
    cache.apply_changes(&changes).unwrap();

    // Only member 51's chunk survives; the changed members were invalidated.
    assert_eq!(cache.chunk_count(), 1);
    let survivor = cache.chunks().next().unwrap();
    assert_eq!(survivor.sources[0].entity, EntityId(51));
}

// ---- Image draw items (spec §4.8 image primitives) ----
