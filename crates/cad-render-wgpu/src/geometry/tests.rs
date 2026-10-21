use super::*;
use cad_domain::{EntityId, SelectionRef};
use cad_scene::RenderTopology;

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn mesh_batch(mirrored: bool) -> RenderBatch {
    RenderBatch {
        conics: Vec::new(),
        local_origin: point(0.0, 0.0, 0.0),
        topology: RenderTopology::Mesh,
        vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Vec::new(),
        colors: Vec::new(),
        indices: vec![[0, 1, 2]],
        edges: Vec::new(),
        mirrored,
        alpha: 1.0,
        color: cad_scene::DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: cad_scene::LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![SelectionRef {
            document: cad_domain::DocumentId(1),
            entity: EntityId(1),
            instance: Default::default(),
            sub_element: None,
        }],
        draw_order: 0,
    }
}

#[test]
fn winding_flip_follows_the_mirror_flag() {
    assert!(!winding_is_flipped(false));
    assert!(winding_is_flipped(true));
    assert!(front_face_ccw(false));
    assert!(!front_face_ccw(true));
}

#[test]
fn missing_normals_are_recomputed_to_plus_z() {
    let batch = mesh_batch(false);
    assert!(normals_need_repair(&batch));
    let n = repaired_normals(&batch);
    assert_eq!(n.len(), 3);
    for v in &n {
        assert!((v[2] - 1.0).abs() < 1e-6, "expected +Z, got {v:?}");
    }
}

#[test]
fn consistent_finite_normals_are_kept_and_normalized() {
    let mut batch = mesh_batch(false);
    batch.normals = vec![[0.0, 0.0, 2.0], [0.0, 0.0, 2.0], [0.0, 0.0, 2.0]];
    assert!(!normals_need_repair(&batch));
    let n = repaired_normals(&batch);
    for v in &n {
        assert!((v[2] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn non_finite_normal_triggers_repair_and_never_leaks_nan() {
    let mut batch = mesh_batch(false);
    batch.normals = vec![[f32::NAN, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]];
    assert!(normals_need_repair(&batch));
    let n = repaired_normals(&batch);
    for v in &n {
        assert!(v.iter().all(|c| c.is_finite()), "got {v:?}");
    }
}

#[test]
fn degenerate_mesh_gets_fallback_normal() {
    // A zero-area triangle: recomputation yields no direction.
    let mut batch = mesh_batch(false);
    batch.vertices = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
    let n = repaired_normals(&batch);
    for v in &n {
        assert!(v.iter().all(|c| c.is_finite()));
        assert!((length(point(v[0] as f64, v[1] as f64, v[2] as f64)) - 1.0).abs() < 1e-6);
    }
}

#[test]
fn edge_indices_are_sorted_and_deduplicated() {
    // Two triangles sharing edge (1,2).
    let edges = sorted_edge_indices(&[[0, 1, 2], [2, 1, 3]]);
    // Unique edges: (0,1),(1,2),(0,2),(1,3),(2,3) => 10 indices.
    assert_eq!(edges.len(), 10);
    assert_eq!(edges, vec![0, 1, 0, 2, 1, 2, 1, 3, 2, 3]);
    // Deterministic for the same input.
    assert_eq!(sorted_edge_indices(&[[0, 1, 2], [2, 1, 3]]), edges);
}

#[test]
fn mesh_upload_edge_indices_follow_the_optional_overlay_contract() {
    let mut batch = mesh_batch(false);
    assert!(mesh_edge_indices(&batch).is_empty());
    batch.edges = vec![[0.0; 3], [1.0, 0.0, 0.0]];
    assert_eq!(mesh_edge_indices(&batch), vec![0, 1, 0, 2, 1, 2]);
}

#[test]
fn plan_frame_accepts_within_budget() {
    let batches = vec![mesh_batch(false)];
    let budget = FrameBudget {
        max_vertices: 100,
        max_triangles: 100,
        max_bytes: usize::MAX,
    };
    let (accepted, usage, over) = plan_frame(&batches, &budget);
    assert_eq!(accepted.len(), 1);
    assert_eq!(usage.vertices, 3);
    assert_eq!(usage.triangles, 1);
    assert!(over.is_none());
}

#[test]
fn plan_frame_reports_over_budget_and_skipped_batches() {
    let batches = vec![mesh_batch(false), mesh_batch(false), mesh_batch(false)];
    let budget = FrameBudget {
        max_vertices: 4,
        max_triangles: 100,
        max_bytes: usize::MAX,
    };
    let (accepted, usage, over) = plan_frame(&batches, &budget);
    assert_eq!(accepted.len(), 1);
    assert_eq!(usage.vertices, 3);
    let over = over.expect("expected an over-budget report");
    assert_eq!(over.category, "vertices");
    assert_eq!(over.limit, 4);
    assert_eq!(over.skipped_batches, 2);
}

#[test]
fn plan_frame_reports_over_byte_budget() {
    // One mesh batch without an edge overlay is 3 positions + 3 normals +
    // 3 colours + 1 triangle index = 120 bytes. A budget of 100 rejects it.
    let batches = vec![mesh_batch(false)];
    assert_eq!(batches[0].upload_size_bytes(), 120);
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 100,
    };
    let (accepted, usage, over) = plan_frame(&batches, &budget);
    assert!(accepted.is_empty());
    assert_eq!(usage.bytes, 0);
    let over = over.expect("expected an over-budget report");
    assert_eq!(over.category, "bytes");
    assert_eq!(over.requested, 120);
    assert_eq!(over.limit, 100);
}

#[test]
fn plan_frame_charges_exact_upload_bytes_on_success() {
    let batches = vec![mesh_batch(false)];
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: usize::MAX,
    };
    let (accepted, usage, over) = plan_frame(&batches, &budget);
    assert_eq!(accepted.len(), 1);
    assert!(over.is_none());
    assert_eq!(usage.bytes, batches[0].upload_size_bytes());
}

#[test]
fn plan_view_projects_nearer_geometry_to_smaller_depth() {
    let camera = Camera3d::plan_view(point(0.0, 0.0, 10.0), point(0.0, 0.0, 0.0));
    let vp = camera.view_projection(1.0).expect("usable camera");
    let near = apply(vp, point(0.0, 0.0, 0.0));
    let far = apply(vp, point(0.0, 0.0, -10.0));
    assert!(near[2] < far[2], "near {near:?} should be < far {far:?}");
    // Clip w is the distance along the view axis (10 and 20 here).
    assert!((near[3] - 10.0).abs() < 1e-4, "w {near:?}");
    assert!((far[3] - 20.0).abs() < 1e-4, "w {far:?}");
    // Normalized device depth of the near point is inside [0, 1].
    let ndc_near = near[2] / near[3];
    assert!((0.0..=1.0).contains(&ndc_near), "ndc {ndc_near}");
}

#[test]
fn degenerate_camera_has_no_projection() {
    let camera = Camera3d::plan_view(point(0.0, 0.0, 0.0), point(0.0, 0.0, 0.0));
    assert!(!camera.is_usable(1.0));
    assert!(camera.view_projection(1.0).is_none());
    let bad_up = Camera3d {
        eye: point(0.0, 0.0, 10.0),
        target: point(0.0, 0.0, 0.0),
        up: point(0.0, 0.0, 1.0), // parallel to the view direction
        ..Camera3d::plan_view(point(0.0, 0.0, 10.0), point(0.0, 0.0, 0.0))
    };
    assert!(bad_up.view_projection(1.0).is_none());
}

fn apply(m: [[f32; 4]; 4], p: Point3) -> [f32; 4] {
    let v = [p.x as f32, p.y as f32, p.z as f32, 1.0];
    let mut out = [0.0f32; 4];
    for (r, cell) in out.iter_mut().enumerate() {
        *cell = m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2] + m[3][r] * v[3];
    }
    out
}

// --- Draw order + transparency policy (F14) ---

fn entry(draw_order: i64, alpha: f32, x: f32) -> BatchOrderEntry {
    BatchOrderEntry {
        draw_order,
        alpha,
        centroid: [x, 0.0, 0.0],
    }
}

#[test]
fn alpha_is_clamped_and_nan_is_opaque() {
    assert_eq!(clamp_alpha(0.5), 0.5);
    assert_eq!(clamp_alpha(-1.0), 0.0);
    assert_eq!(clamp_alpha(2.0), 1.0);
    // An unreadable opacity must not delete geometry: NaN is opaque.
    assert_eq!(clamp_alpha(f32::NAN), 1.0);
    assert_eq!(clamp_alpha(f32::INFINITY), 1.0);
    assert_eq!(clamp_alpha(f32::NEG_INFINITY), 0.0);
}

#[test]
fn alpha_classification_covers_the_three_buckets() {
    assert_eq!(classify_alpha(1.0), AlphaClass::Opaque);
    assert_eq!(classify_alpha(2.0), AlphaClass::Opaque);
    assert_eq!(classify_alpha(f32::NAN), AlphaClass::Opaque);
    assert_eq!(classify_alpha(0.25), AlphaClass::Transparent);
    assert_eq!(classify_alpha(0.0), AlphaClass::Invisible);
    assert_eq!(classify_alpha(-0.5), AlphaClass::Invisible);
}

#[test]
fn opaque_batches_sort_by_draw_order_and_keep_upload_order_on_ties() {
    // Draw orders: [5, 1, 1, 3]; the two `1`s must keep upload order (1 before 2).
    let entries = [
        entry(5, 1.0, 0.0),
        entry(1, 1.0, 0.0),
        entry(1, 1.0, 0.0),
        entry(3, 1.0, 0.0),
    ];
    let plan = plan_draw_order(&entries, None);
    assert_eq!(plan.opaque, vec![1, 2, 3, 0]);
    assert!(plan.transparent.is_empty());
    assert!(plan.invisible.is_empty());
    assert_eq!(plan.drawn(), 4);
}

#[test]
fn transparent_batches_are_back_to_front_by_camera_distance() {
    // Camera at x = 0; nearer batch at x = 1, farther at x = 10.
    let entries = [entry(0, 0.5, 1.0), entry(0, 0.5, 10.0), entry(0, 0.5, 5.0)];
    let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
    // Farthest first: index 1 (x=10), then 2 (x=5), then 0 (x=1).
    assert_eq!(plan.transparent, vec![1, 2, 0]);
}

#[test]
fn transparent_distance_ties_fall_back_to_draw_order_then_index() {
    // Same distance; draw_order decides, then upload index.
    let entries = [entry(9, 0.5, 3.0), entry(1, 0.5, 3.0), entry(1, 0.5, 3.0)];
    let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
    assert_eq!(plan.transparent, vec![1, 2, 0]);
}

#[test]
fn without_a_camera_transparent_batches_use_draw_order_only() {
    let entries = [entry(4, 0.5, 1.0), entry(2, 0.5, 99.0)];
    let plan = plan_draw_order(&entries, None);
    // Distance is ignored; ascending draw order, stable.
    assert_eq!(plan.transparent, vec![1, 0]);
}

#[test]
fn zero_alpha_batches_are_partitioned_out_not_drawn() {
    let entries = [entry(0, 1.0, 0.0), entry(0, 0.0, 0.0), entry(0, 0.4, 0.0)];
    let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
    assert_eq!(plan.opaque, vec![0]);
    assert_eq!(plan.transparent, vec![2]);
    assert_eq!(plan.invisible, vec![1]);
    assert_eq!(plan.drawn(), 2);
}

#[test]
fn opaque_always_precedes_transparent_regardless_of_draw_order() {
    // A transparent batch with a huge draw_order must still follow opaque.
    let entries = [entry(1_000_000, 0.5, 0.0), entry(0, 1.0, 0.0)];
    let plan = plan_draw_order(&entries, Some([0.0, 0.0, 0.0]));
    assert_eq!(plan.opaque, vec![1]);
    assert_eq!(plan.transparent, vec![0]);
}

#[test]
fn plan_is_deterministic_for_the_same_input() {
    let entries = [
        entry(5, 0.5, 2.0),
        entry(1, 1.0, 0.0),
        entry(0, 0.0, 0.0),
        entry(5, 0.5, 8.0),
    ];
    let a = plan_draw_order(&entries, Some([1.0, 2.0, 3.0]));
    let b = plan_draw_order(&entries, Some([1.0, 2.0, 3.0]));
    assert_eq!(a, b);
    // Sanity: the partition covers every input exactly once.
    let mut all: Vec<usize> = a
        .opaque
        .iter()
        .chain(a.transparent.iter())
        .chain(a.invisible.iter())
        .copied()
        .collect();
    all.sort_unstable();
    assert_eq!(all, vec![0, 1, 2, 3]);
}

#[test]
fn moving_camera_changes_the_transparent_order() {
    let entries = [entry(0, 0.5, -10.0), entry(0, 0.5, 10.0)];
    let from_left = plan_draw_order(&entries, Some([-100.0, 0.0, 0.0]));
    let from_right = plan_draw_order(&entries, Some([100.0, 0.0, 0.0]));
    // From the left, x=10 is farther; from the right, x=-10 is farther.
    assert_eq!(from_left.transparent, vec![1, 0]);
    assert_eq!(from_right.transparent, vec![0, 1]);
}

#[test]
fn cached_partition_matches_fresh_plan_across_cameras() {
    use crate::plan::draw_passes;

    // Deterministic LCG so the fuzz is reproducible without a `rand`
    // dependency; the point is many random-ish alpha/draw_order/centroid
    // combinations, including distance ties.
    let mut state = 0x1234_5678u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state
    };
    let n = 500usize;
    let entries: Vec<BatchOrderEntry> = (0..n)
        .map(|_| {
            let alpha = match next() % 5 {
                0 => 0.0,
                1 => 0.5,
                2 => 0.25,
                _ => 1.0,
            };
            BatchOrderEntry {
                draw_order: (next() % 9) as i64 - 4,
                alpha,
                centroid: [
                    (next() % 400) as f32 - 200.0,
                    (next() % 400) as f32 - 200.0,
                    (next() % 400) as f32 - 200.0,
                ],
            }
        })
        .collect();

    let mut cache = DrawOrderCache::new(entries.clone());
    let accepted: Vec<usize> = (0..n).collect();

    for k in 0..25 {
        let eye = [
            k as f32 * 13.0 - 150.0,
            150.0 - k as f32 * 9.0,
            k as f32 * 1.5 - 20.0,
        ];
        // The freshly computed plan the renderer used before the cache.
        let fresh = draw_passes(&accepted, &entries, Some(eye));
        cache.sort_transparent(Some(eye));
        let plan = cache.to_plan();
        assert_eq!(plan.opaque, fresh.opaque, "opaque at camera {eye:?}");
        assert_eq!(
            plan.transparent, fresh.transparent,
            "transparent at camera {eye:?}"
        );
        assert_eq!(
            plan.invisible.len(),
            fresh.invisible,
            "invisible at camera {eye:?}"
        );
        assert_eq!(
            plan,
            plan_draw_order(&entries, Some(eye)),
            "cached plan must equal the fresh plan at camera {eye:?}"
        );
    }

    // `camera = None` (transparent by draw order only) must also match.
    let fresh = draw_passes(&accepted, &entries, None);
    cache.sort_transparent(None);
    let plan = cache.to_plan();
    assert_eq!(plan.opaque, fresh.opaque);
    assert_eq!(plan.transparent, fresh.transparent);
    assert_eq!(plan.invisible.len(), fresh.invisible);
    assert_eq!(plan, plan_draw_order(&entries, None));
}

/// The 3D display light is the reversed view direction, derived from the camera
/// itself - never a world constant. This is the CPU half of the shader's
/// camera-relative headlight (the GPU half is fenced by the oblique-cube pixel
/// tests: three distinct face shades that re-assign on a second azimuth).
#[test]
fn camera_headlight_points_from_the_target_toward_the_eye() {
    let plan = Camera3d::plan_view(point(0.0, 0.0, 5.0), point(0.0, 0.0, 0.0));
    assert_eq!(
        plan.headlight(),
        Some([0.0, 0.0, 1.0]),
        "a top-down camera lights the +Z face head-on"
    );
    // An oblique eye: the unit vector toward the eye, not a world axis.
    let oblique = Camera3d {
        eye: point(3.0, -4.0, 0.0),
        ..plan
    };
    let light = oblique.headlight().expect("a non-degenerate camera");
    assert!((light[0] - 0.6).abs() < 1e-6, "got {light:?}");
    assert!((light[1] + 0.8).abs() < 1e-6, "got {light:?}");
    assert_eq!(light[2], 0.0);
    // `eye == target` has no direction at all.
    let degenerate = Camera3d {
        target: point(3.0, -4.0, 0.0),
        ..oblique
    };
    assert_eq!(degenerate.headlight(), None);
}
