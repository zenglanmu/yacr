//! A1 contract: the renderer caches the draw-order partition and the segmented
//! planner's facts over the **base** prefix only (`[0, base_end)`) and filters
//! `< base_end`. For the only shape a true suffix commit can carry (run-free
//! overlay batches, staged legacy), base-only cache/facts + filter must produce
//! exactly the same base order and page plan as the previous all-batch cache
//! filtered to the base.

use super::*;

/// Deterministic xorshift PRNG so the property test needs no dev-dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        assert!(n > 0);
        (self.next() % n as u64) as usize
    }
}

/// A resident batch with no device buffers (the planners only read the run
/// lists, the counts and the cached upload bytes).
fn gpu_batch(
    topology: RenderTopology,
    instanced: Vec<InstanceRun>,
    mesh_runs: Vec<MeshRun>,
    arc_runs: Vec<ArcRun>,
    upload_bytes: usize,
) -> GpuBatch {
    GpuBatch {
        vertices: None,
        vertex_count: 0,
        normals: None,
        colors: None,
        topology,
        text_glyph: false,
        indices: None,
        index_count: 0,
        line_quads: None,
        line_quad_count: 0,
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        lineweight: 0.0,
        draw_order: 0,
        centroid: [0.0; 3],
        origin: [0.0; 3],
        instanced,
        mesh_runs,
        arc_runs,
        upload_bytes,
        pick_key: 0,
        z_range: [0.0, 0.0],
        sources: Vec::new(),
        vertex_charge: None,
        index_charge: None,
    }
}

fn random_batch(rng: &mut Rng, streams: usize, chunks: usize) -> GpuBatch {
    let topology = match rng.below(4) {
        0 => RenderTopology::Lines,
        1 => RenderTopology::Mesh,
        2 => RenderTopology::MeshEdges,
        _ => RenderTopology::Conics,
    };
    let mut instanced = Vec::new();
    let mut mesh_runs = Vec::new();
    let mut arc_runs = Vec::new();
    match topology {
        RenderTopology::Lines | RenderTopology::MeshEdges => {
            for _ in 0..rng.below(3) {
                instanced.push(InstanceRun {
                    stream: rng.below(streams) as u32,
                    offset: 0,
                    count: rng.below(8) as u32,
                });
            }
        }
        RenderTopology::Mesh => {
            for _ in 0..rng.below(3) {
                mesh_runs.push(MeshRun {
                    chunk: rng.below(chunks) as u32,
                    index_offset: 0,
                    index_count: rng.below(15) as u32,
                    vertex_count: rng.below(9) as u32,
                });
            }
        }
        RenderTopology::Conics => {
            for _ in 0..rng.below(3) {
                arc_runs.push(ArcRun {
                    stream: rng.below(streams) as u32,
                    offset: 0,
                    count: rng.below(8) as u32,
                });
            }
        }
    }
    gpu_batch(topology, instanced, mesh_runs, arc_runs, rng.below(200))
}

#[test]
fn base_only_cache_and_facts_match_the_filtered_all_batch_plan() {
    let mut rng = Rng::new(0xBA5E_0001);
    for case in 0..250 {
        // Deterministically include the empty-base shape (`base_len == 0`): the
        // base-only cache/facts must still match the filtered all-batch plan
        // (both empty), and the page-plan comparison is skipped because there is
        // no ordered base batch.
        let base_len = if case % 40 == 0 { 0 } else { 1 + rng.below(20) };
        let suffix_len = rng.below(6);
        let streams = 1 + rng.below(4);
        let chunks = 1 + rng.below(4);

        // Base batches are ordinary converted/legacy batches.
        let mut batches: Vec<GpuBatch> = (0..base_len)
            .map(|_| random_batch(&mut rng, streams, chunks))
            .collect();
        // The suffix is run-free (the only shape the commit's overlay-run
        // rejection admits), i.e. what `prepare_upload_batches_legacy` stages.
        for _ in 0..suffix_len {
            batches.push(gpu_batch(
                RenderTopology::Lines,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                rng.below(200),
            ));
        }
        // Give the partition real alpha/centroid/draw-order variety.
        for batch in &mut batches {
            batch.alpha = match rng.below(5) {
                0 => 0.0,
                1 => 0.5,
                2 => 0.25,
                _ => 1.0,
            };
            batch.draw_order = rng.below(9) as i64 - 4;
            batch.centroid = [
                rng.below(400) as f32 - 200.0,
                rng.below(400) as f32 - 200.0,
                rng.below(400) as f32 - 200.0,
            ];
        }

        let all_entries: Vec<BatchOrderEntry> = batches
            .iter()
            .map(|batch| BatchOrderEntry {
                draw_order: batch.draw_order,
                alpha: batch.alpha,
                centroid: batch.centroid,
            })
            .collect();
        let mut all_cache = DrawOrderCache::new(all_entries.clone());
        // The base-only cache: the new renderer build.
        let mut base_cache = DrawOrderCache::new(all_entries[..base_len].to_vec());

        for k in 0..4 {
            let eye = [
                k as f32 * 37.0 - 50.0,
                60.0 - k as f32 * 11.0,
                k as f32 * 3.0 - 5.0,
            ];
            all_cache.sort_transparent(Some(eye));
            base_cache.sort_transparent(Some(eye));

            // The old renderer filtered the all-batch cache to `< base_end`.
            let filter = |indices: &[usize]| -> Vec<usize> {
                indices
                    .iter()
                    .copied()
                    .filter(|&index| index < base_len)
                    .collect()
            };
            assert_eq!(
                base_cache.opaque(),
                filter(all_cache.opaque()).as_slice(),
                "case {case} camera {k}: opaque order"
            );
            assert_eq!(
                base_cache.transparent(),
                filter(all_cache.transparent()).as_slice(),
                "case {case} camera {k}: transparent order"
            );
            assert_eq!(
                base_cache.invisible(),
                filter(all_cache.invisible()).as_slice(),
                "case {case} camera {k}: invisible set"
            );
            // The full base plan is exactly the fresh plan over the base slice.
            assert_eq!(
                base_cache.to_plan(),
                plan_draw_order(&all_entries[..base_len], Some(eye)),
                "case {case} camera {k}: base-only cached plan"
            );
        }

        // The segmented planner's facts are base-only and agree with the
        // all-batch facts restricted to the base prefix (the run-free suffix
        // contributes no fact of its own).
        let base_facts = BatchFacts::build(&batches[..base_len]);
        let all_facts = BatchFacts::build(&batches);
        assert_eq!(base_facts.class, all_facts.class[..base_len]);
        assert_eq!(base_facts.upload_bytes, all_facts.upload_bytes[..base_len]);
        assert_eq!(
            base_facts.legacy_batches,
            all_facts
                .legacy_batches
                .iter()
                .copied()
                .filter(|&i| (i as usize) < base_len)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            base_facts.always_active,
            all_facts
                .always_active
                .iter()
                .copied()
                .filter(|&i| (i as usize) < base_len)
                .collect::<Vec<_>>()
        );
        assert_eq!(base_facts.chunk_batches, all_facts.chunk_batches);
        assert_eq!(base_facts.converted_total, all_facts.converted_total);
        assert_eq!(base_facts.has_line_class, all_facts.has_line_class);
        assert_eq!(base_facts.has_mesh_class, all_facts.has_mesh_class);

        // ...and the page plan over the base order is identical with either facts
        // set / index.
        let ordered: Vec<usize> = base_cache.to_plan().opaque;
        if ordered.is_empty() {
            continue;
        }
        let cursor = rng.below(ordered.len() + 1);
        let mut instance: Vec<bool> = (0..base_facts.chunk_batches[0].len().max(1))
            .map(|_| rng.below(4) != 0)
            .collect();
        instance[0] = false;
        let mesh: Vec<bool> = (0..base_facts.chunk_batches[1].len())
            .map(|_| rng.below(4) != 0)
            .collect();
        let arc: Vec<bool> = (0..base_facts.chunk_batches[2].len())
            .map(|_| rng.below(4) != 0)
            .collect();
        let cull = ChunkCull::from_visibility(instance, mesh, arc);
        let budget = FrameBudget {
            max_vertices: if rng.below(3) == 0 {
                usize::MAX
            } else {
                rng.below(60)
            },
            max_triangles: if rng.below(3) == 0 {
                usize::MAX
            } else {
                rng.below(40)
            },
            max_bytes: if rng.below(3) == 0 {
                usize::MAX
            } else {
                rng.below(600)
            },
        };
        let initial = cad_scene::FrameUsage {
            vertices: rng.below(10),
            triangles: rng.below(10),
            bytes: rng.below(40),
        };
        let base_index = OrderedPageIndex::build(&batches[..base_len], &ordered);
        let all_index = OrderedPageIndex::build(&batches, &ordered);
        let with_base = plan_ordered_page(
            &batches,
            &ordered,
            cursor,
            &budget,
            initial,
            &cull,
            &base_facts,
            &base_index,
        );
        let with_all = plan_ordered_page(
            &batches, &ordered, cursor, &budget, initial, &cull, &all_facts, &all_index,
        );
        assert_eq!(
            with_base.accepted, with_all.accepted,
            "case {case}: accepted prefix"
        );
        assert_eq!(with_base.usage, with_all.usage, "case {case}: usage");
        assert_eq!(
            with_base.over_budget.as_ref().map(|over| over.report),
            with_all.over_budget.as_ref().map(|over| over.report),
            "case {case}: over-budget report"
        );
    }
}

/// The encode mask is built from the base-only facts, but the overlay pass calls
/// `participates` with absolute overlay indices (`>= base_end`). A culled frame
/// (`all == false`) must still admit those out-of-table indices instead of
/// panicking or reading out of bounds; base indices follow the mask as before.
#[test]
fn encode_emit_mask_admits_absolute_overlay_indices_past_the_base_table() {
    // Two base batches: batch 0 references instance stream 0, batch 1 stream 1.
    let base = vec![
        gpu_batch(
            RenderTopology::Lines,
            vec![InstanceRun {
                stream: 0,
                offset: 0,
                count: 4,
            }],
            Vec::new(),
            Vec::new(),
            8,
        ),
        gpu_batch(
            RenderTopology::Lines,
            vec![InstanceRun {
                stream: 1,
                offset: 0,
                count: 4,
            }],
            Vec::new(),
            Vec::new(),
            8,
        ),
    ];
    let facts = BatchFacts::build(&base);
    // Cull stream 0, keep stream 1: `any_culled` is true, so the mask is built
    // (`all == false`) instead of short-circuiting.
    let cull = ChunkCull::from_visibility(vec![false, true], Vec::new(), Vec::new());
    assert!(cull.any_culled());

    let mut mask = EncodeEmitMask::default();
    mask.rebuild(&facts, &cull);

    // Base indices follow the mask: the culled batch is skipped, the visible one
    // participates.
    assert!(
        !mask.participates(0),
        "a fully-culled batch must not participate"
    );
    assert!(mask.participates(1), "a visible batch must participate");

    // The overlay indices are past the base-only table (`base.len() == 2`). They
    // are run-free legacy batches and must always participate — no panic, no
    // out-of-bounds word read, even on a culled frame.
    for overlay in 0..4 {
        assert!(
            mask.participates(base.len() + overlay),
            "overlay index {} must participate past the base-only facts",
            base.len() + overlay
        );
    }
}
