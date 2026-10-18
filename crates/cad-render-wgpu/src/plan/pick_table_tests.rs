use super::*;
use cad_domain::{EntityId, InstancePath, SelectionRef};

fn source(entity: u128, instance: &[u128]) -> SelectionRef {
    SelectionRef {
        document: cad_domain::DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath(instance.iter().copied().map(EntityId).collect()),
        sub_element: None,
    }
}

#[test]
fn pick_key_none_is_the_background_and_never_resolves() {
    let table = PickTable::build(vec![(1, vec![source(7, &[])])], true).expect("unique keys");
    assert_eq!(PICK_KEY_NONE, 0);
    assert!(table.resolve(PICK_KEY_NONE).is_none());
    assert!(table.resolve(999).is_none(), "an unknown key is not a hit");
    assert_eq!(table.len(), 1);
}

#[test]
fn a_single_source_key_resolves_exactly_and_preserves_the_full_u128() {
    // An EntityId far beyond 32 bits: the table must return the whole value,
    // not its low 32 bits.
    let entity = (1u128 << 100) | 0xDEAD_BEEF;
    let table = PickTable::build(vec![(3, vec![source(entity, &[])])], true).expect("unique keys");
    let resolved = table.resolve(3).expect("known key");
    assert_eq!(resolved.batch, 0, "batch ordinal is the entry position");
    let hit = resolved.source.expect("exact");
    assert_eq!(hit.entity, EntityId(entity), "full u128 identity preserved");
    assert_eq!(
        hit.entity.0 as u32, 0xDEAD_BEEF,
        "low 32 bits are the truncation"
    );
    assert!(resolved.reason.is_none());
    assert!(!resolved.collision);
    assert!(table.collisions().is_empty());
}

#[test]
fn a_multi_source_batch_is_ambiguous_and_names_every_candidate() {
    // The scene builder concatenates adjacent line fragments of different
    // entities into one batch; the id pass cannot tell them apart, so the
    // resolution must list both and never silently pick the first.
    let a = source(11, &[]);
    let b = source(22, &[]);
    let table = PickTable::build(vec![(1, vec![a.clone(), b.clone()])], true).expect("unique keys");
    let resolved = table.resolve(1).expect("known key");
    assert!(resolved.source.is_none(), "no exact identity");
    assert_eq!(resolved.sources, vec![a, b]);
    assert_eq!(resolved.reason, Some(PICK_REASON_MULTI_SOURCE));
}

#[test]
fn a_batch_with_no_source_resolves_to_no_identity() {
    let table = PickTable::build(vec![(1, Vec::new())], true).expect("unique keys");
    let resolved = table.resolve(1).expect("known key");
    assert!(resolved.source.is_none());
    assert!(resolved.sources.is_empty());
    assert_eq!(resolved.reason, Some(PICK_REASON_NO_SOURCE));
}

#[test]
fn keys_for_source_matches_identity_and_ignores_the_sub_element() {
    let instance_a = source(20, &[10]);
    let instance_b = source(20, &[20]);
    let table = PickTable::build(
        vec![
            (1, vec![instance_a.clone()]),
            (2, vec![instance_b.clone()]),
            (3, vec![source(7, &[])]),
        ],
        true,
    )
    .expect("unique keys");

    // Exact placement match, in key order.
    assert_eq!(table.keys_for_source(&instance_a), vec![1]);
    assert_eq!(table.keys_for_source(&instance_b), vec![2]);
    // The same entity under a different instance is a different placement.
    assert!(table.keys_for_source(&source(20, &[99])).is_empty());
    // A sub-element ref names the same whole entity: the flags path ignores
    // the sub-element and matches the placement's key.
    let mut face = instance_a.clone();
    face.sub_element = Some(cad_domain::SubElementId {
        source_key: "face-a".into(),
        topology_revision: cad_domain::Revision(0),
    });
    assert_eq!(table.keys_for_source(&face), vec![1]);
}

#[test]
fn keys_for_source_returns_a_multi_source_batch_key_once() {
    // A merged batch whose sources are two entities: a flag on either source
    // must map to the batch's single key (batch-granularity limit).
    let a = source(11, &[]);
    let b = source(22, &[]);
    let table = PickTable::build(vec![(5, vec![a.clone(), b.clone()])], true).expect("unique keys");
    assert_eq!(table.keys_for_source(&a), vec![5]);
    assert_eq!(table.keys_for_source(&b), vec![5]);
}

#[test]
fn a_sub_element_batch_source_is_not_matched_by_an_entity_flag() {
    // The sub-element overlay batch's source is `E#face-a`. Selecting the
    // whole entity `E` must not resolve to that batch's key, or the face
    // overlay would be double-tinted (and a future `FLAG_HIDDEN` would
    // discard the face highlight). S2.
    let face = |entity: u128| {
        let mut source = source(entity, &[]);
        source.sub_element = Some(cad_domain::SubElementId {
            source_key: "face-a".into(),
            topology_revision: cad_domain::Revision(0),
        });
        source
    };
    let whole = source(40, &[]);
    let table = PickTable::build(vec![(1, vec![whole.clone()]), (2, vec![face(40)])], true)
        .expect("unique keys");
    assert_eq!(
        table.keys_for_source(&whole),
        vec![1],
        "only the whole-entity batch matches"
    );
    // A batch that carries *only* the face source is never matched by an
    // entity-level ref (nor by a different entity).
    let face_only = PickTable::build(vec![(2, vec![face(40)])], true).expect("unique keys");
    assert!(face_only.keys_for_source(&whole).is_empty());
    assert!(face_only.keys_for_source(&source(41, &[])).is_empty());
}

#[test]
fn distinct_entities_sharing_the_low_32_bits_are_reported_as_a_collision() {
    // Two distinct u128 entities whose low 32 bits agree. The dense key still
    // resolves exactly (no mis-pick), and the truncation is reported.
    let low = 0x0000_1234u128;
    let a = low;
    let b = (1u128 << 64) | low;
    let table = PickTable::build(
        vec![(1, vec![source(a, &[])]), (2, vec![source(b, &[])])],
        true,
    )
    .expect("unique keys");
    assert_eq!(table.collisions().len(), 1);
    let collision = &table.collisions()[0];
    assert_eq!(collision.truncated_key, 0x1234);
    assert_eq!(collision.identities.len(), 2);
    // Both keys still resolve to their own full identity...
    assert_eq!(
        table.resolve(1).unwrap().source.unwrap().entity,
        EntityId(a)
    );
    assert_eq!(
        table.resolve(2).unwrap().source.unwrap().entity,
        EntityId(b)
    );
    // ...and both are flagged as colliding.
    assert!(table.resolve(1).unwrap().collision);
    assert!(table.resolve(2).unwrap().collision);
}

#[test]
fn the_same_entity_under_two_instances_is_a_collision_too() {
    // One block placed twice: the same EntityId, two distinct placements. For
    // a 32-bit consumer they are indistinguishable, so this is reported.
    let a = source(5, &[100]);
    let b = source(5, &[200]);
    let table = PickTable::build(vec![(1, vec![a]), (2, vec![b])], true).expect("unique keys");
    assert_eq!(table.collisions().len(), 1);
    assert_eq!(table.collisions()[0].identities.len(), 2);
}

#[test]
fn distinct_entities_with_distinct_low_bits_are_not_a_collision() {
    let table = PickTable::build(
        vec![(1, vec![source(1, &[])]), (2, vec![source(2, &[])])],
        true,
    )
    .expect("unique keys");
    assert!(table.collisions().is_empty());
    assert!(!table.resolve(1).unwrap().collision);
    assert!(!table.resolve(2).unwrap().collision);
}

#[test]
fn a_repeated_key_is_a_hard_error_not_first_wins() {
    // Keys are allocated monotonically, so a duplicate can only come from a
    // lifecycle bug (the allocator restarted while keys were resident).
    // Silently keeping one owner would resolve a fragment to the wrong
    // entity, so the table refuses to build.
    let duplicate = PickTable::build(
        vec![(1, vec![source(1, &[])]), (1, vec![source(2, &[])])],
        true,
    )
    .expect_err("a duplicate key must not build a table");
    assert_eq!(duplicate.key, 1);
    assert_eq!(duplicate.first, 0);
    assert_eq!(duplicate.second, 1);
}

#[test]
fn max_key_tracks_the_highest_live_key() {
    let table = PickTable::build(
        vec![(4, vec![source(1, &[])]), (9, vec![source(2, &[])])],
        true,
    )
    .expect("unique keys");
    assert_eq!(table.max_key(), 9);
    assert!(table.resolve(5).is_none(), "a gap is not a hit");
}

/// A tiny deterministic PRNG (SplitMix64): the differential sequence is
/// reproducible without pulling a dependency into the crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_source(rng: &mut Rng) -> SelectionRef {
    // Bias toward dense / high-bit-aliased entity ids so low-32 truncation
    // collisions actually occur in the sequence.
    let entity = match rng.below(4) {
        0 => rng.below(8) as u128,
        1 => (rng.below(8) as u128) | (1u128 << 64),
        _ => rng.next() as u128,
    };
    let instance = if rng.below(3) == 0 {
        Vec::new()
    } else {
        vec![cad_domain::EntityId(rng.below(4) as u128)]
    };
    let sub_element = if rng.below(5) == 0 {
        Some(cad_domain::SubElementId {
            source_key: "face-a".into(),
            topology_revision: cad_domain::Revision(0),
        })
    } else {
        None
    };
    SelectionRef {
        document: cad_domain::DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath(instance),
        sub_element,
    }
}

fn random_entry(rng: &mut Rng, key: u32) -> (u32, Vec<SelectionRef>) {
    let count = 1 + rng.below(3) as usize;
    (key, (0..count).map(|_| random_source(rng)).collect())
}

/// The modelled resident set: the base/suffix batches (combined indices
/// `0..batches.len()`) followed by the resident image quads (indices
/// `batches.len()..`), exactly as the renderer feeds `PickTable::build`.
struct Resident {
    batches: Vec<(u32, Vec<SelectionRef>)>,
    images: Vec<(u32, Vec<SelectionRef>)>,
    next_key: u32,
}

impl Resident {
    fn combined(&self) -> Vec<(u32, Vec<SelectionRef>)> {
        self.batches
            .iter()
            .chain(self.images.iter())
            .cloned()
            .collect()
    }
    fn alloc(&mut self) -> u32 {
        let key = self.next_key;
        self.next_key += 1;
        key
    }
}

/// The table a full rebuild would produce for `resident`, with its identity
/// index built (so it compares like-for-like with the maintained table).
fn fresh(resident: &Resident) -> PickTable {
    let mut table = PickTable::build(resident.combined(), false).expect("resident keys are unique");
    table.ensure_source_index();
    table
}

/// The renderer's incremental precondition: no resident image (which would
/// renumber), and the table still covers the kept batch prefix.
fn covers(table: &PickTable, resident: &Resident, prefix: usize) -> bool {
    if prefix == 0 {
        return true;
    }
    match resident.batches.get(prefix - 1) {
        Some((key, _)) => table.batch_index_of_key(*key) == Some(prefix as u32 - 1),
        None => false,
    }
}

#[test]
fn incremental_pick_table_matches_a_fresh_build_across_commit_sequences() {
    // Randomized commit sequences (append-only, suffix replacement, image
    // re-upload, reset). After every step the incrementally maintained table
    // must equal a freshly built one byte for byte: the entry mapping, the
    // `keys_for_source` answers, the collision set and `max_key`.
    for seed in 0..128u64 {
        let mut rng = Rng(seed.wrapping_mul(0x100_0000_01B3).wrapping_add(0x9E37));
        let mut resident = Resident {
            batches: Vec::new(),
            images: Vec::new(),
            next_key: 1,
        };
        let mut table = PickTable::default();
        for step in 0..40 {
            match rng.below(8) {
                // Append batches (a base/overlay commit that keeps everything).
                0..=2 => {
                    let n = 1 + rng.below(6) as usize;
                    let added: Vec<_> = (0..n)
                        .map(|_| {
                            let key = resident.alloc();
                            random_entry(&mut rng, key)
                        })
                        .collect();
                    let incremental = resident.images.is_empty()
                        && covers(&table, &resident, resident.batches.len());
                    if incremental {
                        table
                            .replace_suffix(resident.batches.len() as u32, &[], added.clone())
                            .expect("unique keys");
                    }
                    resident.batches.extend(added);
                    if !incremental {
                        table = PickTable::build(resident.combined(), false)
                            .expect("resident keys are unique");
                    }
                }
                // Replace a suffix (drop the batches after `keep`, append new).
                3 | 4 => {
                    let keep = rng.below(resident.batches.len() as u64 + 1) as usize;
                    let retired: Vec<u32> = resident.batches[keep..]
                        .iter()
                        .map(|(key, _)| *key)
                        .collect();
                    let m = rng.below(4) as usize;
                    let added: Vec<_> = (0..m)
                        .map(|_| {
                            let key = resident.alloc();
                            random_entry(&mut rng, key)
                        })
                        .collect();
                    let incremental = resident.images.is_empty() && covers(&table, &resident, keep);
                    if incremental {
                        table
                            .replace_suffix(keep as u32, &retired, added.clone())
                            .expect("unique keys");
                    }
                    resident.batches.truncate(keep);
                    resident.batches.extend(added);
                    if !incremental {
                        table = PickTable::build(resident.combined(), false)
                            .expect("resident keys are unique");
                    }
                }
                // Re-upload the images (a wholesale replacement: full rebuild).
                5 => {
                    let n = rng.below(4) as usize;
                    resident.images = (0..n)
                        .map(|_| {
                            let key = resident.alloc();
                            random_entry(&mut rng, key)
                        })
                        .collect();
                    if resident.batches.is_empty() && resident.images.is_empty() {
                        table = PickTable::default();
                    } else {
                        table = PickTable::build(resident.combined(), false)
                            .expect("resident keys are unique");
                    }
                }
                // A full clear: the table resets with the resident set.
                6 => {
                    resident.batches.clear();
                    resident.images.clear();
                    table = PickTable::default();
                }
                // Simulate an entity-flag request: the index must be built and
                // thereafter kept exact by the incremental updates.
                _ => table.ensure_source_index(),
            }
            let mut expected = fresh(&resident);
            let mut actual = table.clone();
            actual.ensure_source_index();
            expected.ensure_source_index();
            assert_eq!(
                actual, expected,
                "seed {seed} step {step}: incremental table diverged from a fresh build"
            );
            assert_eq!(
                actual.max_key(),
                expected.max_key(),
                "seed {seed} step {step}: max_key diverged"
            );
        }
    }
}

#[test]
fn an_incremental_duplicate_key_is_rejected_exactly_like_a_build() {
    let mk = |key: u32| (key, vec![source(key as u128, &[])]);
    // A resident set with keys 1..=3, replaced down to a prefix of 2, then a
    // new suffix that reuses the still-resident key 2.
    let mut table = PickTable::build(vec![mk(1), mk(2), mk(3)], true).expect("unique keys");
    let duplicate = table
        .replace_suffix(2, &[3], vec![mk(2)])
        .expect_err("a duplicate key must not update the table");
    assert_eq!(duplicate.key, 2);
    // The rejected update left the table untouched (never a partial result).
    assert_eq!(table.max_key(), 3);
    assert_eq!(table.batch_index_of_key(1), Some(0));
    assert_eq!(table.batch_index_of_key(2), Some(1));
    assert_eq!(table.batch_index_of_key(3), Some(2));
    // A fresh build over the same (invalid) resident set refuses identically.
    let built = PickTable::build(vec![mk(1), mk(2), mk(2)], true)
        .expect_err("a duplicate key must not build a table");
    assert_eq!(built.key, 2);
    assert_eq!(built.first, 1);
    assert_eq!(built.second, 2);
    // A duplicate *within* the appended set is rejected too.
    let within = table
        .replace_suffix(3, &[], vec![mk(5), mk(5)])
        .expect_err("a duplicate key must not update the table");
    assert_eq!(within.key, 5);
}
