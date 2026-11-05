//! Pure contracts for the renderer-owned transient cursor (ADR 0028 phase 2a).
//!
//! No GPU: these pin the sentinel's sorted position/opaque-only placement, the
//! bounded packing, the dirty/`None` semantics and the drawn-batch accounting.

use super::*;

fn p(x: f64, y: f64) -> cad_domain::Point3 {
    cad_domain::Point3 { x, y, z: 0.0 }
}

/// A resident batch with no device buffers: `plan_overlay` only reads the
/// ordering fields, so `None` buffers are enough.
fn overlay_batch(draw_order: i64) -> GpuBatch {
    GpuBatch {
        vertices: None,
        vertex_count: 0,
        normals: None,
        colors: None,
        topology: RenderTopology::Lines,
        indices: None,
        index_count: 0,
        line_quads: None,
        line_quad_count: 0,
        mirrored: false,
        alpha: 1.0,
        color: [1.0, 1.0, 1.0],
        lineweight: 0.0,
        draw_order,
        centroid: [0.0; 3],
        origin: [0.0; 3],
        instanced: Vec::new(),
        mesh_runs: Vec::new(),
        arc_runs: Vec::new(),
        upload_bytes: 0,
        pick_key: 0,
        z_range: [0.0, 0.0],
        sources: Vec::new(),
        vertex_charge: None,
        index_charge: None,
    }
}

fn cursor(draw_order: i64) -> cad_scene::TransientCursor {
    cad_scene::TransientCursor {
        segments: vec![[p(0.1, 0.2), p(0.1, 0.4)], [p(0.0, 0.3), p(0.2, 0.3)]],
        color: [0.72, 0.75, 0.80, 1.0],
        draw_order,
    }
}

fn renderer_with_overlay() -> Renderer {
    Renderer {
        batches: vec![
            overlay_batch(2_000_000), // preview
            overlay_batch(3_000_000), // snap hints
        ],
        // Everything resident is overlay (no base prefix).
        base_end: 0,
        ..Renderer::default()
    }
}

#[test]
fn transient_sorts_into_the_opaque_walk_at_its_draw_order() {
    const PREVIEW: i64 = 2_000_000;
    const CURSOR: i64 = 2_500_000;
    const SNAP: i64 = 3_000_000;

    let renderer = renderer_with_overlay();
    // Between preview and snap: preview < cursor < snap (the fixed order).
    let plan = renderer.plan_overlay([0.0; 3], Some(&cursor(CURSOR)));
    assert!(plan.transient, "the sentinel must be in the plan");
    assert!(
        plan.transparent.is_empty(),
        "the transient is opaque-only, never transparent"
    );
    let sentinel = plan
        .opaque
        .iter()
        .position(|&index| index == plan::TRANSIENT_CURSOR)
        .expect("the sentinel is in the opaque walk");
    assert_eq!(
        plan.opaque,
        vec![0, plan::TRANSIENT_CURSOR, 1],
        "preview < cursor < snap"
    );
    assert_eq!(sentinel, 1);
    // `drawn_batches` counts resident overlay batches only.
    assert_eq!(plan.drawn_batches(), 2);
    assert_eq!(plan.opaque.len(), 3);

    // A draw order below/above the other overlay entries still sorts by value.
    let early = renderer.plan_overlay([0.0; 3], Some(&cursor(PREVIEW - 1)));
    assert_eq!(early.opaque, vec![plan::TRANSIENT_CURSOR, 0, 1]);
    let late = renderer.plan_overlay([0.0; 3], Some(&cursor(SNAP + 1)));
    assert_eq!(late.opaque, vec![0, 1, plan::TRANSIENT_CURSOR]);
    assert_eq!(late.drawn_batches(), 2);
}

#[test]
fn no_or_empty_transient_adds_no_sentinel_and_no_draw() {
    let renderer = renderer_with_overlay();
    let plan = renderer.plan_overlay([0.0; 3], None);
    assert!(!plan.transient);
    assert_eq!(plan.opaque, vec![0, 1]);
    assert_eq!(plan.drawn_batches(), 2);

    // An empty segment list is an explicit empty, identical to `None`.
    let empty = cad_scene::TransientCursor {
        segments: Vec::new(),
        color: [1.0, 1.0, 1.0, 1.0],
        draw_order: 2_500_000,
    };
    let plan = renderer.plan_overlay([0.0; 3], Some(&empty));
    assert!(!plan.transient);
    assert!(!plan.opaque.contains(&plan::TRANSIENT_CURSOR));
    assert_eq!(plan.drawn_batches(), 2);
}

#[test]
fn packing_is_bounded_and_uses_the_declared_instance_layout() {
    let cursor = cursor(2_500_000);
    let instances = plan::pack_transient_cursor(&cursor);
    assert_eq!(instances.len(), 2);
    // origin = segments[0][0] = (0.1, 0.2, 0): every coordinate is relative.
    assert_eq!(instances[0].p0_width, [0.0, 0.0, 0.0, 0.0]);
    assert_eq!(instances[0].p1_pad, [0.0, 0.2, 0.0, 0.0]);
    // Second segment: (0.0, 0.3) -> (-0.1, 0.1), (0.2, 0.3) -> (0.1, 0.1).
    assert_eq!(instances[1].p0_width, [-0.1, 0.1, 0.0, 0.0]);
    assert_eq!(instances[1].p1_pad, [0.1, 0.1, 0.0, 0.0]);
    // Cursor colour (already in range) and opaque instance flag; pick key 0.
    for instance in &instances {
        assert_eq!(instance.rgba, [0.72, 0.75, 0.80, 1.0]);
        assert_eq!(instance.id_flags, [0, plan::INSTANCE_FLAG_OPAQUE]);
    }
    // One fixed stride per segment.
    assert_eq!(
        line_instance_bytes(&instances).len(),
        2 * plan::LINE_INSTANCE_STRIDE as usize
    );

    // Over-long input is truncated to the bounded segment count.
    let mut many = cursor;
    many.segments = (0..10)
        .map(|i| {
            let x = i as f64 * 0.1;
            [p(x, 0.0), p(x + 0.05, 1.0)]
        })
        .collect();
    let packed = plan::pack_transient_cursor(&many);
    assert_eq!(packed.len(), cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS);
    assert!(
        line_instance_bytes(&packed).len()
            <= plan::LINE_INSTANCE_STRIDE as usize * cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS
    );
}

#[test]
fn setter_packs_and_marks_dirty_only_on_change() {
    let mut renderer = Renderer::default();
    assert!(renderer.cursor_transient.is_none());
    assert!(!renderer.cursor_transient_dirty);

    let cursor = cursor(2_500_000);
    assert!(renderer.set_cursor_transient(Some(cursor.clone())));
    assert!(renderer.cursor_transient_dirty);
    assert_eq!(
        renderer.cursor_transient_packed.len(),
        2 * plan::LINE_INSTANCE_STRIDE as usize
    );
    assert_eq!(renderer.cursor_transient(), Some(&cursor));

    // A repeated identical value is not a change (no dirty, no rewrite).
    renderer.cursor_transient_dirty = false;
    assert!(!renderer.set_cursor_transient(Some(cursor)));
    assert!(!renderer.cursor_transient_dirty);

    // A moved cursor is a change and repacks.
    let moved = cad_scene::TransientCursor {
        segments: vec![[p(0.5, 0.2), p(0.5, 0.4)]],
        ..cursor_for_move()
    };
    assert!(renderer.set_cursor_transient(Some(moved)));
    assert!(renderer.cursor_transient_dirty);
    assert_eq!(
        renderer.cursor_transient_packed.len(),
        plan::LINE_INSTANCE_STRIDE as usize
    );

    // Clearing is a change once, then a no-op; an empty list normalises to None.
    assert!(renderer.set_cursor_transient(None));
    assert!(renderer.cursor_transient.is_none());
    assert!(renderer.cursor_transient_packed.is_empty());
    assert!(!renderer.set_cursor_transient(None));
    let empty = cad_scene::TransientCursor {
        segments: Vec::new(),
        color: [1.0, 1.0, 1.0, 1.0],
        draw_order: 2_500_000,
    };
    assert!(!renderer.set_cursor_transient(Some(empty)));
    assert!(renderer.cursor_transient.is_none());

    // The setter never touches scene state: no batches, no dirty plan.
    assert!(renderer.batches.is_empty());
    assert!(renderer.draw_plan.is_none());
}

fn cursor_for_move() -> cad_scene::TransientCursor {
    cad_scene::TransientCursor {
        segments: vec![[p(0.0, 0.0), p(0.0, 1.0)]],
        color: [0.72, 0.75, 0.80, 1.0],
        draw_order: 2_500_000,
    }
}
