use super::*;
use cad_domain::{Completeness, DocumentId, EntityId, InstancePath, TaskStamp};

fn delta(revision: u64) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), revision),
        added: Vec::new(),
        removed_chunks: Vec::new(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

fn reference(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[test]
fn overlay_reuse_is_pointer_identity_on_a_retained_arc() {
    // The applied overlay is retained by strong reference, so its allocation
    // stays alive and a later rebuild cannot land on the same address. This
    // is what makes `Arc::ptr_eq` sound where the raw `usize` identity was
    // ABA-prone (a freed overlay reallocated at the same address between two
    // applies silently skipped the new overlay upload).
    let applied = Arc::new(delta(1));
    let runtime = CadRenderRuntime {
        applied_highlight: Some(applied.clone()),
        ..CadRenderRuntime::default()
    };
    assert!(
        overlay_identity_matches(runtime.applied_highlight.as_ref(), &applied),
        "the same allocation is reused"
    );
    assert_eq!(
        Arc::strong_count(&applied),
        2,
        "the runtime retains the overlay Arc, so its address cannot be reused"
    );
    drop(applied);
    // A rebuild is a distinct allocation: the fast path must not match it.
    let rebuilt = Arc::new(delta(2));
    assert!(
        !overlay_identity_matches(runtime.applied_highlight.as_ref(), &rebuilt),
        "a distinct overlay must not take the flags-only fast path"
    );
}

#[test]
fn flags_diagnostic_names_unmatched_refs_and_is_none_when_all_resolve() {
    assert_eq!(flags_diagnostic(&[]), None);
    let diagnostic = flags_diagnostic(&[reference(7), reference(9)]).expect("unmatched");
    assert!(diagnostic.starts_with("flags.unresolved: "));
    assert!(diagnostic.contains('7') && diagnostic.contains('9'));
}

#[test]
fn merge_diagnostics_preserves_the_image_part_across_a_flags_update() {
    assert_eq!(merge_diagnostics(None, None), None);
    assert_eq!(
        merge_diagnostics(Some("image: a"), None).as_deref(),
        Some("image: a")
    );
    assert_eq!(
        merge_diagnostics(None, Some("flags.unresolved: 3")).as_deref(),
        Some("flags.unresolved: 3")
    );
    assert_eq!(
        merge_diagnostics(Some("image: a"), Some("flags.unresolved: 3")).as_deref(),
        Some("image: a; flags.unresolved: 3")
    );
}

#[test]
fn applied_highlight_strong_count_is_observable_for_the_hardware_test() {
    // The hardware session test asserts the runtime retains the applied
    // overlay (the ABA guard); this keeps the helper exercised without a GPU.
    let runtime = CadRenderRuntime::default();
    assert!(runtime.applied_highlight_for_test().is_none());
    let applied = Arc::new(delta(1));
    let runtime = CadRenderRuntime {
        applied_highlight: Some(applied.clone()),
        ..CadRenderRuntime::default()
    };
    let retained = runtime.applied_highlight_for_test().expect("retained");
    assert!(Arc::ptr_eq(&retained, &applied));
    drop(retained);
    assert_eq!(
        Arc::strong_count(&applied),
        2,
        "the runtime retains the overlay Arc"
    );
}

#[test]
fn upload_decision_distinguishes_clear_start_continue_and_reset() {
    // No ready scene clears any pending upload.
    assert_eq!(
        upload_decision(false, Some(1), 1),
        UploadDecision::ClearPending
    );
    assert_eq!(
        upload_decision(false, None, 1),
        UploadDecision::ClearPending
    );
    // A ready scene with nothing pending starts a fresh upload.
    assert_eq!(upload_decision(true, None, 7), UploadDecision::Start);
    // Same revision continues (no reset, offset keeps advancing).
    assert_eq!(upload_decision(true, Some(7), 7), UploadDecision::Continue);
    // A different revision resets (the reset/starvation signal).
    assert_eq!(
        upload_decision(true, Some(7), 8),
        UploadDecision::ResetRevision
    );
}

#[test]
fn diagnostics_snapshot_reports_the_default_detached_state_without_side_effects() {
    let runtime = CadRenderRuntime::default();
    let diagnostics = runtime.diagnostics();
    assert_eq!(diagnostics.lifecycle, "detached");
    assert_eq!(diagnostics.applied_revision, None);
    assert_eq!(diagnostics.applied_base, None);
    assert_eq!(diagnostics.resident_base_batches, 0);
    assert!(!diagnostics.uploading);
    assert_eq!(diagnostics.upload, None);
    assert_eq!(diagnostics.frames_rendered, 0);
    assert_eq!(diagnostics.uploads_started, 0);
    assert_eq!(diagnostics.upload_revision_resets, 0);
    assert_eq!(diagnostics.ready_none_clears, 0);
    assert_eq!(diagnostics.last_upload_reset, None);
    assert_eq!(diagnostics.diagnostic, None);
    // Reading twice is stable (pure projection).
    assert_eq!(runtime.diagnostics(), diagnostics);
}

fn sample_observation(render_seq: u64) -> RenderStatsObservation {
    RenderStatsObservation {
        render_seq,
        device_epoch: 1,
        view_revision: 2,
        applied_revision: Some(1),
        applied_base: Some(1),
        uploads_started: 0,
        upload_revision_resets: 0,
        ready_none_clears: 0,
        cpu_ms: 0.0,
        gpu_ms: None,
        gpu_time: cad_render_wgpu::gpu_time::GpuTimeState::Disabled,
        plan_ms: 0.0,
        uniform_ms: 0.0,
        encode_ms: 0.0,
        submit_ms: 0.0,
        wait_ms: 0.0,
        draw_calls: 0,
        vertices: 0,
        triangles: 0,
        instanced_draws: 0,
        mesh_draws: 0,
        legacy_opaque_draws: 0,
        arc_draws: 0,
        transparent_batches: 0,
        image_batches: 0,
        cull_chunks_total: 0,
        cull_chunks_kept: 0,
        accelerated: false,
        pulled_state: cad_render_wgpu::PulledState::Unavailable,
        instance_streams: 0,
        instanced_batches: 0,
        tier: "universal",
        over_budget: false,
        budget_scale: 1.0,
        budget_target_ms: 13.0,
        budget_cost_ema_ms: 0.0,
        budget_vertices: 8_000_000,
        budget_triangles: 2_000_000,
        pacing: "active",
        progressive_cursor: None,
        progressive_total: None,
        progressive_complete: true,
    }
}

#[test]
fn render_observation_is_delivered_once_and_never_replayed() {
    // A retained/waiting frame (no actual render) has nothing to take.
    let mut runtime = CadRenderRuntime::default();
    assert_eq!(runtime.take_render_observation(), None);
    let mut runtime = CadRenderRuntime {
        pending_render_observation: Some(sample_observation(1)),
        ..CadRenderRuntime::default()
    };
    let first = runtime
        .take_render_observation()
        .expect("first actual render");
    assert_eq!(first.render_seq, 1);
    assert_eq!(first.gpu_ms, None, "gpu_ms stays honestly absent");
    // The SAME observation is never delivered twice (no replay on a retained
    // or waiting frame).
    assert_eq!(runtime.take_render_observation(), None);
    runtime.pending_render_observation = Some(sample_observation(2));
    assert_eq!(
        runtime.take_render_observation().map(|o| o.render_seq),
        Some(2)
    );
    assert_eq!(runtime.take_render_observation(), None);
}

fn upload_timing(scene_revision: u64, committed: bool) -> UploadTimingObservation {
    UploadTimingObservation {
        seq: 0,
        scene_revision,
        base_changed: true,
        committed,
        group: if committed { 2 } else { 0 },
        batches: 512,
        prepare_us: 1200,
        append_us: 300,
        commit_us: if committed { 4200 } else { 0 },
        total_us: if committed { 5700 } else { 1600 },
        commit: None,
    }
}

#[test]
fn upload_timing_observation_is_delivered_once_and_never_replayed() {
    let mut runtime = CadRenderRuntime::default();
    // No upload work -> nothing to take.
    assert_eq!(runtime.take_upload_timing_observation(), None);
    let mut runtime = CadRenderRuntime {
        pending_upload_timing: Some(upload_timing(3, false)),
        ..CadRenderRuntime::default()
    };
    // `record_upload_timing` assigns the sequence.
    let staged = runtime.pending_upload_timing.take().unwrap();
    runtime.record_upload_timing(staged);
    let observation = runtime.take_upload_timing_observation().expect("first");
    assert_eq!(observation.seq, 1);
    assert_eq!(observation.scene_revision, 3);
    assert!(!observation.committed);
    assert_eq!((observation.prepare_us, observation.append_us), (1200, 300));
    // The same observation is never delivered twice (no replay).
    assert_eq!(runtime.take_upload_timing_observation(), None);
}

#[test]
fn detach_invalidates_an_untaken_upload_timing_observation() {
    let mut runtime = CadRenderRuntime {
        pending_upload_timing: Some(upload_timing(1, true)),
        ..CadRenderRuntime::default()
    };
    let staged = runtime.pending_upload_timing.take().unwrap();
    runtime.record_upload_timing(staged);
    assert!(runtime.take_upload_timing_observation().is_some());
    // Inject a fresh, un-taken observation, then drop the device: it must not
    // be replayed afterwards.
    let mut observation = upload_timing(2, true);
    runtime.record_upload_timing(observation.clone());
    observation = runtime.pending_upload_timing.clone().unwrap();
    assert_eq!(observation.scene_revision, 2);
    runtime.detach();
    assert_eq!(runtime.take_upload_timing_observation(), None);
}

#[test]
fn detach_invalidates_an_untaken_render_observation() {
    let mut runtime = CadRenderRuntime {
        pending_render_observation: Some(sample_observation(5)),
        ..CadRenderRuntime::default()
    };
    // A device change must not let an old-epoch observation be replayed.
    runtime.detach();
    assert_eq!(runtime.take_render_observation(), None);
    assert_eq!(runtime.device_epoch(), 0);
    // The render sequence stays monotonic for the session.
    assert_eq!(runtime.render_seq, 0);
}

fn prepared_scene(revision: u64, base_revision: u64, flags_revision: u64) -> PreparedScene {
    PreparedScene {
        revision,
        base_revision,
        flags_revision,
        base: Arc::new(delta(revision)),
        highlight: Arc::new(delta(revision)),
        entity_flags: Vec::new(),
        highlight_diagnostics: Vec::new(),
        highlight_completeness: Completeness::Complete,
        images: None,
    }
}

#[test]
fn flags_delta_applies_only_over_a_committed_or_suppressed_scene() {
    let scene = prepared_scene(7, 3, 5);
    assert!(flags_delta_applies(Some(7), Some(4), false, false, &scene));
    assert!(
        flags_delta_applies(None, Some(4), false, true, &scene),
        "the suppressed base-only scene still takes the flags delta"
    );
    assert!(
        !flags_delta_applies(Some(7), Some(5), false, false, &scene),
        "the same flags revision is a no-op"
    );
    assert!(
        !flags_delta_applies(Some(6), Some(4), false, false, &scene),
        "a new scene revision must take the upload path (F4)"
    );
    assert!(
        !flags_delta_applies(Some(7), Some(4), true, false, &scene),
        "a pending upload keeps the upload path"
    );
    assert!(
        !flags_delta_applies(None, Some(4), false, false, &scene),
        "no applied scene and no suppression means no flags delta"
    );
    assert!(!flags_delta_applies(None, None, false, false, &scene));
}

/// A worker publication that lands after a synchronous flags refresh must not
/// regress the flags to the hover captured at submission time: the publication
/// boundary re-resolves them from the CURRENT overlay inputs, and because the
/// geometry allocations are unchanged it keeps the published scene revision
/// (never a fresh un-applied revision that would re-enter the base-only path).
#[test]
fn publication_recomputes_flags_from_the_latest_overlay_inputs() {
    use crate::render::publish_controller;

    let drawing = cad_app::host::HostController::with_demo_document([800.0, 600.0])
        .unwrap()
        .drawing()
        .unwrap();
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    session.sync_drawing(Some(drawing.clone()));
    session.prepare().unwrap();

    let published = session.current_prepared_scene().expect("published scene");
    let mut whole_entities = published
        .base
        .added
        .iter()
        .flat_map(|batch| batch.sources.iter())
        .filter(|reference| reference.sub_element.is_none())
        .cloned();
    let older = whole_entities.next().expect("a resident whole-entity ref");
    let latest = whole_entities
        .find(|reference| reference.entity != older.entity)
        .expect("a second resident whole-entity ref");

    let mut state = session.0.borrow_mut();
    // An in-flight preparation token. `advance_content` models a submission
    // whose inputs produced the same geometry as the published scene.
    state.advance_content();
    let token = state.content_revision;
    let document = state.view.document;
    let mut candidate = state.controller.clone();
    let overlays_older = OverlayInputs {
        hovered: SelectionSet::from_refs([older.clone()]),
        ..OverlayInputs::default()
    };
    candidate
        .prepare_shared_with_overlays_and_images(
            Some(drawing.clone()),
            document,
            state.fonts.clone(),
            &state.layers.clone(),
            state.view.space,
            &overlays_older,
            state.images.clone(),
        )
        .unwrap();

    // The hover moves after submission: applied synchronously to the state
    // controller, while the candidate still carries the older hover.
    let overlays_latest = OverlayInputs {
        hovered: SelectionSet::from_refs([latest.clone()]),
        ..OverlayInputs::default()
    };
    state.overlays = overlays_latest;
    state.refresh_flags_only();
    let scene_revision_before = state.controller.ready.as_ref().unwrap().revision;

    publish_controller(&mut state, candidate, token);

    let ready = state.controller.ready.as_ref().expect("published scene");
    assert_eq!(
        ready.revision, scene_revision_before,
        "a same-geometry publication must keep the published scene revision"
    );
    assert!(
        ready
            .entity_flags
            .iter()
            .any(|(flagged, _)| flagged.entity == latest.entity),
        "the published flags must reflect the latest hover"
    );
    assert!(
        !ready
            .entity_flags
            .iter()
            .any(|(flagged, _)| flagged.entity == older.entity),
        "the stale submitted hover must never be published"
    );
    assert_eq!(ready.flags_revision, state.flags_publication_revision);
}
