use super::*;

/// Deterministic xorshift PRNG so the property tests need no dev-dependency.
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

fn instance_run(stream: u32, count: u32) -> InstanceRun {
    InstanceRun {
        stream,
        offset: 0,
        count,
    }
}

fn arc_run(stream: u32, count: u32) -> ArcRun {
    ArcRun {
        stream,
        offset: 0,
        count,
    }
}

fn mesh_run(chunk: u32, index_count: u32, vertex_count: u32) -> MeshRun {
    MeshRun {
        chunk,
        index_offset: 0,
        index_count,
        vertex_count,
    }
}

/// A resident batch with no device buffers: the budget planners only read the
/// run lists, the counts and the cached upload bytes, so `None` buffers are
/// enough to exercise them without a GPU.
#[allow(clippy::too_many_arguments)]
fn gpu_batch(
    topology: RenderTopology,
    vertex_count: u32,
    index_count: u32,
    line_quad_count: u32,
    instanced: Vec<InstanceRun>,
    mesh_runs: Vec<MeshRun>,
    arc_runs: Vec<ArcRun>,
    upload_bytes: usize,
) -> GpuBatch {
    GpuBatch {
        vertices: None,
        vertex_count,
        normals: None,
        colors: None,
        topology,
        indices: None,
        index_count,
        line_quads: None,
        line_quad_count,
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
    }
}

/// A random resident batch mixing runs, legacy geometry and line quads.
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
                instanced.push(instance_run(rng.below(streams) as u32, rng.below(8) as u32));
            }
        }
        RenderTopology::Mesh => {
            for _ in 0..rng.below(3) {
                mesh_runs.push(mesh_run(
                    rng.below(chunks) as u32,
                    rng.below(15) as u32,
                    rng.below(9) as u32,
                ));
            }
        }
        RenderTopology::Conics => {
            for _ in 0..rng.below(3) {
                arc_runs.push(arc_run(rng.below(streams) as u32, rng.below(8) as u32));
            }
        }
    }
    // Legacy per-batch quads can coexist with runs (a converted batch never
    // has them, but the planner must not assume that).
    let line_quad_count = if rng.below(2) == 0 {
        rng.below(6) as u32
    } else {
        0
    };
    gpu_batch(
        topology,
        rng.below(20) as u32,
        rng.below(30) as u32,
        line_quad_count,
        instanced,
        mesh_runs,
        arc_runs,
        rng.below(200),
    )
}

/// The unified `usage.bytes` the production planner must return, derived from
/// the pre-amendment reference: identical to it except that a
/// vertices/triangles failure keeps the failing batch's byte charge (which had
/// already committed).
fn expected_unified_bytes(
    reference: &GpuPlan,
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
) -> usize {
    match reference.over_budget.as_ref() {
        Some(r) if r.report.category != "bytes" => {
            let fail_pos = cursor + reference.accepted.len();
            reference.usage.bytes + batches[ordered[fail_pos]].upload_size_bytes()
        }
        _ => reference.usage.bytes,
    }
}

fn assert_same_plan(
    case: usize,
    got: &GpuPlan,
    reference: &GpuPlan,
    batches: &[GpuBatch],
    ordered: &[usize],
    cursor: usize,
) {
    assert_eq!(
        got.accepted, reference.accepted,
        "case {case}: accepted prefix"
    );
    assert_eq!(
        got.usage.vertices, reference.usage.vertices,
        "case {case}: usage vertices"
    );
    assert_eq!(
        got.usage.triangles, reference.usage.triangles,
        "case {case}: usage triangles"
    );
    assert_eq!(
        got.usage.bytes,
        expected_unified_bytes(reference, batches, ordered, cursor),
        "case {case}: unified usage bytes"
    );
    match (&got.over_budget, &reference.over_budget) {
        (None, None) => {}
        (Some(a), Some(b)) => assert_eq!(a.report, b.report, "case {case}: over_budget report"),
        (a, b) => panic!("case {case}: over_budget presence differs: {a:?} vs {b:?}"),
    }
}

/// The differential/property test: the segmented planner must return exactly
/// what the linear reference returns over randomized batches, cull masks,
/// orders, cursors, initial reservations and budgets (finite `max_bytes` and
/// saturating corners included).
#[test]
fn segmented_page_planner_matches_the_linear_reference() {
    let mut rng = Rng::new(0xC0FFEE);
    for case in 0..4000 {
        let streams = 1 + rng.below(4);
        let chunks = 1 + rng.below(4);
        let n = 1 + rng.below(40);
        let batches: Vec<GpuBatch> = (0..n)
            .map(|_| random_batch(&mut rng, streams, chunks))
            .collect();

        // A random permutation of the batch indices; sometimes drop entries to
        // model invisible (non-drawn) batches.
        let mut ordered: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = rng.below(i + 1);
            ordered.swap(i, j);
        }
        if rng.below(4) == 0 && !ordered.is_empty() {
            let drop = rng.below(ordered.len());
            ordered.remove(drop);
        }
        let cursor = rng.below(ordered.len() + 1);

        // Arbitrary cull mask, forced to cull at least one chunk so the
        // segmented path (not the fallback) is exercised.
        let mut instance: Vec<bool> = (0..streams).map(|_| rng.below(4) != 0).collect();
        let mesh: Vec<bool> = (0..chunks).map(|_| rng.below(4) != 0).collect();
        let arc: Vec<bool> = (0..streams).map(|_| rng.below(4) != 0).collect();
        instance[0] = false;
        let cull = ChunkCull::from_visibility(instance, mesh, arc);
        assert!(cull.any_culled());

        let max_bytes = match rng.below(4) {
            0 => usize::MAX,
            1 => 0,
            2 => rng.below(400),
            _ => rng.below(4000),
        };
        let max_vertices = match rng.below(4) {
            0 => usize::MAX,
            1 => 0,
            2 => rng.below(60),
            _ => rng.below(600),
        };
        let max_triangles = match rng.below(4) {
            0 => usize::MAX,
            1 => 0,
            2 => rng.below(40),
            _ => rng.below(400),
        };
        let budget = FrameBudget {
            max_vertices,
            max_triangles,
            max_bytes,
        };
        // Initial reservation, sometimes already over the budget (the image
        // reservation case).
        let initial = cad_scene::FrameUsage {
            vertices: rng.below(30),
            triangles: rng.below(30),
            bytes: rng.below(120),
        };

        let facts = BatchFacts::build(&batches);
        let index = OrderedPageIndex::build(&batches, &ordered);
        let reference =
            plan_ordered_page_reference(&batches, &ordered, cursor, &budget, initial, &cull);
        let got = plan_ordered_page(
            &batches, &ordered, cursor, &budget, initial, &cull, &facts, &index,
        );
        assert_same_plan(case, &got, &reference, &batches, &ordered, cursor);
    }
}

/// The fallback path (nothing culled / a large active set) must still match
/// the reference exactly.
#[test]
fn fallback_path_matches_the_reference() {
    let mut rng = Rng::new(0x5EED);
    for case in 0..200 {
        let n = 1 + rng.below(20);
        let batches: Vec<GpuBatch> = (0..n).map(|_| random_batch(&mut rng, 2, 2)).collect();
        let ordered: Vec<usize> = (0..n).collect();
        let cursor = rng.below(n + 1);
        // The identity mask culls nothing, so `plan_ordered_page` takes the
        // linear fallback.
        let cull = ChunkCull::all_visible();
        let budget = FrameBudget {
            max_vertices: rng.below(200),
            max_triangles: rng.below(200),
            max_bytes: rng.below(2000),
        };
        let initial = cad_scene::FrameUsage::default();
        let facts = BatchFacts::build(&batches);
        let index = OrderedPageIndex::build(&batches, &ordered);
        let reference =
            plan_ordered_page_reference(&batches, &ordered, cursor, &budget, initial, &cull);
        let got = plan_ordered_page(
            &batches, &ordered, cursor, &budget, initial, &cull, &facts, &index,
        );
        assert_same_plan(case, &got, &reference, &batches, &ordered, cursor);
    }
}

/// A fully-culled batch is accepted with unchanged usage; a batch with legacy
/// line quads whose runs are all culled is **active** (the gap predicate is
/// `always_active`, not `batch_has_visible_work`).
#[test]
fn fully_culled_batches_are_accepted_and_line_quad_batches_stay_active() {
    let batches = vec![
        // Fully culled instanced batch: charges zero vertices/triangles.
        gpu_batch(
            RenderTopology::Lines,
            0,
            0,
            0,
            vec![instance_run(0, 4)],
            Vec::new(),
            Vec::new(),
            8,
        ),
        // Same culled run, but legacy quads keep it charged unconditionally.
        gpu_batch(
            RenderTopology::Lines,
            0,
            0,
            1,
            vec![instance_run(0, 4)],
            Vec::new(),
            Vec::new(),
            8,
        ),
    ];
    let cull = ChunkCull::from_visibility(vec![false], Vec::new(), Vec::new());
    let budget = FrameBudget {
        max_vertices: 0,
        max_triangles: 0,
        max_bytes: usize::MAX,
    };
    let initial = cad_scene::FrameUsage::default();
    let facts = BatchFacts::build(&batches);
    let index = OrderedPageIndex::build(&batches, &[0, 1]);

    // The culled run makes batch 1 look invisible to `batch_has_visible_work`
    // only when it has no quads; here its quads keep it active.
    assert!(facts.always_active.contains(&1));
    assert!(!facts.always_active.contains(&0));

    let plan = plan_ordered_page(
        &batches,
        &[0, 1],
        0,
        &budget,
        initial,
        &cull,
        &facts,
        &index,
    );
    assert_eq!(
        plan.accepted,
        vec![0],
        "the culled zero-charge batch is free"
    );
    assert_eq!(plan.usage.vertices, 0);
    assert_eq!(plan.usage.triangles, 0);
    let report = plan.over_budget.as_ref().expect("over budget").report;
    assert_eq!(report.category, "vertices");
    assert_eq!(report.requested, 6, "line quads charge 6 vertices");
    assert_eq!(report.skipped_batches, 1);
}

/// The cursor advances monotonically to completion, counting culled batches,
/// so `frame_pending` (cursor < len) eventually turns false.
#[test]
fn progressive_cursor_advances_monotonically_to_completion() {
    let mut rng = Rng::new(0xA11CE);
    let n = 30;
    let batches: Vec<GpuBatch> = (0..n).map(|_| random_batch(&mut rng, 3, 3)).collect();
    let ordered: Vec<usize> = (0..n).collect();
    // A budget every batch fits individually, so each page makes progress.
    let budget = FrameBudget {
        max_vertices: 64,
        max_triangles: 64,
        max_bytes: usize::MAX,
    };
    let initial = cad_scene::FrameUsage::default();
    let cull = ChunkCull::from_visibility(vec![false], vec![false], vec![false]);
    let facts = BatchFacts::build(&batches);
    let index = OrderedPageIndex::build(&batches, &ordered);

    let mut cursor = 0;
    let mut pages = 0;
    while cursor < ordered.len() {
        let plan = plan_ordered_page(
            &batches, &ordered, cursor, &budget, initial, &cull, &facts, &index,
        );
        assert!(!plan.accepted.is_empty(), "every page makes progress");
        let next = cursor + plan.accepted.len();
        assert!(next > cursor, "cursor strictly increases");
        cursor = next;
        pages += 1;
        assert!(pages <= ordered.len(), "pages are bounded by the scene");
    }
    assert_eq!(cursor, ordered.len());
    assert!(
        !(cursor < ordered.len()),
        "frame_pending is false once the cursor reaches the end"
    );
}

/// A batch that crosses bytes and vertices at the same position reports the
/// `"bytes"` category: `charge_bytes` runs first and short-circuits.
#[test]
fn byte_category_wins_when_bytes_and_vertices_cross_together() {
    let batches = vec![gpu_batch(
        RenderTopology::Mesh,
        4,
        6,
        0,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        100,
    )];
    // Force the segmented path: one culled chunk.
    let cull = ChunkCull::from_visibility(vec![false], Vec::new(), Vec::new());
    let budget = FrameBudget {
        max_vertices: 0,
        max_triangles: usize::MAX,
        max_bytes: 50,
    };
    let initial = cad_scene::FrameUsage::default();
    let facts = BatchFacts::build(&batches);
    let index = OrderedPageIndex::build(&batches, &[0]);
    let plan = plan_ordered_page(&batches, &[0], 0, &budget, initial, &cull, &facts, &index);
    assert!(plan.accepted.is_empty());
    let report = plan.over_budget.as_ref().expect("over budget").report;
    assert_eq!(report.category, "bytes");
    assert_eq!(report.requested, 100);
    assert_eq!(report.limit, 50);
    assert_eq!(report.skipped_batches, 1);
}

/// The two planners share the unified usage contract: a vertices/triangles
/// failure keeps the failing batch's bytes, a bytes failure keeps none of it.
/// The accepted prefix and report stay identical.
#[test]
fn both_planners_share_the_failing_batch_bytes_usage_contract() {
    let quad = |bytes| {
        gpu_batch(
            RenderTopology::Lines,
            0,
            0,
            1,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            bytes,
        )
    };
    let cull = ChunkCull::from_visibility(vec![false], Vec::new(), Vec::new());
    let initial = cad_scene::FrameUsage::default();

    // (a) vertices failure after the byte charge succeeded: both keep the
    // failing batch's 40 bytes.
    let batches = vec![quad(40), quad(40)];
    let budget = FrameBudget {
        max_vertices: 8,
        max_triangles: usize::MAX,
        max_bytes: usize::MAX,
    };
    let facts = BatchFacts::build(&batches);
    let ordered: Vec<usize> = (0..batches.len()).collect();
    let index = OrderedPageIndex::build(&batches, &ordered);
    let from_gpu = plan_from_gpu(&batches, &budget, initial, &cull);
    let page = plan_ordered_page(
        &batches, &ordered, 0, &budget, initial, &cull, &facts, &index,
    );
    assert_eq!(from_gpu.accepted, vec![0]);
    assert_eq!(page.accepted, vec![0]);
    assert_eq!(from_gpu.usage, page.usage);
    assert_eq!(page.usage.bytes, 80, "the failing batch's bytes are kept");
    assert_eq!(page.usage.vertices, 6);
    assert_eq!(
        from_gpu.over_budget.as_ref().map(|r| r.report),
        page.over_budget.as_ref().map(|r| r.report)
    );

    // (b) bytes failure: neither keeps the failing batch's bytes.
    let batches = vec![quad(40), quad(100)];
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 60,
    };
    let facts = BatchFacts::build(&batches);
    let index = OrderedPageIndex::build(&batches, &ordered);
    let from_gpu = plan_from_gpu(&batches, &budget, initial, &cull);
    let page = plan_ordered_page(
        &batches, &ordered, 0, &budget, initial, &cull, &facts, &index,
    );
    assert_eq!(from_gpu.accepted, vec![0]);
    assert_eq!(page.accepted, vec![0]);
    assert_eq!(from_gpu.usage, page.usage);
    assert_eq!(page.usage.bytes, 40, "a byte failure keeps none of it");
}

/// The pacing signal (`accepted_with_work` per page) is identical between the
/// planners, so the `pacing::step` trajectory is identical.
#[test]
fn accepted_with_work_sequence_and_pacing_trajectory_are_identical() {
    let mut rng = Rng::new(0xBEEF);
    for case in 0..300 {
        let n = 1 + rng.below(30);
        let batches: Vec<GpuBatch> = (0..n).map(|_| random_batch(&mut rng, 3, 3)).collect();
        let mut ordered: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = rng.below(i + 1);
            ordered.swap(i, j);
        }
        let cull =
            ChunkCull::from_visibility(vec![false, rng.below(2) == 0], vec![false], vec![false]);
        let budget = FrameBudget {
            max_vertices: 200,
            max_triangles: 200,
            max_bytes: usize::MAX,
        };
        let initial = cad_scene::FrameUsage::default();
        let facts = BatchFacts::build(&batches);
        let index = OrderedPageIndex::build(&batches, &ordered);

        let mut reference_seq = Vec::new();
        let mut segmented_seq = Vec::new();
        let mut cursor = 0;
        while cursor < ordered.len() {
            let reference =
                plan_ordered_page_reference(&batches, &ordered, cursor, &budget, initial, &cull);
            let got = plan_ordered_page(
                &batches, &ordered, cursor, &budget, initial, &cull, &facts, &index,
            );
            assert_eq!(reference.accepted, got.accepted, "case {case}");
            let work = |plan: &GpuPlan| {
                plan.accepted
                    .iter()
                    .filter(|&&i| batch_has_visible_work(&batches[i], &cull))
                    .count()
            };
            reference_seq.push(work(&reference));
            segmented_seq.push(work(&got));
            cursor += reference.accepted.len().max(1);
        }
        assert_eq!(reference_seq, segmented_seq, "case {case}: pacing signal");

        let mut reference_state = pacing::PacingState::new(13.0);
        let mut segmented_state = pacing::PacingState::new(13.0);
        for (&r, &s) in reference_seq.iter().zip(segmented_seq.iter()) {
            reference_state = pacing::step(
                reference_state,
                pacing::Sample {
                    cpu_ms: 5.0,
                    wait_ms: 1.0,
                    accepted: r,
                    restart: false,
                },
            )
            .0;
            segmented_state = pacing::step(
                segmented_state,
                pacing::Sample {
                    cpu_ms: 5.0,
                    wait_ms: 1.0,
                    accepted: s,
                    restart: false,
                },
            )
            .0;
        }
        assert_eq!(reference_state.scale, segmented_state.scale, "case {case}");
        assert_eq!(
            reference_state.cost_ema_ms, segmented_state.cost_ema_ms,
            "case {case}"
        );
    }
}

/// The encode loop's per-batch effect under `cull`, with no skipping: a
/// run-free batch takes the legacy branch (which flushes the pending coalesced
/// runs, and usually draws its legacy buffers), and a batch with runs issues a
/// draw for each run that survives the cull. A batch whose runs are all culled
/// and which owns runs is a pure no-op.
fn encode_reference_emits(batch: &GpuBatch, cull: &ChunkCull) -> bool {
    let has_mesh = !batch.mesh_runs.is_empty();
    let has_line = !batch.instanced.is_empty();
    let has_arc = !batch.arc_runs.is_empty();
    if !has_mesh && !has_line && !has_arc {
        return true;
    }
    batch
        .mesh_runs
        .iter()
        .any(|run| cull.mesh_visible(run.chunk as usize))
        || batch
            .instanced
            .iter()
            .any(|run| cull.stream_visible(run.stream as usize))
        || batch
            .arc_runs
            .iter()
            .any(|run| cull.arc_visible(run.stream as usize))
}

/// The encode-skip predicate is a **superset-safe** filter: an entry is skipped
/// only when today's encode loop would do nothing for it (no draw, no
/// pending-run flush). Compares the mask against a reference walk that never
/// skips and asserts the emitted-entry set and the reporting counters are
/// identical.
#[test]
fn encode_skip_is_superset_safe_and_preserves_counters() {
    let mut rng = Rng::new(0xE0C0DE);
    for case in 0..4000 {
        let streams = 1 + rng.below(4);
        let chunks = 1 + rng.below(4);
        let n = 1 + rng.below(40);
        let batches: Vec<GpuBatch> = (0..n)
            .map(|_| random_batch(&mut rng, streams, chunks))
            .collect();
        // A cull mask that removes at least one chunk; otherwise the mask is
        // the trivial `all` and nothing is ever skipped.
        let mut instance: Vec<bool> = (0..streams).map(|_| rng.below(4) != 0).collect();
        let mesh: Vec<bool> = (0..chunks).map(|_| rng.below(4) != 0).collect();
        let arc: Vec<bool> = (0..streams).map(|_| rng.below(4) != 0).collect();
        instance[0] = false;
        let cull = ChunkCull::from_visibility(instance, mesh, arc);
        assert!(cull.any_culled());

        let facts = BatchFacts::build(&batches);
        let mut mask = EncodeEmitMask::default();
        mask.rebuild(&facts, &cull);

        let mut emitted_reference = Vec::new();
        let mut emitted_skipped = Vec::new();
        let mut counters_reference = [0usize; 4];
        let mut counters_skipped = [0usize; 4];
        for (i, batch) in batches.iter().enumerate() {
            let emits = encode_reference_emits(batch, &cull);
            if emits {
                emitted_reference.push(i);
            }
            let participates = mask.participates(i);
            if !participates {
                assert!(
                    !emits,
                    "case {case}: batch {i} skipped but the reference emits"
                );
            }
            if participates && emits {
                emitted_skipped.push(i);
            }
            // The reporting counters count accepted converted batches, not
            // emitted draws; the class byte must reproduce the batch-derived
            // counts exactly.
            let class = facts.class[i];
            if class != 0 {
                counters_skipped[0] += 1;
                if class & CLASS_MESH != 0 {
                    counters_skipped[1] += 1;
                }
                if class & CLASS_INSTANCED != 0 {
                    counters_skipped[2] += 1;
                }
                if class & CLASS_ARC != 0 {
                    counters_skipped[3] += 1;
                }
            }
            if !batch.instanced.is_empty()
                || !batch.mesh_runs.is_empty()
                || !batch.arc_runs.is_empty()
            {
                counters_reference[0] += 1;
                if !batch.mesh_runs.is_empty() {
                    counters_reference[1] += 1;
                }
                if !batch.instanced.is_empty() {
                    counters_reference[2] += 1;
                }
                if !batch.arc_runs.is_empty() {
                    counters_reference[3] += 1;
                }
            }
        }
        assert_eq!(
            emitted_skipped, emitted_reference,
            "case {case}: emitted-entry set"
        );
        assert_eq!(
            counters_skipped, counters_reference,
            "case {case}: counters"
        );
    }
}

/// Regression for the predicate: a fully-culled batch with legacy
/// `line_quad_count > 0` is never skipped, even though it also owns runs.
#[test]
fn fully_culled_batch_with_legacy_quads_is_still_processed() {
    let batches = vec![gpu_batch(
        RenderTopology::Lines,
        0,
        0,
        1,
        vec![instance_run(0, 4)],
        Vec::new(),
        Vec::new(),
        8,
    )];
    let cull = ChunkCull::from_visibility(vec![false], Vec::new(), Vec::new());
    let facts = BatchFacts::build(&batches);
    let mut mask = EncodeEmitMask::default();
    mask.rebuild(&facts, &cull);
    assert!(facts.always_active.contains(&0));
    assert!(
        mask.participates(0),
        "a fully-culled batch with legacy line quads must not be skipped"
    );
}

/// The uniform bytes written for every entry that still gets a slot are
/// bit-identical with and without the skip: the skip only drops entries whose
/// slot is never read (a batch with runs owns no per-batch uniform, and a
/// fully-culled one draws nothing). Models the old (no-skip) slot set and the
/// new one over randomized batches.
#[test]
fn skipped_uniform_slots_leave_surviving_bytes_bit_identical() {
    let projection = FrameProjection::Ortho {
        sx: 0.03125,
        sy: 0.03125,
        center: [0.0, 0.0, 0.0],
        z_plane: 0.0,
        depth: OrthoDepth::default(),
        world_per_px: 0.0,
    };
    let line_wpp_bits = (projection.world_per_px() as f32).to_bits();
    let pack = |batch: &GpuBatch| {
        let (m, [r, g, b], alpha) = (
            projection.batch_matrix(batch.origin),
            renderer_color(batch.color),
            clamp_alpha(batch.alpha),
        );
        uniform_bytes(
            &m,
            [r, g, b, alpha],
            [batch.pick_key, batch.instance_flags(), line_wpp_bits, 0],
        )
    };

    let mut rng = Rng::new(0x5107);
    for case in 0..2000 {
        let n = 1 + rng.below(30);
        let batches: Vec<GpuBatch> = (0..n).map(|_| random_batch(&mut rng, 3, 3)).collect();
        let facts = BatchFacts::build(&batches);
        let mut reference: Vec<(usize, [u8; 96])> = Vec::new();
        let mut skipped: Vec<(usize, [u8; 96])> = Vec::new();
        for (i, batch) in batches.iter().enumerate() {
            // Old (no-skip) slot condition: no instanced and no mesh runs.
            if batch.instanced.is_empty() && batch.mesh_runs.is_empty() {
                reference.push((i, pack(batch)));
            }
            // New (skip) condition: owns no runs at all.
            if facts.class[i] == 0 {
                skipped.push((i, pack(batch)));
            }
        }
        // Every entry that still gets a slot has identical bytes.
        for (index, bytes) in &skipped {
            let reference_bytes = reference
                .iter()
                .find(|(j, _)| j == index)
                .map(|(_, b)| b)
                .expect("a run-free batch is slotted by both");
            assert_eq!(bytes, reference_bytes, "case {case}: batch {index} bytes");
        }
        // The dropped entries are exactly the arc-only batches (which never
        // read `frame_slots`).
        for (index, _) in &reference {
            if facts.class[*index] != 0 {
                assert_eq!(
                    facts.class[*index], CLASS_ARC,
                    "case {case}: only arc-only batches may be dropped"
                );
            }
        }
    }
}
