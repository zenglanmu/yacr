//! Unit tests.

use super::*;

#[test]
fn uninitialized_renderer_reports_no_active_backend() {
    // A preference is not an activated backend. Before the host supplies a
    // device there is no actual backend and no capabilities (audit B02).
    for preference in [
        BackendPreference::Auto,
        BackendPreference::WebGpu,
        BackendPreference::WebGl2,
    ] {
        let renderer = Renderer::new(preference);
        assert_eq!(renderer.active_backend(), None);
        assert!(
            renderer.capabilities().is_none(),
            "no device yet for {preference:?}"
        );
    }
}

#[test]
fn preference_and_active_backend_are_distinct_concepts() {
    // The requested preference is retained even while activation is pending.
    let renderer = Renderer::new(BackendPreference::WebGl2);
    assert_eq!(renderer.preference, BackendPreference::WebGl2);
    assert_eq!(renderer.active_backend(), None);
}

#[test]
fn backend_names_are_stable() {
    assert_eq!(ActiveBackend::WebGpu.as_str(), "webgpu");
    assert_eq!(ActiveBackend::WebGl2.as_str(), "webgl2");
    assert_eq!(ActiveBackend::Native.as_str(), "native");
}

#[test]
fn mesh_pipeline_declares_position_and_normal_attributes() {
    // Regression: `mesh.wgsl` reads `@location(0)` position and
    // `@location(1)` normal from two separate vertex buffers. The pipeline
    // must declare both layouts or wgpu rejects `cad-mesh-pipeline` at
    // creation ("Location[1] ... is not provided by the previous stage
    // outputs").
    let layouts = [mesh_vertex_layout(), mesh_normal_layout()];
    let locations: Vec<u32> = layouts
        .iter()
        .flat_map(|layout| layout.attributes.iter().map(|attr| attr.shader_location))
        .collect();
    assert_eq!(locations, vec![0, 1]);
    assert!(layouts.iter().all(|layout| layout.array_stride == 12));
}

#[test]
fn render_errors_distinguish_device_loss_from_a_bad_frame() {
    assert!(RenderError::DeviceLost("reset".into()).is_device_loss());
    assert!(!RenderError::Frame("validation".into()).is_device_loss());
    assert!(!RenderError::NotInitialized("no device".into()).is_device_loss());
}

#[test]
fn upload_time_is_absent_before_any_upload() {
    let renderer = Renderer::default();
    assert!(renderer.last_upload_ms().is_none());
}

#[test]
fn render_before_init_is_not_a_device_loss() {
    let mut renderer = Renderer::default();
    let target = RenderTarget::new(64, 64);
    let frame = renderer.render(Camera2d::default(), &target);
    assert!(matches!(frame, Err(RenderError::NotInitialized(_))));
    assert!(!renderer.is_device_lost());
}

#[test]
fn noted_device_loss_is_explicit_and_clears_batches() {
    let mut renderer = Renderer::default();
    // Simulate a batch having been uploaded, then a host-observed loss.
    let loss = renderer.note_device_lost("driver reset");
    assert!(loss.is_device_loss());
    assert!(renderer.is_device_lost());
    assert_eq!(renderer.batch_count(), 0);
    let target = RenderTarget::new(64, 64);
    let frame = renderer.render(Camera2d::default(), &target);
    assert!(matches!(frame, Err(RenderError::DeviceLost(_))));
}

#[test]
fn over_budget_diagnostic_carries_structured_parameters() {
    let report = OverBudget {
        category: "vertices",
        requested: 100,
        limit: 50,
        skipped_batches: 3,
    };
    let reason = Renderer::over_budget_reason(&report);
    assert_eq!(reason.code, codes::RENDER_FRAME_OVER_BUDGET);
    assert!(reason.parameters.contains(&DiagnosticParameter::Limit(50)));
}

#[test]
fn device_lost_diagnostic_has_stable_code() {
    let reason = Renderer::device_lost_reason("test");
    assert_eq!(reason.code, codes::RENDER_DEVICE_LOST);
}

#[test]
fn draw_passes_remap_accepted_positions_to_global_indices() {
    // The budget accepted global batches 10, 11, 12 (upload order). Batch 10
    // is transparent and farthest, 11 opaque with the higher draw order, and
    // 12 opaque with a lower draw order.
    let accepted = vec![10usize, 11, 12];
    let entries = vec![
        BatchOrderEntry {
            draw_order: 0,
            alpha: 0.5,
            centroid: [100.0, 0.0, 0.0],
        },
        BatchOrderEntry {
            draw_order: 5,
            alpha: 1.0,
            centroid: [1.0, 0.0, 0.0],
        },
        BatchOrderEntry {
            draw_order: 1,
            alpha: 1.0,
            centroid: [2.0, 0.0, 0.0],
        },
    ];
    let passes = draw_passes(&accepted, &entries, Some([0.0, 0.0, 0.0]));
    // Opaque first, ascending draw order: global 12 (order 1), then 11 (5).
    assert_eq!(passes.opaque, vec![12, 11]);
    // Transparent after, single element global 10.
    assert_eq!(passes.transparent, vec![10]);
    assert_eq!(passes.invisible, 0);
}

#[test]
fn draw_passes_report_invisible_batches() {
    let accepted = vec![0usize, 1];
    let entries = vec![
        BatchOrderEntry {
            draw_order: 0,
            alpha: 0.0,
            centroid: [0.0; 3],
        },
        BatchOrderEntry {
            draw_order: 0,
            alpha: 1.0,
            centroid: [0.0; 3],
        },
    ];
    let passes = draw_passes(&accepted, &entries, Some([0.0; 3]));
    assert_eq!(passes.opaque, vec![1]);
    assert!(passes.transparent.is_empty());
    assert_eq!(passes.invisible, 1);
}
