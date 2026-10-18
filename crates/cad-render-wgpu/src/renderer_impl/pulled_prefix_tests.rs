//! Pure contracts for the retained pulled-prefix checkpoint + boundary-aware
//! segmentation (no GPU required).

use super::*;

fn mk_checkpoint(
    retained_batches: usize,
    instance_streams: usize,
    mesh_chunks: usize,
    line_segments: usize,
    mesh_segments: usize,
) -> PulledPrefixCheckpoint {
    PulledPrefixCheckpoint {
        device_generation: 1,
        cache_epoch: 0,
        retained_batches,
        instance_streams,
        mesh_chunks,
        arc_streams: 0,
        line_segments,
        mesh_segments,
        line_base_ranges: (0..line_segments).map(|i| (i, i + 1)).collect(),
        mesh_base_ranges: (0..mesh_segments).map(|i| (i, i + 1)).collect(),
    }
}

fn facts() -> CheckpointFacts {
    CheckpointFacts {
        device_generation: 1,
        cache_epoch: 0,
        accelerated: true,
        instance_streams: 4,
        mesh_chunks: 4,
        arc_streams: 0,
        line_segments: 4,
        mesh_segments: 4,
    }
}

#[test]
fn prefix_identity_count_accepts_only_identity_subsequences() {
    assert_eq!(Renderer::prefix_identity_count(&[], 0), Some(0));
    assert_eq!(Renderer::prefix_identity_count(&[0, 1, 2], 3), Some(3));
    assert_eq!(
        Renderer::prefix_identity_count(&[0, 1, 2, 0, 1], 3),
        Some(3)
    );
    // A shrunken prefix (lower indices only) is still identity.
    assert_eq!(Renderer::prefix_identity_count(&[0, 1], 3), Some(2));
    // Out of order (a resource first encountered later) is not identity.
    assert_eq!(Renderer::prefix_identity_count(&[1, 0], 3), None);
    assert_eq!(Renderer::prefix_identity_count(&[0, 2], 3), None);
    // An index beyond the retained range is a shared/foreign resource.
    assert_eq!(Renderer::prefix_identity_count(&[0, 1, 3], 3), None);
}

#[test]
fn boundary_ranges_isolate_the_base_prefix() {
    let counts = [4usize, 4, 4, 4];
    assert_eq!(
        Renderer::forced_boundary_ranges(&counts, usize::MAX, None),
        vec![(0, 4)]
    );
    let split = Renderer::forced_boundary_ranges(&counts, usize::MAX, Some(2));
    assert_eq!(split, vec![(0, 2), (2, 4)]);
    assert_eq!(Renderer::base_range_count(&split, Some(2)), 1);
    assert_eq!(
        split[..Renderer::base_range_count(&split, Some(2))],
        [(0, 2)]
    );
    // The base ranges are stable when the suffix grows.
    let grown = [4usize, 4, 4, 4, 4, 4];
    let base = Renderer::forced_boundary_ranges(&counts, 4, Some(2));
    let base_long = Renderer::forced_boundary_ranges(&grown, 4, Some(2));
    let n = Renderer::base_range_count(&base, Some(2));
    assert_eq!(
        base[..n],
        base_long[..Renderer::base_range_count(&base_long, Some(2))]
    );
    // Breaks only ever fall on a whole stream index.
    for (start, end) in &base_long {
        assert!(start < end && *end <= grown.len());
    }
}

#[test]
fn boundary_chunk_ranges_isolate_the_base_prefix() {
    let verts = [10usize, 10, 10, 10];
    let indices = [30usize, 30, 30, 30];
    assert_eq!(
        Renderer::forced_boundary_chunk_ranges(&verts, &indices, usize::MAX, usize::MAX, None),
        vec![(0, 4)]
    );
    let split =
        Renderer::forced_boundary_chunk_ranges(&verts, &indices, usize::MAX, usize::MAX, Some(3));
    assert_eq!(split, vec![(0, 3), (3, 4)]);
    assert_eq!(Renderer::base_range_count(&split, Some(3)), 1);
    // A vertex cap splits greedily; the base ranges stay stable when the
    // suffix grows.
    let base = Renderer::forced_boundary_chunk_ranges(&verts, &indices, 20, usize::MAX, Some(2));
    assert_eq!(base, vec![(0, 2), (2, 4)]);
    let grown = Renderer::forced_boundary_chunk_ranges(
        &[10usize, 10, 10, 10, 10, 10],
        &[30usize, 30, 30, 30, 30, 30],
        20,
        usize::MAX,
        Some(2),
    );
    let n = Renderer::base_range_count(&base, Some(2));
    assert_eq!(
        base[..n],
        grown[..Renderer::base_range_count(&grown, Some(2))]
    );
}

#[test]
fn checkpoint_admission_rejects_every_failure_mode() {
    let checkpoint = mk_checkpoint(10, 4, 4, 2, 2);
    let good = facts();
    assert!(Renderer::checkpoint_scalar_admits(&checkpoint, 10, &good));
    // No retained prefix.
    assert!(!Renderer::checkpoint_scalar_admits(&checkpoint, 0, &good));
    // A different retained prefix length.
    assert!(!Renderer::checkpoint_scalar_admits(&checkpoint, 9, &good));
    // A different device generation (rebuild).
    let mut rebuild = good;
    rebuild.device_generation = 2;
    assert!(!Renderer::checkpoint_scalar_admits(
        &checkpoint,
        10,
        &rebuild
    ));
    // An invalidated epoch (clear/loss/config change).
    let mut invalidated = good;
    invalidated.cache_epoch = 1;
    assert!(!Renderer::checkpoint_scalar_admits(
        &checkpoint,
        10,
        &invalidated
    ));
    // Not the accelerated tier.
    let mut universal = good;
    universal.accelerated = false;
    assert!(!Renderer::checkpoint_scalar_admits(
        &checkpoint,
        10,
        &universal
    ));
    // A resource shrank below the retained count.
    let mut shrunk = good;
    shrunk.instance_streams = 3;
    assert!(!Renderer::checkpoint_scalar_admits(
        &checkpoint,
        10,
        &shrunk
    ));
    // The retained segments for a class that owns resources are gone.
    let mut no_segments = good;
    no_segments.line_segments = 0;
    assert!(!Renderer::checkpoint_scalar_admits(
        &checkpoint,
        10,
        &no_segments
    ));
    // A checkpoint with no reusable base segments is refused outright.
    let empty = mk_checkpoint(10, 4, 4, 0, 0);
    assert!(!Renderer::checkpoint_scalar_admits(&empty, 10, &good));
}

#[test]
fn checkpoint_admission_requires_segments_only_for_classes_with_resources() {
    let good = facts();
    // Line-only scene: mesh has no resources, so no mesh segments are required.
    let line_only = mk_checkpoint(10, 4, 0, 2, 0);
    assert!(Renderer::checkpoint_scalar_admits(&line_only, 10, &good));
    // Mesh-only scene: line has no resources, so no line segments are required.
    let mesh_only = mk_checkpoint(10, 0, 4, 0, 2);
    assert!(Renderer::checkpoint_scalar_admits(&mesh_only, 10, &good));
    // Both classes present, both segmented.
    let both = mk_checkpoint(10, 4, 4, 2, 2);
    assert!(Renderer::checkpoint_scalar_admits(&both, 10, &good));
    // Neither pulled class owns resources (arcs-only base): refused.
    let neither = mk_checkpoint(10, 0, 0, 0, 0);
    assert!(!Renderer::checkpoint_scalar_admits(&neither, 10, &good));
    // A class with resources but no segment is refused (per class).
    let line_missing = mk_checkpoint(10, 4, 0, 0, 0);
    assert!(!Renderer::checkpoint_scalar_admits(
        &line_missing,
        10,
        &good
    ));
    let mesh_missing = mk_checkpoint(10, 0, 4, 0, 0);
    assert!(!Renderer::checkpoint_scalar_admits(
        &mesh_missing,
        10,
        &good
    ));
}

#[test]
fn ownership_proof_rejects_a_suffix_sharing_a_retained_resource() {
    // The prefix owns [0,2) in identity order; the suffix uses only new ones.
    assert_eq!(Renderer::owned_prefix_count(&[0, 1], &[2, 3], 4), Some(2));
    // A shrunken prefix (only the first resource) still owns [0,1).
    assert_eq!(Renderer::owned_prefix_count(&[0], &[1, 2], 3), Some(1));
    // Out-of-range prefix index is rejected.
    assert_eq!(Renderer::owned_prefix_count(&[0, 5], &[1], 6), None);
    // Non-identity prefix (a resource first encountered later) is rejected.
    assert_eq!(Renderer::owned_prefix_count(&[1, 0], &[2], 3), None);
    // A SUFFIX that references a retained resource is rejected (shared
    // resource across the boundary).
    assert_eq!(Renderer::owned_prefix_count(&[0, 1], &[2, 0], 3), None);
    // An out-of-range SUFFIX index is rejected too.
    assert_eq!(Renderer::owned_prefix_count(&[0, 1], &[7], 3), None);
    // Empty prefix owns nothing and any suffix index is fine.
    assert_eq!(Renderer::owned_prefix_count(&[], &[0], 1), Some(0));
}

#[test]
fn reuse_completion_requires_every_owned_class_to_have_completed() {
    let both = mk_checkpoint(10, 4, 4, 2, 2);
    let line_ranges = both.line_base_ranges.clone();
    let mesh_ranges = both.mesh_base_ranges.clone();
    // Both classes present with the recorded ranges: completed.
    assert!(Renderer::reuse_completion(
        &both,
        true,
        true,
        &line_ranges,
        &mesh_ranges
    ));
    // A class fell back to the universal path (pulled class gone): declined.
    assert!(!Renderer::reuse_completion(
        &both,
        false,
        true,
        &line_ranges,
        &mesh_ranges
    ));
    assert!(!Renderer::reuse_completion(
        &both,
        true,
        false,
        &line_ranges,
        &mesh_ranges
    ));
    // Fewer ranges than the checkpoint recorded (a segment was not reused).
    assert!(!Renderer::reuse_completion(
        &both,
        true,
        true,
        &line_ranges[..1],
        &mesh_ranges
    ));
    // Ranges that differ from the checkpoint's record: declined.
    assert!(!Renderer::reuse_completion(
        &both,
        true,
        true,
        &[(9, 10), (10, 11)],
        &mesh_ranges
    ));
    // Line-only: the mesh class requires nothing.
    let line_only = mk_checkpoint(10, 4, 0, 2, 0);
    assert!(Renderer::reuse_completion(
        &line_only,
        true,
        false,
        &line_only.line_base_ranges,
        &[]
    ));
    assert!(!Renderer::reuse_completion(
        &line_only,
        false,
        false,
        &line_only.line_base_ranges,
        &[]
    ));
    // Mesh-only: the line class requires nothing.
    let mesh_only = mk_checkpoint(10, 0, 4, 0, 2);
    assert!(Renderer::reuse_completion(
        &mesh_only,
        false,
        true,
        &[],
        &mesh_only.mesh_base_ranges
    ));
    // No pulled class at all: never completed.
    let none = mk_checkpoint(10, 0, 0, 0, 0);
    assert!(!Renderer::reuse_completion(&none, false, false, &[], &[]));
}

#[test]
fn prefix_outcome_never_counts_a_failed_reuse() {
    use super::PrefixOutcome as O;
    // A successful reuse counts; a failed reuse invalidates (and never counts).
    assert_eq!(prefix_outcome(true, false, None), O::CountReuse);
    assert_eq!(prefix_outcome(true, true, None), O::Invalidate);
    // A full rebuild with a prefix establishes; without one it invalidates.
    assert_eq!(prefix_outcome(false, false, Some(5)), O::Establish);
    assert_eq!(prefix_outcome(false, false, None), O::Invalidate);
    assert_eq!(prefix_outcome(false, true, Some(5)), O::Invalidate);
    // A reuse plan never establishes.
    assert_eq!(prefix_outcome(true, false, Some(5)), O::CountReuse);
}

#[test]
fn invalidation_clears_the_checkpoint_and_bumps_the_epoch() {
    let mut renderer = Renderer {
        pulled_prefix_checkpoint: Some(mk_checkpoint(10, 4, 4, 2, 2)),
        ..Renderer::default()
    };
    let epoch = renderer.pulled_cache_epoch;
    renderer.invalidate_pulled_prefix();
    assert!(renderer.pulled_prefix_checkpoint.is_none());
    assert_eq!(renderer.pulled_cache_epoch, epoch + 1);
    // The test-hook override changes the segmentation, so it invalidates too.
    renderer.pulled_prefix_checkpoint = Some(mk_checkpoint(10, 4, 4, 2, 2));
    renderer.set_pulled_line_segment_elems_override(Some(2));
    assert!(renderer.pulled_prefix_checkpoint.is_none());
    renderer.set_pulled_line_segment_elems_override(None);
    // The binding-limit override changes the segmentation capacity too.
    let epoch = renderer.pulled_cache_epoch;
    renderer.pulled_prefix_checkpoint = Some(mk_checkpoint(10, 4, 4, 2, 2));
    renderer.set_pulled_binding_limit_override(Some(1 << 20));
    assert!(renderer.pulled_prefix_checkpoint.is_none());
    assert_eq!(renderer.pulled_cache_epoch, epoch + 1);
    renderer.set_pulled_binding_limit_override(None);
    // Counters start at zero (diagnostics only).
    assert_eq!(renderer.pulled_prefix_reuses(), 0);
    assert_eq!(renderer.pulled_prefix_rebuilds(), 0);
}

#[test]
fn plan_is_full_without_a_checkpoint_and_establishes_only_with_a_prefix() {
    let renderer = Renderer::default();
    // No checkpoint: a base replacement (keep_prefix 0) establishes nothing.
    assert!(matches!(
        renderer.pulled_fastpath_plan(0, &IncomingResources::default()),
        PulledFastPath::Full { establish: None }
    ));
    // A surviving prefix asks for a checkpoint to be established.
    assert!(matches!(
        renderer.pulled_fastpath_plan(7, &IncomingResources::default()),
        PulledFastPath::Full { establish: Some(7) }
    ));
}

#[test]
fn incoming_resources_must_be_disjoint_from_the_retained_prefix() {
    // A suffix of new resources is disjoint.
    let disjoint = IncomingResources {
        streams: vec![4, 5],
        meshes: vec![2],
        arcs: vec![],
    };
    assert!(disjoint.disjoint_from(4, 2, 0));
    // A suffix that references a retained resource is rejected.
    let shared = IncomingResources {
        streams: vec![4, 1],
        meshes: vec![],
        arcs: vec![],
    };
    assert!(!shared.disjoint_from(4, 2, 0));
    // A retained mesh chunk referenced by the suffix is rejected.
    let shared_mesh = IncomingResources {
        streams: vec![],
        meshes: vec![0],
        arcs: vec![2],
    };
    assert!(!shared_mesh.disjoint_from(4, 2, 2));
    // No retained resources: everything is disjoint.
    assert!(shared_mesh.disjoint_from(0, 0, 0));
}
