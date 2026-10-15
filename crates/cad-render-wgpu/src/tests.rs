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

#[test]
fn image_quad_maps_the_unit_square_through_the_transform() {
    let batch = ImageBatch {
        resource: ResourceKey("texture".into()),
        transform: Transform3::translation(Point3 {
            x: 2.0,
            y: 3.0,
            z: 0.0,
        }),
        clip: None,
        alpha: 1.0,
        draw_order: 0,
        sources: Vec::new(),
    };
    let (vertices, indices) = image_quad_vertices(&batch);
    // Four vertices of 20 bytes (position vec3 + uv vec2), two triangles.
    assert_eq!(vertices.len(), 4 * 20);
    assert_eq!(indices, vec![0, 1, 2, 0, 2, 3]);
    let read = |i: usize| {
        let mut out = [0.0f32; 5];
        for (j, slot) in out.iter_mut().enumerate() {
            let start = i * 20 + j * 4;
            *slot = f32::from_le_bytes(vertices[start..start + 4].try_into().unwrap());
        }
        out
    };
    // The CAD origin is the lower-left with v pointing up, while decoded image
    // rows are top-down, so `v` is flipped: corner (0,0) gets UV (0,1) and
    // corner (1,1) gets UV (1,0).
    assert_eq!(read(0), [2.0, 3.0, 0.0, 0.0, 1.0]);
    assert_eq!(read(2), [3.0, 4.0, 0.0, 1.0, 0.0]);
}

#[test]
fn image_quad_uses_the_clip_polygon_instead_of_the_unit_square() {
    use cad_representation::ImageVertex;
    use std::sync::Arc;
    // A world-space left half of the unit square with matching UVs; the batch
    // transform is a large translation that must NOT be applied to the clip.
    let clip: Arc<[ImageVertex]> = Arc::from(
        vec![
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.0, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: -0.8,
                    z: 0.0,
                },
                uv: [0.5, 0.0],
            },
            ImageVertex {
                position: Point3 {
                    x: 0.0,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.5, 1.0],
            },
            ImageVertex {
                position: Point3 {
                    x: -0.8,
                    y: 0.8,
                    z: 0.0,
                },
                uv: [0.0, 1.0],
            },
        ]
        .into_boxed_slice(),
    );
    let batch = ImageBatch {
        resource: ResourceKey("texture".into()),
        transform: Transform3::translation(Point3 {
            x: 100.0,
            y: 100.0,
            z: 0.0,
        }),
        clip: Some(clip),
        alpha: 1.0,
        draw_order: 0,
        sources: Vec::new(),
    };
    let (vertices, indices) = image_quad_vertices(&batch);
    // Four clip vertices, fan-triangulated (0,1,2),(0,2,3).
    assert_eq!(vertices.len(), 4 * 20);
    assert_eq!(indices, vec![0, 1, 2, 0, 2, 3]);
    let read = |i: usize| {
        let mut out = [0.0f32; 5];
        for (j, slot) in out.iter_mut().enumerate() {
            let start = i * 20 + j * 4;
            *slot = f32::from_le_bytes(vertices[start..start + 4].try_into().unwrap());
        }
        out
    };
    // Clip positions are used verbatim (no transform), and `v` is flipped.
    assert_eq!(read(0), [-0.8, -0.8, 0.0, 0.0, 1.0]);
    assert_eq!(read(3), [-0.8, 0.8, 0.0, 0.0, 0.0]);
}

#[test]
fn cad_clear_color_is_pure_black_opaque() {
    // The React host requires the CAD render target to clear to actual pure
    // black (not a DOM overlay); this is the global default for every space.
    let color = CAD_CLEAR_COLOR;
    assert_eq!(
        (color.r, color.g, color.b, color.a),
        (0.0, 0.0, 0.0, 1.0),
        "the CAD render target clears to pure opaque black"
    );
}

#[test]
fn uniform_page_stays_within_the_default_binding_limit() {
    // WebGPU defaults: 64 KiB uniform binding limit, 256-byte offset alignment.
    let limits = wgpu::Limits::default();
    let (slot_size, slots) = uniform_layout_params(&limits);
    assert_eq!(
        slot_size, 256,
        "the struct is padded to the offset alignment"
    );
    assert_eq!(slots, 256, "64 KiB / 256 B = 256 slots per page");
    assert!(
        slots as u64 * u64::from(slot_size) <= limits.max_uniform_buffer_binding_size,
        "a whole page must remain a legal uniform binding"
    );
}

#[test]
fn uniform_slot_size_follows_the_offset_alignment() {
    // A backend with a larger offset alignment must still get aligned slots.
    let limits = wgpu::Limits {
        min_uniform_buffer_offset_alignment: 512,
        ..Default::default()
    };
    let (slot_size, _) = uniform_layout_params(&limits);
    assert_eq!(slot_size, 512);
    assert_eq!(slot_size % limits.min_uniform_buffer_offset_alignment, 0);
    // An adapter reporting a very large binding size is still capped.
    let huge = wgpu::Limits {
        max_uniform_buffer_binding_size: u64::MAX,
        ..Default::default()
    };
    let (_, slots) = uniform_layout_params(&huge);
    assert_eq!(slots, MAX_UNIFORM_SLOTS_PER_PAGE);
}

#[test]
fn slot_location_picks_page_and_offset_across_buffers() {
    // 3 slots per page at a 256-byte stride.
    assert_eq!(slot_location(0, 3, 256), (0, 0));
    assert_eq!(slot_location(2, 3, 256), (0, 512));
    // Slot 3 is the first slot of the *second* shared buffer.
    assert_eq!(slot_location(3, 3, 256), (1, 0));
    assert_eq!(slot_location(259, 256, 256), (1, 3 * 256));
}

#[test]
fn the_id_target_probe_prefers_r32uint_when_the_spec_guarantees_it_renderable() {
    // The WebGPU spec's guaranteed format features list `R32Uint` as a render
    // attachment, so a device that enabled no optional feature still gets the
    // single-channel id target. A device that did not would take the packed
    // fallback, which the headless tests exercise through the override hook.
    assert_eq!(
        Renderer::id_format_for_features(wgpu::Features::empty()),
        IdTargetFormat::R32Uint
    );
    assert_eq!(
        IdTargetFormat::R32Uint.texture_format(),
        wgpu::TextureFormat::R32Uint
    );
    assert_eq!(
        IdTargetFormat::PackedRgba8.texture_format(),
        wgpu::TextureFormat::Rgba8Unorm
    );
    assert_eq!(IdTargetFormat::R32Uint.as_str(), "r32uint");
    assert_eq!(IdTargetFormat::PackedRgba8.as_str(), "packed-rgba8");
    // A renderer with no device has no id target format.
    assert!(Renderer::default().id_target_format().is_none());
}

#[test]
fn the_uniform_struct_grew_for_the_pick_key_without_moving_the_slot_stride() {
    // `mat4x4` (64) + tint `vec4<f32>` (16) + key_flags `vec4<u32>` (16) = 96.
    assert_eq!(UNIFORM_STRUCT_BYTES, 96);
    // The dynamic-offset slot stride stays at the 256-byte alignment, so the
    // per-page slot count is unchanged by the extra field.
    let (slot_size, slots) = uniform_layout_params(&wgpu::Limits::default());
    assert_eq!(slot_size, 256);
    assert_eq!(slots, 256);
    let uniform = uniform_bytes(
        &[1.0; 16],
        [0.25, 0.5, 0.75, 1.0],
        [7, INSTANCE_FLAG_OPAQUE, 0, 0],
    );
    assert_eq!(uniform.len(), UNIFORM_STRUCT_BYTES as usize);
    // Word 16..20 = tint, 20..24 = key/flags (little-endian words).
    assert_eq!(
        f32::from_le_bytes(uniform[64..68].try_into().unwrap()),
        0.25
    );
    assert_eq!(u32::from_le_bytes(uniform[80..84].try_into().unwrap()), 7);
    assert_eq!(
        u32::from_le_bytes(uniform[84..88].try_into().unwrap()),
        INSTANCE_FLAG_OPAQUE
    );
}
