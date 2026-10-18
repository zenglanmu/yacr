use super::*;

// ---------------------------------------------------------------------------
// 1. Real buffer identity across admitted suffix commits
// ---------------------------------------------------------------------------

#[test]
fn retained_base_geometry_is_the_same_wgpu_buffers_across_repeated_suffix_commits() {
    const WHAT: &str = "retained_base_geometry";
    let Some(gpu) = gpu_or_skip(WHAT) else {
        return;
    };
    if !accelerated_supported(&gpu, WHAT) {
        return;
    }
    let mut renderer = init_accelerated(&gpu, WHAT);
    let target = RenderTarget::new(64, 64);
    let base = base_batches();
    let base_len = base.len();

    // Base commit with no retained prefix: nothing to establish yet.
    commit(&mut renderer, &base, 0);
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a base commit with no prefix must not establish a checkpoint"
    );

    // First suffix commit: the full warm-up that establishes the checkpoint.
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    let checkpoint = renderer
        .pulled_prefix_checkpoint
        .clone()
        .expect("the warm-up must establish a checkpoint");
    assert_eq!(checkpoint.retained_batches, base_len, "{WHAT}");
    assert!(
        checkpoint.instance_streams >= 1 && checkpoint.mesh_chunks >= 1,
        "{WHAT}: the fixture must retain resources of BOTH pulled classes (lines={}, meshes={})",
        checkpoint.instance_streams,
        checkpoint.mesh_chunks
    );
    assert!(
        checkpoint.line_segments >= 1 && checkpoint.mesh_segments >= 1,
        "{WHAT}: the warm-up must record base segments of both pulled classes"
    );
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: the warm-up is a full rebuild, not a reuse"
    );
    assert!(renderer.pulled_prefix_rebuilds() >= 1, "{WHAT}");

    // Capture the real base buffers and the chunk tables of the warm-up.
    let base_line = base_line_buffers(&renderer, checkpoint.line_segments);
    let base_mesh = base_mesh_buffers(&renderer, checkpoint.mesh_segments);
    let line_table_before = renderer
        .pulled_lines
        .as_ref()
        .expect("line storage")
        .chunk_table
        .clone();
    let mesh_table_before = renderer
        .pulled_mesh
        .as_ref()
        .expect("mesh storage")
        .chunk_table
        .clone();
    let streams_after_warmup = renderer.instance_stream_count();
    let chunks_after_warmup = renderer.mesh_chunk_count();

    // Second suffix commit: admitted reuse of the retained base.
    commit(&mut renderer, &suffix_batches(20, -0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "{WHAT}: the second suffix commit must reuse the retained base"
    );
    assert_eq!(
        renderer.instance_stream_count(),
        streams_after_warmup,
        "{WHAT}: the replaced suffix's streams must be collected, not accumulated"
    );
    assert_eq!(
        renderer.mesh_chunk_count(),
        chunks_after_warmup,
        "{WHAT}: the replaced suffix's mesh chunks must be collected, not accumulated"
    );
    assert_same_line_buffers(&renderer, &base_line, WHAT);
    assert_same_mesh_buffers(&renderer, &base_mesh, WHAT);

    // Identity is discriminating: the chunk table is a *new* buffer every commit,
    // so a same-buffer assertion above cannot be satisfied vacuously.
    assert!(
        renderer
            .pulled_lines
            .as_ref()
            .expect("line storage")
            .chunk_table
            != line_table_before,
        "{WHAT}: the line chunk table must be rebuilt (a fresh buffer) on every commit"
    );
    assert!(
        renderer
            .pulled_mesh
            .as_ref()
            .expect("mesh storage")
            .chunk_table
            != mesh_table_before,
        "{WHAT}: the mesh chunk table must be rebuilt (a fresh buffer) on every commit"
    );

    // The new suffix is fresh geometry, never one of the retained base buffers.
    let all_line_after_reuse = line_buffers(&renderer);
    assert!(
        all_line_after_reuse.len() > base_line.len(),
        "{WHAT}: the reuse must still build a suffix segment"
    );
    let suffix_line = &all_line_after_reuse[base_line.len()..];
    assert!(
        all_distinct(suffix_line, &base_line),
        "{WHAT}: the rebuilt suffix must use fresh buffers, distinct from the retained base"
    );
    let all_mesh_after_reuse = mesh_buffers(&renderer);
    let suffix_mesh = &all_mesh_after_reuse[base_mesh.len()..];
    assert!(
        suffix_mesh
            .iter()
            .all(|suffix| base_mesh.iter().all(|base| suffix != base)),
        "{WHAT}: the rebuilt mesh suffix must use fresh buffers, distinct from the retained base"
    );

    // Repeat: the base identity never changes and the resident resources stay
    // bounded (the previous suffix is collected every time).
    let streams_after_reuse = renderer.instance_stream_count();
    let chunks_after_reuse = renderer.mesh_chunk_count();
    let line_segments_after_reuse = renderer.pulled_line_segment_count();
    let mesh_segments_after_reuse = renderer.pulled_mesh_segment_count();
    for (index, y) in [0.85_f32, -0.45, 0.45].into_iter().enumerate() {
        commit(
            &mut renderer,
            &suffix_batches(30 + (index as u128) * 2, y),
            base_len,
        );
        assert_eq!(
            renderer.pulled_prefix_reuses(),
            2 + index as u64,
            "{WHAT}: repeat {index} must reuse the retained base again"
        );
        assert_eq!(
            renderer.instance_stream_count(),
            streams_after_reuse,
            "{WHAT}: repeat {index} must not accumulate instance streams"
        );
        assert_eq!(
            renderer.mesh_chunk_count(),
            chunks_after_reuse,
            "{WHAT}: repeat {index} must not accumulate mesh chunks"
        );
        assert_eq!(
            renderer.pulled_line_segment_count(),
            line_segments_after_reuse,
            "{WHAT}: repeat {index} must not accumulate pulled line segments"
        );
        assert_eq!(
            renderer.pulled_mesh_segment_count(),
            mesh_segments_after_reuse,
            "{WHAT}: repeat {index} must not accumulate pulled mesh segments"
        );
        assert_same_line_buffers(&renderer, &base_line, WHAT);
        assert_same_mesh_buffers(&renderer, &base_mesh, WHAT);
    }

    // The retained base still submits and draws after all the reuse commits.
    assert_draws_and_picks(&mut renderer, &target, WHAT);
}

// ---------------------------------------------------------------------------
// 2. `clear_batches` invalidates, so the next commit is a full warm-up
// ---------------------------------------------------------------------------

#[test]
fn clear_batches_forces_the_next_commit_to_be_a_full_warmup() {
    const WHAT: &str = "clear_invalidates";
    let Some(gpu) = gpu_or_skip(WHAT) else {
        return;
    };
    if !accelerated_supported(&gpu, WHAT) {
        return;
    }
    let mut renderer = init_accelerated(&gpu, WHAT);
    let base = base_batches();
    let base_len = base.len();

    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: the warm-up must establish a checkpoint"
    );
    let epoch_before = renderer.pulled_cache_epoch;

    renderer.clear_batches();
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: clear_batches must invalidate the retained prefix"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before,
        "{WHAT}: clear_batches must bump the pulled cache epoch"
    );
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{WHAT}: clear_batches must drop the pulled storage"
    );

    // A commit after the clear cannot reuse anything: the checkpoint is gone.
    commit(&mut renderer, &base, 0);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: no commit after clear_batches may count a reuse"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a prefix-less commit must not establish a checkpoint"
    );

    // The first prefix-bearing commit after the clear is a full warm-up again.
    let rebuilds_before = renderer.pulled_prefix_rebuilds();
    commit(&mut renderer, &suffix_batches(20, -0.75), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        0,
        "{WHAT}: the post-clear suffix commit must be a full rebuild, not a reuse"
    );
    assert!(
        renderer.pulled_prefix_rebuilds() > rebuilds_before,
        "{WHAT}: the post-clear suffix commit must re-establish the checkpoint"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a fresh checkpoint must be established after the full warm-up"
    );

    // ...and only the commit after that reuses.
    commit(&mut renderer, &suffix_batches(30, 0.45), base_len);
    assert_eq!(
        renderer.pulled_prefix_reuses(),
        1,
        "{WHAT}: the commit after the post-clear warm-up is the first that may reuse"
    );
}

// ---------------------------------------------------------------------------
// 3. Device recovery: documented `rebuild_device` teardown + host re-attach
//    (stale uploads rejected, new generation/device rebuilds, draws + picks)
// ---------------------------------------------------------------------------

#[test]
fn device_reinit_rejects_a_stale_upload_and_rebuilds_on_the_new_generation() {
    const WHAT: &str = "device_reinit";
    // One shared `wgpu::Instance`, two distinct device handles (see the fixture's
    // doc comment: wgpu 30's device equality is hub-local, so a shared hub is what
    // makes "a genuinely different device" assertable).
    let Some((instance, first_device, second_device)) = two_devices_from_one_instance(WHAT) else {
        return;
    };
    // The shared hub must stay alive for the whole contract: both device ids are
    // hub-local and the handles below are compared across the recoveries.
    let _shared_instance = instance;
    if !accelerated_supported(&first_device, WHAT) {
        return;
    }
    let target = RenderTarget::new(64, 64);
    let base = base_batches();
    let base_len = base.len();

    let mut renderer = init_accelerated(&first_device, WHAT);
    let generation_a = renderer.device_generation;
    commit(&mut renderer, &base, 0);
    commit(&mut renderer, &suffix_batches(10, 0.75), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: the warm-up must establish a checkpoint before the reset"
    );
    let line_before_reset = line_buffers(&renderer);
    let mesh_before_reset = mesh_buffers(&renderer);
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // The attachments the documented teardown must clear actually exist here.
    assert!(
        renderer.flags_ctl_buffer.is_some()
            && renderer.flags_texture.is_some()
            && renderer.flags_bind_group.is_some(),
        "{WHAT}: the selection-flags attachments must exist before the teardown"
    );
    assert!(
        renderer.pick_target.is_some() && renderer.pick_view.is_some(),
        "{WHAT}: the id target must exist before the teardown"
    );

    // An upload staged on generation A, committed after the recovery.
    let stale_same_gpu = renderer
        .prepare_upload_batches(&suffix_batches(20, -0.75))
        .expect("stage an upload before the recovery");

    // Documented recovery sequence (`Renderer::rebuild_device` docs, renderer_impl.rs
    // 3576-3643): the host tears the derived GPU state down first and only then
    // hands in a device; re-initializing while still attached is **not** supported:
    // it leaves the previous attachment's flags control buffer (and id target)
    // behind, and `ensure_flags` then rebuilds its bind group from that stale buffer
    // — observed as a fatal wgpu-core panic ("Cannot get non-existent resource
    // BufferId(2,1)") in the pre-correction run (logs 01/02). That path is a product
    // gap recorded outside this contract and deliberately **not** fixed here.
    let epoch_before_teardown = renderer.pulled_cache_epoch;
    let rebuild = renderer.rebuild_device(&target);
    assert_eq!(
        rebuild,
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into()
        )),
        "{WHAT}: the documented teardown must require the host to hand in a new device"
    );
    assert!(
        renderer.device_lost,
        "{WHAT}: the teardown must mark the device as lost"
    );
    // Every resource `rebuild_device` clears (renderer_impl.rs 3581-3643) is
    // verified before the new device is handed in: the checkpoint (invalidated),
    // both pulled classes, the selection-flags attachments, the id target, the
    // resident batches/streams, and the device/queue themselves. Counters that
    // `rebuild_device` does NOT touch (the monotonic reuse/rebuild diagnostics and
    // the device generation) are deliberately not asserted here.
    assert!(
        renderer.pulled_prefix_checkpoint.is_none()
            && renderer.pulled_lines.is_none()
            && renderer.pulled_mesh.is_none(),
        "{WHAT}: the teardown must drop the checkpoint and the pulled storage"
    );
    assert!(
        renderer.flags_ctl_buffer.is_none()
            && renderer.flags_texture.is_none()
            && renderer.flags_view.is_none()
            && renderer.flags_bind_group.is_none(),
        "{WHAT}: the teardown must drop the selection-flags attachments"
    );
    assert!(
        renderer.pick_target.is_none()
            && renderer.pick_view.is_none()
            && renderer.pick_depth_view.is_none(),
        "{WHAT}: the teardown must drop the id target"
    );
    assert_eq!(
        renderer.pick_size,
        (0, 0),
        "{WHAT}: the teardown must reset the id target size"
    );
    assert_eq!(
        renderer.batch_count(),
        0,
        "{WHAT}: the teardown must drop every resident batch"
    );
    assert_eq!(renderer.instance_stream_count(), 0, "{WHAT}");
    assert_eq!(renderer.mesh_chunk_count(), 0, "{WHAT}");
    assert!(
        renderer.device.is_none() && renderer.queue.is_none(),
        "{WHAT}: the teardown must drop the device/queue until the host hands a new one in"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before_teardown,
        "{WHAT}: the teardown must invalidate the pulled cache epoch"
    );

    // Re-attach the *same* `wgpu::Device`: the device identity is unchanged, so
    // this isolates the device-**generation** rejection.
    let epoch_before = renderer.pulled_cache_epoch;
    renderer
        .initialize_with_device(first_device.device.clone(), first_device.queue.clone())
        .expect("re-attach the same device after the teardown");
    assert!(
        !renderer.device_lost,
        "{WHAT}: a successful attach must clear the device-lost state (so the stale upload cannot be rejected by it)"
    );
    assert!(
        renderer.device.as_ref() == Some(&first_device.device),
        "{WHAT}: the re-attach must hold the same device handle"
    );
    assert_eq!(
        renderer.device_generation,
        generation_a + 1,
        "{WHAT}: a successful re-attach establishes a new device generation"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before,
        "{WHAT}: the re-attach must invalidate the pulled cache epoch"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a new generation must not keep the previous checkpoint"
    );
    assert!(
        renderer.pulled_lines.is_none() && renderer.pulled_mesh.is_none(),
        "{WHAT}: the re-attach must drop the pulled storage"
    );
    assert_eq!(
        renderer.batch_count(),
        0,
        "{WHAT}: the re-attach must drop every resident batch"
    );
    assert!(
        matches!(
            renderer.commit_upload(stale_same_gpu, base_len),
            Err(CadError::StaleResult)
        ),
        "{WHAT}: an upload staged before the re-attach must be rejected by the generation check"
    );

    // A fresh base on the new generation: fresh buffers, valid draws + picks.
    commit(&mut renderer, &base, 0);
    let line_after_same_gpu_reinit = line_buffers(&renderer);
    assert!(
        all_distinct(&line_after_same_gpu_reinit, &line_before_reset),
        "{WHAT}: the re-initialized renderer must build a fresh base, never reuse the pre-re-init buffers"
    );
    assert!(
        all_distinct(&mesh_buffers(&renderer), &mesh_before_reset),
        "{WHAT}: the re-initialized renderer must build fresh mesh buffers"
    );
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // Re-establish a pulled prefix (and keep the flags/id-target attachments the
    // `assert_draws_and_picks` above created) so the second teardown has real state
    // to clear, then stage an upload on this device's generation.
    commit(&mut renderer, &suffix_batches(30, -0.85), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a checkpoint must exist before the second teardown"
    );
    assert!(
        renderer.flags_ctl_buffer.is_some() && renderer.pick_target.is_some(),
        "{WHAT}: the flags attachments and the id target must exist before the second teardown"
    );
    let stale_across_devices = renderer
        .prepare_upload_batches(&suffix_batches(40, -0.45))
        .expect("stage an upload before the device swap");
    let generation_on_first_device = renderer.device_generation;

    // A genuinely distinct device handle from the same shared instance: its
    // hub-local id differs from the first device's (asserted in the fixture), so the
    // identity difference below is meaningful rather than a cross-instance id clash.
    if !accelerated_supported(&second_device, WHAT) {
        return;
    }
    // Same documented recovery sequence before the other device is handed in.
    let rebuild_before_swap = renderer.rebuild_device(&target);
    assert_eq!(
        rebuild_before_swap,
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into()
        )),
        "{WHAT}: the second teardown must require the host to hand in a new device"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none()
            && renderer.pulled_lines.is_none()
            && renderer.pulled_mesh.is_none(),
        "{WHAT}: the second teardown must drop the checkpoint and the pulled storage"
    );
    assert!(
        renderer.flags_ctl_buffer.is_none()
            && renderer.flags_texture.is_none()
            && renderer.flags_bind_group.is_none(),
        "{WHAT}: the second teardown must drop the selection-flags attachments"
    );
    assert!(
        renderer.pick_target.is_none()
            && renderer.pick_view.is_none()
            && renderer.pick_size == (0, 0),
        "{WHAT}: the second teardown must drop the id target"
    );
    assert!(
        renderer.device.is_none() && renderer.queue.is_none(),
        "{WHAT}: the second teardown must drop the device/queue until the host hands a new one in"
    );
    let epoch_before_swap = renderer.pulled_cache_epoch;
    renderer
        .initialize_with_device(second_device.device.clone(), second_device.queue.clone())
        .expect("attach the second device after the teardown");
    assert!(
        renderer.device.as_ref() == Some(&second_device.device),
        "{WHAT}: the renderer must hold the second device's handle"
    );
    assert!(
        renderer.device.as_ref() != Some(&first_device.device),
        "{WHAT}: the second attach must hold a genuinely different device handle (distinct hub-local id in the shared instance), not the first one"
    );
    assert!(
        !renderer.device_lost,
        "{WHAT}: a successful attach must clear the device-lost state (so the stale upload cannot be rejected by it)"
    );
    assert_eq!(
        renderer.device_generation,
        generation_on_first_device + 1,
        "{WHAT}: attaching another device must advance the generation"
    );
    assert!(
        renderer.pulled_cache_epoch > epoch_before_swap,
        "{WHAT}: attaching another device must invalidate the pulled cache epoch"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: attaching another device must not keep the previous checkpoint"
    );
    assert_eq!(renderer.batch_count(), 0, "{WHAT}");
    assert!(
        matches!(
            renderer.commit_upload(stale_across_devices, base_len),
            Err(CadError::StaleResult)
        ),
        "{WHAT}: an upload staged on the previous device must be rejected"
    );

    // A valid base on the new device: fresh buffers, and it draws **and picks**.
    // The documented teardown also cleared the id target, so `ensure_pick_target`
    // rebuilds it on this device — the previous attachment's id target can no
    // longer be reached (this is why the pre-correction "draw-only" limitation is
    // gone).
    commit(&mut renderer, &base, 0);
    let line_on_second_device = line_buffers(&renderer);
    assert!(
        all_distinct(&line_on_second_device, &line_after_same_gpu_reinit),
        "{WHAT}: the second device must build its own base buffers"
    );
    assert_draws_and_picks(&mut renderer, &target, WHAT);

    // Distinguish a same-device full replacement: it invalidates the checkpoint
    // and rebuilds the geometry, but the device generation must NOT move (it is
    // not a device recovery, and must not be recorded as one).
    commit(&mut renderer, &suffix_batches(40, 0.45), base_len);
    assert!(
        renderer.pulled_prefix_checkpoint.is_some(),
        "{WHAT}: a checkpoint must exist before the full replacement"
    );
    let line_before_replace = line_buffers(&renderer);
    let generation_before_replace = renderer.device_generation;
    commit(&mut renderer, &base, 0);
    assert_eq!(
        renderer.device_generation,
        generation_before_replace,
        "{WHAT}: a full replacement on the same device is NOT a device recovery; the generation must not move"
    );
    assert!(
        renderer.pulled_prefix_checkpoint.is_none(),
        "{WHAT}: a full replacement invalidates the retained prefix"
    );
    assert!(
        all_distinct(&line_buffers(&renderer), &line_before_replace),
        "{WHAT}: a full replacement rebuilds the pulled geometry"
    );
}
