//! Pure contracts for the pulled capacity gate (no GPU required).
//!
//! Byte-identity of the two paths under a skip, and the reported state, are
//! GPU-tested in `crate::pulled_budget_gpu_tests`.

use super::*;

#[test]
fn payload_bytes_count_every_record_and_table_entry() {
    // Three and two line instances; four mesh vertices with six indices.
    // Every term is derived from the real packed types, not from a constant.
    let line_record = std::mem::size_of::<PulledLine>() as u64;
    let mesh_record = std::mem::size_of::<PulledMeshVertex>() as u64;
    let table_entry = std::mem::size_of::<[f32; 4]>() as u64;
    assert_eq!(line_record, u64::from(PULLED_LINE_STRIDE));
    assert_eq!(mesh_record, u64::from(PULLED_MESH_VERTEX_STRIDE));
    let expected = 5 * line_record   // instances
        + 2 * table_entry            // one chunk-table entry per line stream
        + 4 * mesh_record            // merged-mesh vertices
        + 6 * 4                      // rebased mesh indices
        + table_entry; // one chunk-table entry per mesh chunk
    assert_eq!(pulled_payload_bytes(&[3, 2], &[4], &[6]), expected);
    // No converted geometry: nothing would be allocated.
    assert_eq!(pulled_payload_bytes(&[], &[], &[]), 0);
    // A mesh-only class allocates no line payload or line table.
    assert_eq!(
        pulled_payload_bytes(&[], &[4], &[6]),
        4 * mesh_record + 6 * 4 + table_entry
    );
}

#[test]
fn capacity_gate_rejects_only_requirements_above_the_budget() {
    assert!(pulled_capacity_fits(0, 0));
    // An exact fit is allowed: the budget is the renderer's own ceiling.
    assert!(pulled_capacity_fits(1_000, 1_000));
    assert!(pulled_capacity_fits(999, 1_000));
    // One byte over is rejected.
    assert!(!pulled_capacity_fits(1_001, 1_000));
    assert!(!pulled_capacity_fits(u64::MAX, 1_000));
    assert!(!pulled_capacity_fits(1, 0));
}

#[test]
fn budget_is_the_device_single_allocation_limit() {
    // Derived from `Limits` only, with no machine or scene constant.
    for limits in [
        wgpu::Limits::default(),
        wgpu::Limits::downlevel_defaults(),
        wgpu::Limits::downlevel_webgl2_defaults(),
    ] {
        assert_eq!(pulled_budget_for(&limits), limits.max_buffer_size);
    }
    // The limit pair this machine's shared device reports (the host requests
    // `Limits::default()`): 256 MiB, which admits the repository's
    // 10 656-entity fixture (~172 MiB) and rejects the 44 591-entity fixture
    // (~2.3 GiB) that used to panic.
    let p620_shared_device = wgpu::Limits::default();
    assert_eq!(pulled_budget_for(&p620_shared_device), 268_435_456);
    assert!(pulled_budget_for(&p620_shared_device) > 180_550_060);
    assert!(pulled_budget_for(&p620_shared_device) < 2_281_858_648);
}

#[test]
fn only_the_active_state_permits_the_accelerated_path() {
    // The one-way invariant behind `FrameStats::accelerated`: a frame may
    // report `accelerated == false` while the state is `Active` (a paginated
    // page), but a skipped state must never be drawn through the pulled path.
    assert!(PulledState::Active.allows_accelerated());
    for skipped in [
        PulledState::SkippedInsufficientVram {
            required_bytes: 1_000,
            budget_bytes: 999,
        },
        PulledState::SkippedOutOfMemory {
            required_bytes: 1_000,
            budget_bytes: 2_000,
            message: "out of memory".to_string(),
        },
        PulledState::SkippedAllocationFailed {
            required_bytes: 1_000,
            budget_bytes: 2_000,
            message: "Buffer with 'cad-pulled-line-storage' label is invalid".to_string(),
        },
        PulledState::SkippedUnsegmentable {
            largest_class_bytes: 9,
            binding_limit_bytes: 8,
        },
        PulledState::SkippedNoConvertedGeometry,
        PulledState::Unavailable,
    ] {
        assert!(
            !skipped.allows_accelerated(),
            "{skipped:?} must not permit the accelerated path"
        );
        assert_ne!(skipped.as_str(), "active");
    }
    // The probe/status rendering is single-line and whitespace-free.
    let oom = PulledState::SkippedOutOfMemory {
        required_bytes: 1,
        budget_bytes: 2,
        message: "Out of Memory: driver said no".to_string(),
    };
    let rendered = oom.to_string();
    assert!(!rendered.contains(char::is_whitespace), "{rendered}");
    assert!(rendered.starts_with("skipped-out-of-memory("), "{rendered}");
}

#[test]
fn allocation_verdict_skips_on_any_captured_error() {
    use PulledAllocationError as E;
    use PulledAllocationVerdict as V;

    // Nothing reported: the attempt may proceed.
    assert_eq!(allocation_verdict(None, None, None), V::Proceed);

    // The measured failure mode: a driver-refused allocation is reported as a
    // *validation* error on the next use of the invalid handle, so it must
    // stop the attempt too — and must never be reported as provable OOM.
    let invalid = "Buffer with 'cad-pulled-line-storage' label is invalid".to_string();
    assert_eq!(
        allocation_verdict(Some(invalid.clone()), None, None),
        V::Skip(E::Other(invalid.clone()))
    );

    // A genuine create-time out-of-memory keeps its classification.
    assert_eq!(
        allocation_verdict(None, Some("Out of memory".to_string()), None),
        V::Skip(E::OutOfMemory("Out of memory".to_string()))
    );

    // Both captured: the OOM is the root cause, the validation error the
    // consequence of using the invalid handle.
    assert_eq!(
        allocation_verdict(
            Some(invalid.clone()),
            Some("Out of memory".to_string()),
            None
        ),
        V::Skip(E::OutOfMemory("Out of memory".to_string()))
    );

    // The test-only injection behaves like a captured OOM (the only way to
    // exercise the handler without provoking a real device failure).
    assert_eq!(
        allocation_verdict(None, None, Some("injected".to_string())),
        V::Skip(E::OutOfMemory("injected".to_string()))
    );

    // An error always wins over the injection: the device's own report is the
    // evidence, never the synthetic one.
    assert_eq!(
        allocation_verdict(Some(invalid.clone()), None, Some("injected".to_string())),
        V::Skip(E::Other(invalid))
    );
}

#[test]
fn captured_allocation_failure_maps_to_an_explicit_machine_readable_state() {
    // Provable OOM keeps the existing token: `apps/app-linux/src/host.rs`
    // prints `pulled={Display}` on its probe line, so the token must stay
    // single-line, non-empty and greppable.
    let oom = pulled_skip_state(
        2_000,
        1_000,
        PulledAllocationError::OutOfMemory("Out of Memory".to_string()),
    );
    assert_eq!(oom.as_str(), "skipped-out-of-memory");
    assert!(!oom.allows_accelerated());
    let oom_text = oom.to_string();
    assert!(!oom_text.is_empty());
    assert!(!oom_text.contains(char::is_whitespace), "{oom_text}");
    assert!(
        oom_text
            .starts_with("skipped-out-of-memory(required_bytes=2000,budget_bytes=1000,message="),
        "{oom_text}"
    );

    // An error that is not provably OOM gets its own explicit token instead of
    // a generic/empty one — never a silently successful build.
    let failed = pulled_skip_state(
        2_000,
        1_000,
        PulledAllocationError::Other(
            "Buffer with 'cad-pulled-line-storage' label is invalid".to_string(),
        ),
    );
    assert_eq!(failed.as_str(), "skipped-allocation-failed");
    assert!(!failed.allows_accelerated());
    let failed_text = failed.to_string();
    assert!(!failed_text.is_empty());
    assert!(!failed_text.contains(char::is_whitespace), "{failed_text}");
    assert!(
        failed_text.starts_with(
            "skipped-allocation-failed(required_bytes=2000,budget_bytes=1000,message="
        ),
        "{failed_text}"
    );
    assert!(failed_text.contains("label_is_invalid"), "{failed_text}");
    // The two verdicts are distinguishable on the probe line.
    assert_ne!(oom.as_str(), failed.as_str());
}
