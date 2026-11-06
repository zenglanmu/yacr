//! Native GPU contracts for the in-place **SDF conic** overwrite.
//!
//! Split from `overwrite_gpu_tests` (the line/mesh contracts) to keep every file
//! under the repository's 1000-line code limit; the shared device/commit/pixel
//! helpers live there and are reused here.
//!
//! What is contracted (real headless GPU, auto-skip prints `SKIP ... NOT a pass`):
//!
//! 1. A larger radius plus a shifted centre re-packs against the stream's stored
//!    `f64` origin into the **existing** instance buffer: same batch slot, pick
//!    key, batch count, conic draw count, no stream allocation, draw plan intact,
//!    AABB widened by union, and the ink really moves.
//! 2. A changed conic count is `ConicNotUpdatable` and writes nothing.
//! 3. A move beyond `MAX_STREAM_RADIUS` of the stored origin is the same named
//!    refusal (the origin is never rewritten) and writes nothing.
//! 4. A batch split across several conic streams is written per run, in batch
//!    conic order.
//!
//! Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 conic_overwrite_gpu_tests
//! ```
use super::overwrite_gpu_tests::{
    circle_at, col_of, commit, gpu_or_skip, init, max_channel, pick_key_near, render_frame, row_of,
};
use cad_domain::Point3;
use cad_scene::RenderConic;
#[test]
fn conic_overwrite_moves_and_resizes_an_arc_without_reuploading_a_stream() {
    let Some(gpu) = gpu_or_skip("conic overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, false, "conic overwrite");
    commit(&mut renderer, &[circle_at(1, 0.0, 0.0, 0.2)]);
    let before_stats = render_frame(&mut renderer);
    assert!(
        !renderer.batches[0].arc_runs.is_empty(),
        "an opaque conic batch converts to a conic stream"
    );
    assert_eq!(
        before_stats.arc_batches, 1,
        "one batch draws through the SDF path"
    );
    let arc_draws = before_stats.arc_draws;
    let key = renderer.pick_key_for_batch(0).expect("resident batch");
    renderer
        .set_pick_key_flags(key, cad_scene::FLAG_SELECTED)
        .expect("select the batch");
    let run = renderer.batches[0].arc_runs[0];
    let uploads = renderer.arc_stream_uploads();
    let stream_count = renderer.arc_stream_count();
    let instances = renderer.arc_instance_count();
    let buffer_len = renderer.arc_streams[run.stream as usize].buffer.size();
    let batch_count = renderer.batch_count();
    let plan_before = renderer
        .draw_plan
        .as_ref()
        .map(|plan| plan.opaque().to_vec())
        .expect("a render built the draw plan");
    let plan_rebuilds = renderer.draw_plan_rebuilds();
    // The old ring's rightmost point (world 0.2, 0) draws and picks.
    let old_x = col_of(0.2) as u32;
    let old_y = row_of(0.0) as u32;
    assert!(
        max_channel(&renderer, old_x as i32, old_y as i32) > 40,
        "the old radius draws"
    );
    assert_eq!(
        pick_key_near(&mut renderer, old_x, old_y),
        Some(key),
        "the id pass resolves the old ring"
    );

    // Larger radius and a shifted centre: the same conic slot, re-packed.
    renderer
        .overwrite_batch(0, &circle_at(1, 0.3, 0.3, 0.25))
        .expect("a same-count conic re-packs against the stream origin");
    let after_stats = render_frame(&mut renderer);

    assert_eq!(
        renderer.batch_count(),
        batch_count,
        "no batch is added/removed"
    );
    assert_eq!(
        renderer.pick_key_for_batch(0),
        Some(key),
        "the pick key is unchanged"
    );
    assert_eq!(
        renderer.pick_key_flags(key) & cad_scene::FLAG_SELECTED,
        cad_scene::FLAG_SELECTED,
        "the selection flag still resolves to the same pick key"
    );
    assert_eq!(
        after_stats.arc_batches, 1,
        "the frame still draws one conic batch"
    );
    assert_eq!(
        after_stats.arc_draws, arc_draws,
        "the frame's conic draw count is unchanged"
    );
    assert_eq!(
        renderer.draw_plan_rebuilds(),
        plan_rebuilds,
        "the cached draw plan is not rebuilt"
    );
    assert_eq!(
        renderer
            .draw_plan
            .as_ref()
            .map(|plan| plan.opaque().to_vec()),
        Some(plan_before),
        "the frame's draw plan / batch order is unchanged"
    );
    assert_eq!(
        renderer.arc_stream_uploads(),
        uploads,
        "an overwrite never allocates/uploads a conic stream"
    );
    assert_eq!(
        renderer.arc_stream_count(),
        stream_count,
        "no stream is added"
    );
    assert_eq!(
        renderer.arc_instance_count(),
        instances,
        "the resident conic instance count is unchanged"
    );
    assert_eq!(
        renderer.arc_streams[run.stream as usize].buffer.size(),
        buffer_len,
        "the stream's instance buffer is the same length"
    );
    assert_eq!(renderer.batch_overwrites(), 1);
    // AABB widening only: the old box is still covered and the new extent is too.
    let aabb = renderer.arc_streams[run.stream as usize].aabb;
    assert!(
        aabb.min[0] <= -0.2 + 1e-9 && aabb.max[0] >= 0.55 - 1e-9,
        "the stream AABB is widened by union, never shrunk: {aabb:?}"
    );
    assert!(
        aabb.min[1] <= -0.2 + 1e-9 && aabb.max[1] >= 0.55 - 1e-9,
        "the stream AABB covers the shifted centre: {aabb:?}"
    );
    // The ink really moved: gone at the old rightmost point, present at the new.
    assert!(
        max_channel(&renderer, old_x as i32, old_y as i32) < 40,
        "the old radius no longer draws"
    );
    let new_x = col_of(0.55) as u32;
    let new_y = row_of(0.3) as u32;
    assert!(
        max_channel(&renderer, new_x as i32, new_y as i32) > 40,
        "the moved/resized conic draws at the new radius and centre"
    );
    assert_eq!(
        pick_key_near(&mut renderer, new_x, new_y),
        Some(key),
        "the moved conic keeps its pick key"
    );
}

#[test]
fn conic_overwrite_refuses_a_changed_conic_count() {
    let Some(gpu) = gpu_or_skip("conic count refusal") else {
        return;
    };
    let mut renderer = init(&gpu, false, "conic count refusal");
    commit(&mut renderer, &[circle_at(1, 0.0, 0.0, 0.2)]);
    render_frame(&mut renderer);
    let before = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    let uploads = renderer.arc_stream_uploads();

    // One conic replaced by two: the run coverage (1) no longer equals the
    // rebuilt conic count (2).
    let mut two = circle_at(1, 0.0, 0.0, 0.2);
    two.conics.push(RenderConic {
        center: Point3 {
            x: 0.5,
            y: 0.0,
            z: 0.0,
        },
        major: Point3 {
            x: 0.1,
            y: 0.0,
            z: 0.0,
        },
        minor: Point3 {
            x: 0.0,
            y: 0.1,
            z: 0.0,
        },
        start: 0.0,
        sweep: std::f64::consts::TAU,
        full: true,
    });
    assert_eq!(
        renderer.overwrite_batch(0, &two),
        Err(crate::BatchOverwriteRefusal::ConicNotUpdatable),
        "a changed conic count is refused by name"
    );
    assert_eq!(renderer.batch_overwrites(), 0);
    assert_eq!(
        renderer.arc_stream_uploads(),
        uploads,
        "a refusal uploads nothing"
    );
    render_frame(&mut renderer);
    let after = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    assert_eq!(
        before.pixels, after.pixels,
        "a refused conic overwrite writes nothing"
    );
}

#[test]
fn conic_overwrite_refuses_an_origin_that_no_longer_bounds_the_geometry() {
    let Some(gpu) = gpu_or_skip("conic origin refusal") else {
        return;
    };
    let mut renderer = init(&gpu, false, "conic origin refusal");
    commit(&mut renderer, &[circle_at(1, 0.0, 0.0, 0.2)]);
    render_frame(&mut renderer);
    let before = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    let run = renderer.batches[0].arc_runs[0];
    let origin = renderer.arc_streams[run.stream as usize].origin;

    // Beyond `MAX_STREAM_RADIUS` (8192) from the stream's stored origin.
    let far = circle_at(1, 0.0, 9000.0, 0.2);
    assert_eq!(
        renderer.overwrite_batch(0, &far),
        Err(crate::BatchOverwriteRefusal::ConicNotUpdatable),
        "the stream origin is never rewritten"
    );
    assert_eq!(
        renderer.arc_streams[run.stream as usize].origin, origin,
        "the refusal leaves the stream origin untouched"
    );
    assert_eq!(renderer.batch_overwrites(), 0);
    render_frame(&mut renderer);
    let after = renderer
        .read_target_rgba()
        .expect("read the presented frame");
    assert_eq!(
        before.pixels, after.pixels,
        "a refused conic overwrite writes nothing"
    );
}

#[test]
fn conic_overwrite_writes_a_multi_run_split_in_batch_order() {
    let Some(gpu) = gpu_or_skip("conic multi-run overwrite") else {
        return;
    };
    let mut renderer = init(&gpu, false, "conic multi-run overwrite");
    // Cap two instances per conic stream: three conics split 2 + 1 across two
    // streams, so the batch owns two runs and the packer's conic order is the
    // only thing that says which conic belongs to which run.
    renderer.set_max_instances_per_stream(2);
    let mut batch = circle_at(1, 0.0, 0.0, 0.1);
    for cx in [0.4, -0.4] {
        batch.conics.push(RenderConic {
            center: Point3 {
                x: cx,
                y: 0.0,
                z: 0.0,
            },
            major: Point3 {
                x: 0.1,
                y: 0.0,
                z: 0.0,
            },
            minor: Point3 {
                x: 0.0,
                y: 0.1,
                z: 0.0,
            },
            start: 0.0,
            sweep: std::f64::consts::TAU,
            full: true,
        });
    }
    commit(&mut renderer, &[batch.clone()]);
    render_frame(&mut renderer);
    assert_eq!(renderer.arc_stream_count(), 2, "the cap splits the batch");
    assert_eq!(
        renderer.batches[0].arc_runs.len(),
        2,
        "one run per conic stream piece"
    );
    assert_eq!(
        renderer.batches[0]
            .arc_runs
            .iter()
            .map(|run| run.count)
            .collect::<Vec<_>>(),
        vec![2, 1],
        "the split is 2 + 1 in batch order"
    );
    let uploads = renderer.arc_stream_uploads();
    let instances = renderer.arc_instance_count();
    let key = renderer.pick_key_for_batch(0).unwrap();

    // Move every conic and grow it, in the same order.
    let mut moved = batch.clone();
    for (conic, cx) in moved.conics.iter_mut().zip([0.0, 0.4, -0.4]) {
        conic.center = Point3 {
            x: cx,
            y: 0.4,
            z: 0.0,
        };
        conic.major = Point3 {
            x: 0.12,
            y: 0.0,
            z: 0.0,
        };
        conic.minor = Point3 {
            x: 0.0,
            y: 0.12,
            z: 0.0,
        };
    }
    renderer
        .overwrite_batch(0, &moved)
        .expect("both runs re-pack against their own stream origin");
    assert_eq!(
        renderer.arc_stream_uploads(),
        uploads,
        "the split overwrite allocates no stream"
    );
    assert_eq!(
        renderer.arc_instance_count(),
        instances,
        "the split overwrite keeps the instance count"
    );
    assert_eq!(renderer.pick_key_for_batch(0), Some(key));
    render_frame(&mut renderer);

    // Every new ring draws and every old one is gone.
    for cx in [0.0, 0.4, -0.4] {
        assert!(
            max_channel(&renderer, col_of(cx + 0.12), row_of(0.4)) > 40,
            "the moved conic at x={cx} draws (rightmost point of the new radius)"
        );
        assert!(
            max_channel(&renderer, col_of(cx + 0.1), row_of(0.0)) < 40,
            "the old ring at x={cx} no longer draws"
        );
    }
}
